import type {
  AgentRun,
  AgentRunEvent,
  AgentRunEventRecord,
  AgentRunId,
  AgentRunState,
  EventEnvelope,
  Workspace,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  describeEvent,
  describeRun,
  initialRunsState,
  isTerminalRun,
  MAX_RUN_EVENTS,
  needsAttention,
  runsReducer,
  selectLog,
  selectRuns,
} from './runs-reducer';

const RUN = 'run-1' as AgentRunId;
const OTHER = 'run-2' as AgentRunId;

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: RUN,
    workstreamId: 'wst-1' as AgentRun['workstreamId'],
    workspaceId: 'w-1' as AgentRun['workspaceId'],
    provider: 'FAKE',
    state: 'RUNNING',
    startedAt: 1_800_000_000_000,
    updatedAt: 1_800_000_000_000,
    cancellationRequested: false,
    correlationId: 'corr-1' as AgentRun['correlationId'],
    ...overrides,
  };
}

function record(seq: number, runId: AgentRunId = RUN): AgentRunEventRecord {
  return {
    seq,
    runId,
    at: 1_800_000_000_000 + seq,
    event: { type: 'FILE_WRITTEN', path: { display: `file-${seq}.ts` } },
  };
}

function changed(value: AgentRun): EventEnvelope {
  return {
    seq: 1,
    actor: { type: 'USER' },
    name: 'AgentRunChanged',
    payload: { run: value },
  };
}

function appended(
  runId: AgentRunId,
  events: AgentRunEventRecord[],
  dropped = 0,
  seq = 1,
): EventEnvelope {
  return {
    seq,
    actor: { type: 'USER' },
    name: 'AgentRunEventAppended',
    payload: { runId, events, dropped },
  };
}

/** A state with one known run, which is what most of these start from. */
function withRun(value: AgentRun = run()) {
  return runsReducer(initialRunsState, { type: 'loaded', runs: [value] });
}

