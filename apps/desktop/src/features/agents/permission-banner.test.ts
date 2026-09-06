import type { AgentSession } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { askingSessions, isPermissionQueueChord } from './permission-banner';

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
