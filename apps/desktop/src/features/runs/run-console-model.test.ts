import type { AgentRun, AgentRunEvent, AgentRunId, AgentRunState } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { promptGate, runFeedRows } from './run-console-model';
import type { RunLog } from './runs-reducer';

/**
 * What an owned run's console draws, and what its prompt box will accept
 * (T310-U).
 *
 * Pure, so every state the box refuses — and the reason it gives — is asserted
 * without React, a core or a DOM. The prompt gate in particular is a *mirror*
 * of `AgentRunState::accepts_input`, so the test that matters is the total one:
 * every state named, none left to a default nobody chose.
 */

function record(seq: number, event: AgentRunEvent, at = seq * 1000) {
  return { seq, runId: 'run-1' as AgentRunId, at, event };
}

function log(events: readonly ReturnType<typeof record>[], dropped = 0): RunLog {
  return { events, dropped };
}

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: 'run-1' as AgentRunId,
    workstreamId: 'w1' as AgentRun['workstreamId'],
    workspaceId: 'ws1' as AgentRun['workspaceId'],
    provider: 'CLAUDE_CODE',
    state: 'RUNNING',
    startedAt: 0,
    updatedAt: 0,
    cancellationRequested: false,
    correlationId: 'c1' as AgentRun['correlationId'],
    ...overrides,
  };
}

describe('the feed an owned run draws', () => {
  it('draws the conversation the contract already carries (§6)', () => {
    const rows = runFeedRows(
      log([
        record(1, { type: 'PROMPT_STARTED', text: 'run the tests' }),
        record(2, { type: 'PROMPT_FINISHED', text: 'they pass' }),
      ]),
    );
    expect(rows.map((row) => [row.label, row.text])).toEqual([
      ['You', 'run the tests'],
      ['Agent', 'they pass'],
    ]);
    // The user's own words and the agent's reply are the two halves §6 names,
    // and neither is drawn from anything but the ring.
    expect(rows.map((row) => row.tone)).toEqual(['prompt', 'assistant']);
  });

  it('says a turn ended with nothing rather than drawing a blank row', () => {
    const [row] = runFeedRows(log([record(1, { type: 'PROMPT_FINISHED' })]));
    expect(row?.text).toBe('Answered with nothing to show.');
  });

  it('keeps a command and its result as two rows, each honest about itself', () => {
    // The timeline pairs a tool call with its result because both carry a
    // `toolUseId`. A run's command events carry only the command text, so
    // pairing on that would attach the wrong result the first time an agent
    // runs `cargo test` twice.
    const rows = runFeedRows(
      log([
        record(1, { type: 'COMMAND_STARTED', command: 'cargo test' }),
        record(2, { type: 'COMMAND_FINISHED', command: 'cargo test', ok: false, exitCode: 101 }),
      ]),
    );
    expect(rows.map((row) => row.outcome)).toEqual(['running', 'fail']);
    expect(rows[1]?.detail).toBe('exit 101');
  });

  it('says nothing about an exit code a signal never produced', () => {
    // Not derivable from `ok`: a command killed by a signal has no exit code,
    // and "exit 0" for one would be an invention.
    const [row] = runFeedRows(
      log([record(1, { type: 'COMMAND_FINISHED', command: 'sleep 9', ok: false })]),
    );
    expect(row?.detail).toBeNull();
    expect(row?.outcome).toBe('fail');
  });

  it('draws the loss the core reported instead of repairing it', () => {
    const rows = runFeedRows(log([record(9, { type: 'RUN_RESUMED' })], 4));
    // First, because the events that are missing are the oldest ones.
    expect(rows[0]?.tone).toBe('gap');
    expect(rows[0]?.text).toContain('4 earlier events dropped');
    expect(rows).toHaveLength(2);
  });

  it('counts one dropped event in the singular', () => {
    expect(runFeedRows(log([], 1))[0]?.text).toContain('1 earlier event dropped');
  });

  it('shows an event from a newer core rather than dropping it', () => {
    // The renderer's half of the reason `AgentRunEvent` is tagged: a build
    // against an older protocol shows what it cannot name.
    const [row] = runFeedRows(
      log([record(1, { type: 'QUANTUM_ENTANGLED', spin: 'up' } as unknown as AgentRunEvent)]),
    );
    expect(row?.tone).toBe('unknown');
    expect(row?.label).toBe('Unrecognised');
    expect(row?.text).toContain('QUANTUM_ENTANGLED');
  });

  it('names every variant this build knows, so none renders as a blank row', () => {
    const events: readonly AgentRunEvent[] = [
      { type: 'RUN_STARTED' },
      { type: 'RUN_FINISHED', summary: 'done' },
      { type: 'PROMPT_STARTED', text: 'hi' },
      { type: 'PROMPT_FINISHED', text: 'hello' },
      { type: 'FILE_READ', path: { display: 'src/a.rs' } },
      { type: 'FILE_WRITE_REQUESTED', path: { display: 'src/a.rs' } },
      { type: 'FILE_WRITTEN', path: { display: 'src/a.rs' } },
      { type: 'COMMAND_STARTED', command: 'ls' },
      { type: 'COMMAND_FINISHED', command: 'ls', ok: true, exitCode: 0 },
      { type: 'PERMISSION_REQUESTED', tool: 'Bash', detail: 'touch /tmp/x' },
      { type: 'DECISION_REQUESTED', question: 'which branch?' },
      { type: 'RUN_BLOCKED', reason: 'needs approval' },
      { type: 'RUN_RESUMED' },
      {
        type: 'RUN_FAILED',
        error: { code: 'Internal', message: 'the pipe closed', retryable: false },
      },
    ];
    const rows = runFeedRows(log(events.map((event, index) => record(index + 1, event))));
    expect(rows).toHaveLength(events.length);
    for (const row of rows) {
      expect(row.label).not.toBe('');
      expect(row.text).not.toBe('');
      expect(row.tone).not.toBe('unknown');
    }
  });

  it('keys rows on the core sequence, so React does not redraw the feed', () => {
    const rows = runFeedRows(log([record(7, { type: 'RUN_STARTED' })], 2));
    expect(rows.map((row) => row.key)).toEqual(['gap:dropped', 'event:7']);
  });
});