describe('runsReducer', () => {
  it('separates "no runs" from "not asked yet"', () => {
    expect(initialRunsState.loaded).toBe(false);
    const state = runsReducer(initialRunsState, { type: 'loaded', runs: [] });
    expect(state.loaded).toBe(true);
    expect(selectRuns(state)).toEqual([]);
  });

  it('rehydrates from a list and orders newest first', () => {
    const state = runsReducer(initialRunsState, {
      type: 'loaded',
      runs: [run(), run({ id: OTHER, startedAt: 1_800_000_001_000 })],
    });
    expect(selectRuns(state).map((r) => r.id)).toEqual([OTHER, RUN]);
  });

  it('replaces the whole run on AgentRunChanged', () => {
    let state = withRun();
    state = runsReducer(state, {
      type: 'event',
      event: changed(
        run({
          state: 'BLOCKED',
          updatedAt: 1_800_000_000_500,
          activity: { kind: 'REQUESTING_PERMISSION', startedAt: 1_800_000_000_500 },
        }),
      ),
    });
    expect(state.byId[RUN]?.state).toBe('BLOCKED');
    expect(state.byId[RUN]?.activity?.kind).toBe('REQUESTING_PERMISSION');
  });

  it('never lets a stale snapshot overwrite a newer one', () => {
    let state = withRun();
    const newer = run({ state: 'BLOCKED', updatedAt: 1_800_000_000_900 });
    state = runsReducer(state, { type: 'event', event: changed(newer) });
    // The `runs.cancel` answer racing the event it caused, arriving stale.
    state = runsReducer(state, { type: 'run', run: run({ updatedAt: 1_800_000_000_100 }) });
    expect(state.byId[RUN]?.state).toBe('BLOCKED');
  });

  it('never resurrects a run that already ended, even at the same timestamp', () => {
    let state = withRun();
    const finished = run({
      state: 'FINISHED',
      updatedAt: 1_800_000_000_400,
      finishedAt: 1_800_000_000_400,
    });
    state = runsReducer(state, { type: 'event', event: changed(finished) });
    const before = state;
    state = runsReducer(state, {
      type: 'event',
      event: changed(run({ state: 'RUNNING', updatedAt: 1_800_000_000_400 })),
    });
    expect(state).toBe(before);
    expect(state.byId[RUN]?.state).toBe('FINISHED');
  });

  it('appends events for a known run', () => {
    let state = withRun();
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(1), record(2)]) });
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(3)]) });
    expect(selectLog(state, RUN).events.map((e) => e.seq)).toEqual([1, 2, 3]);
  });

  it('drops events for a run it has never seen', () => {
    const state = runsReducer(initialRunsState, {
      type: 'event',
      event: appended(OTHER, [record(1, OTHER)]),
    });
    expect(state).toBe(initialRunsState);
    expect(selectLog(state, OTHER).events).toEqual([]);
  });

  it('drops a rehydrated log for a run it has never seen', () => {
    const state = runsReducer(initialRunsState, {
      type: 'log',
      log: { runId: OTHER, events: [record(1, OTHER)], dropped: 0 },
    });
    expect(state).toBe(initialRunsState);
  });

  it('ignores a duplicate batch rather than double-appending it', () => {
    let state = withRun();
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(1), record(2)]) });
    const before = state;
    // The same events again, as `runs.events` rehydration racing the live batch.
    state = runsReducer(state, {
      type: 'log',
      log: { runId: RUN, events: [record(1), record(2)], dropped: 0 },
    });
    expect(state).toBe(before);
    expect(selectLog(state, RUN).events.map((e) => e.seq)).toEqual([1, 2]);
  });

  it('carries the core’s dropped count through, and never lowers it', () => {
    let state = withRun();
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(10)], 9) });
    expect(selectLog(state, RUN).dropped).toBe(9);
    // A late batch reporting an older count must not erase the gap.
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(11)], 0) });
    expect(selectLog(state, RUN).dropped).toBe(9);
  });

  it('records a dropped count even when the batch adds nothing new', () => {
    let state = withRun();
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(1)], 0) });
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(1)], 4) });
    expect(selectLog(state, RUN).dropped).toBe(4);
    expect(selectLog(state, RUN).events).toHaveLength(1);
  });

  it('bounds the ring at the core’s own limit, dropping the oldest', () => {
    let state = withRun();
    const batch = Array.from({ length: MAX_RUN_EVENTS + 10 }, (_, i) => record(i + 1));
    state = runsReducer(state, { type: 'event', event: appended(RUN, batch) });
    const { events } = selectLog(state, RUN);
    expect(events).toHaveLength(MAX_RUN_EVENTS);
    expect(events[0]?.seq).toBe(11);
    expect(events.at(-1)?.seq).toBe(MAX_RUN_EVENTS + 10);
  });

  it('forgets everything when the workspace changes', () => {
    let state = withRun();
    state = runsReducer(state, { type: 'event', event: appended(RUN, [record(1)]) });
    state = runsReducer(state, { type: 'workspace', workspace: null });
    expect(state).toBe(initialRunsState);
    expect(selectRuns(state)).toEqual([]);

    const workspace = { id: 'w-2' } as Workspace;
    const reopened = runsReducer(withRun(), { type: 'workspace', workspace });
    expect(selectRuns(reopened)).toEqual([]);
    expect(reopened.loaded).toBe(false);
  });

  it('ignores events from other domains', () => {
    const state = withRun();
    const next = runsReducer(state, {
      type: 'event',
      event: {
        seq: 2,
        actor: { type: 'USER' },
        name: 'AgentSessionsChanged',
        payload: { sessions: [], needsYou: 0, processEvidence: true },
      },
    });
    expect(next).toBe(state);
  });
});

