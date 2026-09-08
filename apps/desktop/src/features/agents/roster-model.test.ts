import type {
  AgentActivity,
  AgentPermissionRequest,
  AgentRoster,
  AgentRun,
  AgentRunId,
  AgentSession,
  Workstream,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { activityLine, partitionRows, rosterRows } from './roster-model';

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: 'run-aaaaaaa-1' as AgentRunId,
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

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: 'sess-1',
    provider: 'CLAUDE_CODE',
    activity: 'WORKING',
    activitySource: 'PROCESS',
    evidence: 'BOTH',
    inWorkspace: true,
    ...overrides,
  };
}

function pending(): AgentPermissionRequest {
  return {
    askId: 'ask-1',
    sessionId: 'sess-1',
    verb: 'WRITE_FILE',
    target: 'crates/terminal/src/pty.rs',
    added: 64,
    removed: 11,
    outsideRoot: true,
    sensitive: false,
    askedAt: 1_800_000_000_000,
  };
}

function roster(sessions: readonly AgentSession[], needsYou = 0): AgentRoster {
  return { sessions: [...sessions], needsYou, processEvidence: true };
}

function workstream(overrides: Partial<Workstream> = {}): Workstream {
  return {
    id: 'wst-1' as Workstream['id'],
    workspaceId: 'w-1' as Workstream['workspaceId'],
    title: 'terminal pty reflow',
    state: 'IN_PROGRESS',
    allowedTransitions: [],
    openDecisions: 0,
    createdAt: 1_800_000_000_000,
    updatedAt: 1_800_000_000_000,
    ...overrides,
  };
}