describe('the prompt gate mirrors the core, state for state (§9)', () => {
  const STATES: readonly AgentRunState[] = [
    'STARTING',
    'RUNNING',
    'WAITING',
    'NEEDS_DECISION',
    'BLOCKED',
    'REVIEW',
    'FINISHED',
    'FAILED',
    'CANCELLED',
    'INTERRUPTED',
  ];

  it('opens on exactly RUNNING and WAITING, which is `accepts_input`', () => {
    // Not one representative case: the manager refuses against that table and
    // this box disables against it, so neither may keep its own copy of the
    // answer. Every state is named here for the same reason.
    const open = STATES.filter((state) => promptGate(run({ state })).kind === 'ready');
    expect(open).toEqual(['RUNNING', 'WAITING']);
  });

  it('gives a reason for every state it refuses', () => {
    for (const state of STATES) {
      const gate = promptGate(run({ state }));
      if (gate.kind === 'ready') continue;
      expect(gate.kind).toBe('refused');
      if (gate.kind === 'refused') expect(gate.reason.length).toBeGreaterThan(0);
    }
  });

  it('shuts while a cancel is outstanding, even from a state that accepts input', () => {
    // The more recent intent wins: the run is being torn down, and queueing a
    // turn for it would be inviting a reply that has nowhere to go.
    const gate = promptGate(run({ state: 'RUNNING', cancellationRequested: true }));
    expect(gate).toEqual({ kind: 'refused', reason: expect.stringContaining('stopping') });
  });

  it('ignores an outstanding cancel once the run has actually ended', () => {
    // "It ended" is the more useful thing to say than "it is stopping".
    const gate = promptGate(run({ state: 'CANCELLED', cancellationRequested: true }));
    expect(gate.kind === 'refused' && gate.reason).toContain('ended');
  });

  it('draws nothing at all for a scripted run', () => {
    // §9's rule, applied to the other lane it names: no box, and no disabled
    // one either — a greyed-out control implies a channel that could be opened.
    expect(promptGate(run({ provider: 'FAKE' }))).toEqual({ kind: 'absent' });
    // Even in a state that would otherwise accept input.
    expect(promptGate(run({ provider: 'FAKE', state: 'WAITING' })).kind).toBe('absent');
  });

  it('refuses a state reported by a newer core rather than assuming it is open', () => {
    // The safe direction: letting someone type into a run this build cannot
    // vouch for is the worse failure.
    const gate = promptGate(run({ state: 'ASCENDED' as AgentRunState }));
    expect(gate.kind).toBe('refused');
  });
});