describe('run state predicates', () => {
  it('names the four terminal states and no others', () => {
    const terminal: AgentRunState[] = ['FINISHED', 'FAILED', 'CANCELLED', 'INTERRUPTED'];
    const rest: AgentRunState[] = [
      'STARTING',
      'RUNNING',
      'WAITING',
      'NEEDS_DECISION',
      'BLOCKED',
      'REVIEW',
    ];
    for (const state of terminal) expect(isTerminalRun(state)).toBe(true);
    for (const state of rest) expect(isTerminalRun(state)).toBe(false);
  });

  it('names the states that are waiting on the user', () => {
    expect(needsAttention('BLOCKED')).toBe(true);
    expect(needsAttention('NEEDS_DECISION')).toBe(true);
    expect(needsAttention('WAITING')).toBe(true);
    expect(needsAttention('REVIEW')).toBe(true);
    expect(needsAttention('RUNNING')).toBe(false);
    expect(needsAttention('FINISHED')).toBe(false);
  });
});

describe('describeRun', () => {
  it('reads the state alone when nothing is happening', () => {
    expect(describeRun(run({ state: 'STARTING' }))).toBe('STARTING');
  });

  it('reads the state and the activity together', () => {
    const value = run({
      activity: {
        kind: 'EDITING',
        path: { path: 'src/auth/session.ts', display: 'src/auth/session.ts' },
        startedAt: 1,
      },
    });
    expect(describeRun(value)).toBe('RUNNING · Editing src/auth/session.ts');
  });

  it('prefers the command to the path, and title-cases a two-word kind', () => {
    const value = run({
      activity: {
        kind: 'REQUESTING_PERMISSION',
        path: { display: '~/elsewhere/notes.md' },
        command: 'cargo test',
        startedAt: 1,
      },
    });
    expect(describeRun(value)).toBe('RUNNING · Requesting permission cargo test');
  });
});

describe('describeEvent', () => {
  const cases: Array<[AgentRunEvent, string]> = [
    [{ type: 'RUN_STARTED' }, 'started'],
    [{ type: 'RUN_FINISHED' }, 'finished'],
    [{ type: 'RUN_FINISHED', summary: 'all green' }, 'finished · all green'],
    [{ type: 'PROMPT_STARTED', text: 'add a test' }, 'prompt · add a test'],
    [{ type: 'PROMPT_FINISHED' }, 'reply'],
    [{ type: 'PROMPT_FINISHED', text: 'done' }, 'reply · done'],
    [{ type: 'FILE_READ', path: { display: 'a.ts' } }, 'read · a.ts'],
    [{ type: 'FILE_WRITE_REQUESTED', path: { display: 'a.ts' } }, 'write requested · a.ts'],
    [{ type: 'FILE_WRITTEN', path: { display: 'a.ts' } }, 'wrote · a.ts'],
    [{ type: 'COMMAND_STARTED', command: 'cargo test' }, 'ran · cargo test'],
    [
      { type: 'COMMAND_FINISHED', command: 'cargo test', ok: true, exitCode: 0 },
      'ok · cargo test (exit 0)',
    ],
    // Killed by a signal: no exit code, and it did not succeed.
    [{ type: 'COMMAND_FINISHED', command: 'cargo test', ok: false }, 'failed · cargo test'],
    [{ type: 'PERMISSION_REQUESTED', tool: 'Bash' }, 'permission · Bash'],
    [
      { type: 'PERMISSION_REQUESTED', tool: 'Bash', detail: 'rm -rf' },
      'permission · Bash · rm -rf',
    ],
    [{ type: 'DECISION_REQUESTED', question: 'which one?' }, 'decision · which one?'],
    [{ type: 'RUN_BLOCKED', reason: 'needs you' }, 'blocked · needs you'],
    [{ type: 'RUN_RESUMED' }, 'resumed'],
    [
      { type: 'RUN_FAILED', error: { code: 'Internal', message: 'boom', retryable: false } },
      'failed · Internal: boom',
    ],
  ];

  it.each(cases)('renders %o as text', (event, expected) => {
    expect(describeEvent(event)).toBe(expected);
  });

  it('names a variant it does not know rather than throwing', () => {
    const future = { type: 'RUN_INTERRUPTED' } as unknown as AgentRunEvent;
    expect(describeEvent(future)).toBe('RUN_INTERRUPTED');
  });
});