describe('rosterRows', () => {
  it('carries both sources and marks which Gritty owns', () => {
    const rows = rosterRows([run()], roster([session()]), [workstream()]);
    expect(rows).toHaveLength(2);

    const owned = rows.find((row) => row.owned);
    const observed = rows.find((row) => !row.owned);

    // The discriminator, and the ids that get a caller back to the type the
    // row actually came from. A row never carries both.
    expect(owned?.runId).toBe('run-aaaaaaa-1');
    expect(owned?.sessionId).toBeNull();
    expect(observed?.sessionId).toBe('sess-1');
    expect(observed?.runId).toBeNull();
  });

  it('keys are unique even when a run and a session share a raw id', () => {
    const rows = rosterRows(
      [run({ id: 'clash' as AgentRunId })],
      roster([session({ id: 'clash' })]),
      [workstream()],
    );
    expect(new Set(rows.map((row) => row.key)).size).toBe(2);
  });

  it('flags a session from another workspace rather than dropping it', () => {
    // The case the execution board named as the only one that justifies this
    // section beside the island: the sidebar is *here*, the island is the
    // machine. But flagged, not filtered — the rail was the only list that
    // showed a machine-wide session, and filtering here would have made
    // deleting the rail quietly lose them (T233-C).
    const rows = rosterRows(
      [],
      roster([session({ id: 'here' }), session({ id: 'away', inWorkspace: false })]),
      [],
    );
    expect(rows.map((row) => row.sessionId)).toEqual(['here', 'away']);
    expect(rows.find((row) => row.sessionId === 'away')?.elsewhere).toBe(true);
    expect(rows.find((row) => row.sessionId === 'here')?.elsewhere).toBe(false);
  });

  describe('what the section may hide', () => {
    it('retires a session whose process is gone and a run that is simply over', () => {
      const rows = rosterRows(
        [run({ id: 'done' as AgentRunId, state: 'FINISHED' })],
        roster([session({ activity: 'ENDED' })]),
        [workstream()],
      );
      expect(rows.every((row) => row.retired)).toBe(true);
    });

    it('never retires a failed or interrupted run', () => {
      // "The agent broke" and "we stopped being able to watch it" are outcomes
      // the user has to see. Folding them in with the ordinary dead would hide
      // the two states the run machine exists to distinguish.
      const rows = rosterRows(
        [
          run({ id: 'broke' as AgentRunId, state: 'FAILED' }),
          run({ id: 'lost' as AgentRunId, state: 'INTERRUPTED' }),
        ],
        roster([]),
        [workstream()],
      );
      expect(rows.every((row) => !row.retired)).toBe(true);
    });

    it('keeps a row that needs you out of the hidden half, wherever it runs', () => {
      const rows = rosterRows(
        [],
        roster([session({ id: 'away', inWorkspace: false, activity: 'BLOCKED' })]),
        [],
      );
      expect(partitionRows(rows).hidden).toEqual([]);
      expect(partitionRows(rows).shown).toHaveLength(1);
    });

    it('splits the quiet from the live', () => {
      const rows = rosterRows(
        [],
        roster([
          session({ id: 'live' }),
          session({ id: 'dead', activity: 'ENDED' }),
          session({ id: 'away', inWorkspace: false }),
        ]),
        [],
      );
      const { shown, hidden } = partitionRows(rows);
      expect(shown.map((row) => row.sessionId)).toEqual(['live']);
      expect(hidden.map((row) => row.sessionId).sort()).toEqual(['away', 'dead']);
    });
  });

  it('attributes an observed status and leaves an owned one unattributed', () => {
    // T221 §24 for the session; a run's state is a transition the core
    // accepted, not a reading of evidence, so there is no source to name.
    const rows = rosterRows([run()], roster([session({ activitySource: 'HOOK' })]), [workstream()]);
    expect(rows.find((row) => !row.owned)?.sourceLabel).toBe('reported by the agent itself');
    expect(rows.find((row) => row.owned)?.sourceLabel).toBeNull();
  });

  it('never recomputes the roster total', () => {
    // `AgentRoster.needsYou` is the core's, and the island publishes it. If
    // this module derived its own the two surfaces could disagree, which is
    // exactly what `agents-panel.tsx` refuses to allow.
    const input = roster([session({ activity: 'BLOCKED' })], 7);
    rosterRows([], input, []);
    expect(input.needsYou).toBe(7);
  });

  describe('ordering', () => {
    it('puts what wants you first, then what is alive, then what is over', () => {
      const rows = rosterRows(
        [
          run({ id: 'r-finished' as AgentRunId, state: 'FINISHED' }),
          run({ id: 'r-running' as AgentRunId, state: 'RUNNING' }),
          run({ id: 'r-asks' as AgentRunId, state: 'NEEDS_DECISION' }),
        ],
        roster([]),
        [workstream()],
      );
      expect(rows.map((row) => row.runId)).toEqual(['r-asks', 'r-running', 'r-finished']);
    });

    it('sorts a failed run to the bottom band with the finished ones', () => {
      const rows = rosterRows(
        [
          run({ id: 'r-failed' as AgentRunId, state: 'FAILED' }),
          run({ id: 'r-running' as AgentRunId, state: 'RUNNING' }),
        ],
        roster([]),
        [workstream()],
      );
      expect(rows.map((row) => row.runId)).toEqual(['r-running', 'r-failed']);
    });

    it('preserves each source order inside a band', () => {
      // Runs arrive newest-first from `selectRuns` and sessions in the core's
      // own ranking. Re-ranking either here would be the renderer
      // second-guessing a decision the core is documented as owning.
      const rows = rosterRows(
        [run({ id: 'r-1' as AgentRunId }), run({ id: 'r-2' as AgentRunId })],
        roster([session({ id: 's-1' }), session({ id: 's-2' })]),
        [workstream()],
      );
      expect(rows.map((row) => row.runId ?? row.sessionId)).toEqual(['r-1', 'r-2', 's-1', 's-2']);
    });

    it('promotes a waiting session above a working run', () => {
      const rows = rosterRows(
        [run({ state: 'RUNNING' })],
        roster([session({ id: 'waiting', activity: 'WAITING' })]),
        [workstream()],
      );
      expect(rows[0]?.sessionId).toBe('waiting');
    });

    it.each<[AgentActivity, boolean]>([
      ['BLOCKED', true],
      ['WAITING', true],
      ['WORKING', false],
      ['IDLE', false],
      ['ENDED', false],
    ])('treats a %s session as needing you: %s', (activity, expected) => {
      // The core's own definition of the count it publishes, applied to one
      // session rather than restated as a second policy.
      const rows = rosterRows([], roster([session({ activity })]), []);
      expect(rows[0]?.needsYou).toBe(expected);
    });
  });

  describe('what a row says', () => {
    it("takes an owned row's title from its workstream", () => {
      // A run brings no task model of its own; the workstream is the work.
      const rows = rosterRows([run()], roster([]), [workstream({ title: 'reflow scrollback' })]);
      expect(rows[0]?.title).toBe('reflow scrollback');
      expect(rows[0]?.id).toBe('run-aaa');
    });

    it('leaves an owned row untitled rather than inventing one', () => {
      const rows = rosterRows([run()], roster([]), []);
      expect(rows[0]?.title).toBeNull();
    });

    it("uses a session's folder as its handle, and its task as the title", () => {
      const rows = rosterRows(
        [],
        roster([session({ rootName: 'gritty', title: 'wire the watcher' })]),
        [],
      );
      expect(rows[0]?.id).toBe('gritty');
      expect(rows[0]?.title).toBe('wire the watcher');
    });

    it('shows a run state and a session activity through their own mappings', () => {
      const rows = rosterRows(
        [run({ state: 'REVIEW' })],
        roster([session({ activity: 'BLOCKED' })]),
        [workstream()],
      );
      expect(rows.find((row) => row.owned)?.status.label).toBe('Ready for review');
      expect(rows.find((row) => !row.owned)?.status.label).toBe('Pending validation');
    });
  });

  describe('badges', () => {
    it('marks an ask on both sources with the same word', () => {
      const rows = rosterRows(
        [run({ state: 'BLOCKED' })],
        roster([session({ pending: pending() })]),
        [workstream()],
      );
      expect(rows.every((row) => row.badge?.text === 'asks')).toBe(true);
      expect(rows.every((row) => row.badge?.tone === 'attention')).toBe(true);
    });

    it('lets a requested cancel outrank an ask', () => {
      // The more recent intent wins: a row still saying `asks` while it is
      // being torn down invites an answer with nowhere left to go.
      const rows = rosterRows(
        [run({ state: 'BLOCKED', cancellationRequested: true })],
        roster([]),
        [workstream()],
      );
      expect(rows[0]?.badge).toEqual({ text: 'cancelling', tone: 'quiet' });
    });

    it('drops the cancelling badge once the run actually stopped', () => {
      const rows = rosterRows(
        [run({ state: 'CANCELLED', cancellationRequested: true })],
        roster([]),
        [workstream()],
      );
      expect(rows[0]?.badge).toBeNull();
    });

    it('leaves a plain running row unbadged', () => {
      const rows = rosterRows([run({ state: 'RUNNING' })], roster([]), [workstream()]);
      expect(rows[0]?.badge).toBeNull();
    });
  });
});

describe('activityLine', () => {
  it('prefers the command over the file it happens to touch', () => {
    expect(
      activityLine(
        run({
          activity: {
            kind: 'EXECUTING',
            command: 'cargo test',
            path: { display: 'pty.rs' },
            startedAt: 1,
          },
        }),
      ),
    ).toBe('Executing cargo test');
  });

  it('falls back to the display path, which is never resolved', () => {
    expect(
      activityLine(
        run({ activity: { kind: 'EDITING', path: { display: 'src/pty.rs' }, startedAt: 1 } }),
      ),
    ).toBe('Editing src/pty.rs');
  });

  it('says the kind alone when the activity is about nothing in particular', () => {
    expect(activityLine(run({ activity: { kind: 'THINKING', startedAt: 1 } }))).toBe('Thinking');
    expect(activityLine(run({ activity: { kind: 'REQUESTING_PERMISSION', startedAt: 1 } }))).toBe(
      'Requesting permission',
    );
  });

  it('is null when there is no activity, which is most of STARTING', () => {
    expect(activityLine(run())).toBeNull();
  });
});
