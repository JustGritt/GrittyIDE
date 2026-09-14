import type {
  AgentRun,
  AgentRunId,
  AgentSession,
  Workstream,
  WorkstreamId,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { askingSessions, isPermissionQueueChord, waitingAsks } from './permission-banner';

function session(id: string, askedAt: number | null): AgentSession {
  return {
    id,
    provider: 'CLAUDE_CODE',
    activity: askedAt === null ? 'WORKING' : 'BLOCKED',
    activitySource: 'HOOK',
    evidence: 'BOTH',
    inWorkspace: true,
    pending:
      askedAt === null
        ? undefined
        : {
            askId: `ask-${id}`,
            sessionId: id,
            verb: 'RUN_COMMAND',
            target: 'cargo publish --dry-run',
            diff: [],
            added: 0,
            removed: 0,
            outsideRoot: false,
            sensitive: false,
            askedAt,
          },
  } as AgentSession;
}

describe('which agents are asking', () => {
  it('lists only the ones stopped at a prompt, oldest ask first', () => {
    const asking = askingSessions([
      session('c', 3_000),
      session('busy', null),
      session('a', 1_000),
      session('b', 2_000),
    ]);
    // Oldest first, because that is the one the banner names and the one the
    // queue puts at the top: a prompt that has been waiting longest is the one
    // most likely to be holding something up.
    expect(asking.map((s) => s.id)).toEqual(['a', 'b', 'c']);
  });

  it('is empty when nothing is waiting, so the banner draws nothing', () => {
    expect(askingSessions([session('busy', null)])).toEqual([]);
    expect(askingSessions([])).toEqual([]);
  });
});

describe('the chord that opens the queue', () => {
  const chord = (over: Partial<Parameters<typeof isPermissionQueueChord>[0]> = {}) => ({
    code: 'KeyA',
    altKey: true,
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    ...over,
  });

  it('is ⌥⌘A, and ⌃ works where ⌘ does not', () => {
    expect(isPermissionQueueChord(chord())).toBe(true);
    expect(isPermissionQueueChord(chord({ metaKey: false, ctrlKey: true }))).toBe(true);
  });

  it('leaves the island’s ⌥A alone', () => {
    // The reason the two can coexist: every existing matcher in this app
    // rejects `metaKey`, and this one requires it.
    expect(isPermissionQueueChord(chord({ metaKey: false, ctrlKey: false }))).toBe(false);
  });

  it('ignores a plain ⌘A, which is select-all', () => {
    expect(isPermissionQueueChord(chord({ altKey: false }))).toBe(false);
  });

  it('is matched on the physical key, not the composed character', () => {
    // ⌥A on macOS produces `å`; `code` is what survives that.
    expect(isPermissionQueueChord(chord({ code: 'KeyB' }))).toBe(false);
    expect(isPermissionQueueChord(chord({ shiftKey: true }))).toBe(false);
  });
});

describe('every ask waiting, from either lane (T310-ANSWER)', () => {
  const run: AgentRun = {
    id: 'run-12345678' as AgentRunId,
    workstreamId: 'w1' as WorkstreamId,
    workspaceId: 'ws1' as AgentRun['workspaceId'],
    provider: 'CLAUDE_CODE',
    state: 'BLOCKED',
    startedAt: 0,
    updatedAt: 0,
    cancellationRequested: false,
    correlationId: 'c1' as AgentRun['correlationId'],
    pending: { askId: 'run-ask', tool: 'Bash', verb: 'RUN_COMMAND', target: 'ls', askedAt: 1_500 },
  };
  const ship = { id: 'w1', title: 'Ship it' } as Workstream;

  it('merges sessions and runs into one list, oldest ask first', () => {
    const asks = waitingAsks([session('a', 1_000), session('b', 2_000)], [run], [ship]);
    expect(asks.map((ask) => ask.askId)).toEqual(['ask-a', 'run-ask', 'ask-b']);
  });

  it('names a run by its workstream, or by its short id while that is loading', () => {
    expect(waitingAsks([], [run], [ship])[0]?.who).toBe('Ship it');
    expect(waitingAsks([], [run], [])[0]?.who).toBe('run-1234');
  });

  it('is what the status bar counts, so a run’s ask is not left out of it', () => {
    expect(waitingAsks([session('a', 1)], [run]).length).toBe(2);
    expect(waitingAsks([session('busy', null)], [{ ...run, pending: undefined }]).length).toBe(0);
  });
});
