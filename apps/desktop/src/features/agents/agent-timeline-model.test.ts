import type { AgentTimeline, AgentTimelineEntry, AgentTimelineKind } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { isPinnedToBottom, timelineRows, touchedFiles } from './agent-timeline-model';

/** The row at `index`, or a failure that names the index rather than `undefined`. */
function at<T>(items: readonly T[], index: number): T {
  const item = items[index];
  if (item === undefined) throw new Error(`nothing at index ${index} of ${items.length}`);
  return item;
}

let nextSeq = 0;

function entry(kind: AgentTimelineKind, overrides: Partial<AgentTimelineEntry> = {}) {
  return { seq: nextSeq++, kind, sidechain: false, ...overrides } satisfies AgentTimelineEntry;
}

function timelineOf(
  entries: readonly AgentTimelineEntry[],
  overrides: Partial<AgentTimeline> = {},
): AgentTimeline {
  return {
    sessionId: 'a',
    entries: [...entries],
    dropped: 0,
    restarted: false,
    transcriptEvidence: true,
    ...overrides,
  };
}

/** Every kind this build knows, one of each, in the order the wire declares. */
const EVERY_KIND: readonly AgentTimelineKind[] = [
  { type: 'USER_PROMPT', text: 'add a test' },
  { type: 'ASSISTANT_TEXT', text: 'on it' },
  { type: 'THINKING', text: 'where does that live' },
  { type: 'TOOL_CALL', toolUseId: 'call-1', name: 'Bash', detail: 'cargo test' },
  { type: 'TOOL_RESULT', toolUseId: 'orphan', ok: true, detail: '42 passed' },
  { type: 'FILE_TOUCHED', path: 'src/main.rs', display: 'src/main.rs' },
  { type: 'NOTICE', level: 'WARNING', text: 'a hook failed' },
  { type: 'PERMISSION_PENDING', tool: 'Bash', detail: 'rm -rf build' },
];

describe('timelineRows', () => {
  it('is total over the entry kinds', () => {
    const rows = timelineRows(timelineOf(EVERY_KIND.map((kind) => entry(kind))));
    // Every kind draws a row, and every row says something: an entry that
    // renders as nothing is an entry that vanished.
    expect(rows).toHaveLength(EVERY_KIND.length);
    for (const row of rows) {
      expect(row.label).not.toBe('');
      expect(row.text).not.toBe('');
    }
    expect(rows.map((row) => row.tone)).toEqual([
      'prompt',
      'assistant',
      'thinking',
      'tool',
      'result',
      'file',
      'notice',
      'permission',
    ]);
  });

  it('shows a kind from a newer core as text rather than dropping it', () => {
    // The protocol is additive: a core newer than this build can send a kind
    // the union above does not contain. Rendering nothing would make the
    // session look like it went quiet.
    const future = {
      type: 'SOMETHING_NEW',
      text: 'a kind from later',
    } as unknown as AgentTimelineKind;
    const rows = timelineRows(timelineOf([entry(future)]));
    expect(rows).toHaveLength(1);
    expect(at(rows, 0).tone).toBe('unknown');
    expect(at(rows, 0).text).toContain('a kind from later');
  });

  it('marks subagent work rather than hiding it', () => {
    // T220 dropped sidechain entries and listed that as a known limitation.
    const rows = timelineRows(
      timelineOf([
        entry({ type: 'ASSISTANT_TEXT', text: 'main' }),
        entry({ type: 'ASSISTANT_TEXT', text: 'sub' }, { sidechain: true }),
      ]),
    );
    expect(rows.map((row) => row.sidechain)).toEqual([false, true]);
  });

  it('pairs a tool result with its call by id', () => {
    const rows = timelineRows(
      timelineOf([
        entry({ type: 'TOOL_CALL', toolUseId: 'u1', name: 'Bash', detail: 'cargo test' }),
        entry({ type: 'TOOL_RESULT', toolUseId: 'u1', ok: false, detail: '1 failed' }),
      ]),
    );
    // One row, not two: the result attaches to its call rather than floating
    // below it where a reader has to match them up by eye.
    expect(rows).toHaveLength(1);
    expect(at(rows, 0).tone).toBe('tool');
    expect(at(rows, 0).text).toBe('Bash');
    expect(at(rows, 0).outcome).toEqual({ ok: false, detail: '1 failed' });
  });

  it('pairs across the entries between a call and its result', () => {
    const rows = timelineRows(
      timelineOf([
        entry({ type: 'TOOL_CALL', toolUseId: 'u1', name: 'Bash', detail: null }),
        entry({ type: 'ASSISTANT_TEXT', text: 'while that runs' }),
        entry({ type: 'TOOL_RESULT', toolUseId: 'u1', ok: true, detail: null }),
      ]),
    );
    expect(rows.map((row) => row.tone)).toEqual(['tool', 'assistant']);
    expect(at(rows, 0).outcome?.ok).toBe(true);
  });

  it('renders an unpaired call as still running', () => {
    const rows = timelineRows(
      timelineOf([entry({ type: 'TOOL_CALL', toolUseId: 'u1', name: 'Bash', detail: 'sleep 60' })]),
    );
    // `null` is what the renderer reads as running. Inventing a result for a
    // call the transcript has not answered would be a claim about a process
    // nobody watched finish.
    expect(at(rows, 0).outcome).toBeNull();
  });

  it('renders a result with no call of its own to attach to', () => {
    // A call that fell out of the ring, or a record with no `toolUseId` at all:
    // shown, because a result with no visible call is still something that
    // happened.
    const rows = timelineRows(
      timelineOf([
        entry({ type: 'TOOL_RESULT', toolUseId: null, ok: true, detail: 'done' }),
        entry({ type: 'TOOL_RESULT', toolUseId: 'long-gone', ok: false, detail: 'oops' }),
      ]),
    );
    expect(rows.map((row) => row.tone)).toEqual(['result', 'result']);
    expect(rows.map((row) => row.text)).toEqual(['Succeeded', 'Failed']);
  });

  it('does not pair calls that carry no id', () => {
    const rows = timelineRows(
      timelineOf([
        entry({ type: 'TOOL_CALL', toolUseId: null, name: 'Read', detail: null }),
        entry({ type: 'TOOL_RESULT', toolUseId: null, ok: true, detail: null }),
      ]),
    );
    // Nothing says these belong together, so nothing here claims they do.
    expect(rows).toHaveLength(2);
    expect(at(rows, 0).outcome).toBeNull();
  });

  it('draws a dropped head as an explicit gap', () => {
    const rows = timelineRows(
      timelineOf([entry({ type: 'ASSISTANT_TEXT', text: 'still here' })], { dropped: 12 }),
    );
    expect(at(rows, 0).tone).toBe('gap');
    expect(at(rows, 0).text).toContain('12 earlier entries dropped');
    // The entries themselves are untouched: the core reported its own loss so
    // that the UI could draw it, not so the UI could splice around it.
    expect(rows).toHaveLength(2);
    expect(at(rows, 1).text).toBe('still here');
  });

  it('counts a single dropped entry in the singular', () => {
    const rows = timelineRows(timelineOf([], { dropped: 1 }));
    expect(at(rows, 0).text).toContain('1 earlier entry dropped');
  });

  it('draws a restart as an explicit reset', () => {
    const rows = timelineRows(
      timelineOf([entry({ type: 'ASSISTANT_TEXT', text: 'after the reset' })], {
        restarted: true,
      }),
    );
    expect(at(rows, 0).tone).toBe('gap');
    expect(at(rows, 0).text).toContain('reset');
    expect(at(rows, 1).text).toBe('after the reset');
  });

  it('states a reset and a drop separately', () => {
    // Their order relative to each other is not knowable from what the core
    // sends, so merging them into one sentence would assert something neither
    // flag says.
    const rows = timelineRows(timelineOf([], { dropped: 3, restarted: true }));
    expect(rows.map((row) => row.tone)).toEqual(['gap', 'gap']);
    expect(rows.map((row) => row.key)).toEqual(['gap:restarted', 'gap:dropped']);
  });

  it('leaves an absent time absent', () => {
    // Stamping an entry with the read time would make a static timeline look
    // like it was moving; the core refuses to, and so does this.
    const rows = timelineRows(
      timelineOf([
        entry({ type: 'ASSISTANT_TEXT', text: 'no time' }),
        entry({ type: 'ASSISTANT_TEXT', text: 'timed' }, { at: 1_800_000_000_000 }),
      ]),
    );
    expect(at(rows, 0).at).toBeNull();
    expect(at(rows, 1).at).toBe(1_800_000_000_000);
  });

  it('gives every row a stable key', () => {
    const rows = timelineRows(
      timelineOf(
        EVERY_KIND.map((kind) => entry(kind)),
        { dropped: 1, restarted: true },
      ),
    );
    expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
  });
});

describe('touchedFiles', () => {
  it('deduplicates by path, most recent first, with the time it was last touched', () => {
    const files = touchedFiles([
      entry({ type: 'FILE_TOUCHED', path: 'src/a.rs', display: 'src/a.rs' }, { at: 1_000 }),
      entry({ type: 'FILE_TOUCHED', path: 'src/b.rs', display: 'src/b.rs' }, { at: 2_000 }),
      entry({ type: 'FILE_TOUCHED', path: 'src/a.rs', display: 'src/a.rs' }, { at: 3_000 }),
    ]);
    expect(files.map((file) => file.display)).toEqual(['src/a.rs', 'src/b.rs']);
    expect(at(files, 0).at).toBe(3_000);
  });

  it('yields no openable target for a file outside the workspace', () => {
    // The core sends no path for those, which is what stops anything outside
    // the root being opened from the renderer (ADR-010). This is that contract
    // arriving intact, not the view choosing to be careful.
    const files = touchedFiles([
      entry({ type: 'FILE_TOUCHED', path: null, display: '~/other/checkout/lib.rs' }),
      entry({ type: 'FILE_TOUCHED', path: 'src/main.rs', display: 'src/main.rs' }),
    ]);
    const outside = files.find((file) => file.display === '~/other/checkout/lib.rs');
    expect(outside?.openPath).toBeNull();
    expect(files.find((file) => file.display === 'src/main.rs')?.openPath).toBe('src/main.rs');
  });

  it('does not merge an outside file with an inside one that reads the same', () => {
    const files = touchedFiles([
      entry({ type: 'FILE_TOUCHED', path: null, display: 'src/main.rs' }),
      entry({ type: 'FILE_TOUCHED', path: 'src/main.rs', display: 'src/main.rs' }),
    ]);
    expect(files).toHaveLength(2);
  });

  it('ignores everything that is not a file', () => {
    expect(
      touchedFiles([
        entry({ type: 'ASSISTANT_TEXT', text: 'src/main.rs' }),
        entry({ type: 'TOOL_CALL', toolUseId: null, name: 'Edit', detail: 'src/main.rs' }),
      ]),
    ).toEqual([]);
  });

  it('orders by the entry, not by a timestamp that may be missing', () => {
    const files = touchedFiles([
      entry({ type: 'FILE_TOUCHED', path: 'src/a.rs', display: 'src/a.rs' }, { at: 9_000 }),
      entry({ type: 'FILE_TOUCHED', path: 'src/b.rs', display: 'src/b.rs' }),
    ]);
    expect(files.map((file) => file.display)).toEqual(['src/b.rs', 'src/a.rs']);
  });
});

describe('isPinnedToBottom', () => {
  it('is pinned at the bottom and while there is nothing to scroll', () => {
    expect(isPinnedToBottom({ scrollTop: 400, scrollHeight: 600, clientHeight: 200 })).toBe(true);
    expect(isPinnedToBottom({ scrollTop: 0, scrollHeight: 200, clientHeight: 200 })).toBe(true);
  });

  it('is not pinned once the reader has scrolled up', () => {
    // The whole point of §1: someone reading something further up is not
    // yanked to the bottom by the next batch.
    expect(isPinnedToBottom({ scrollTop: 100, scrollHeight: 600, clientHeight: 200 })).toBe(false);
  });

  it('tolerates the sub-pixel short fall a growing list leaves behind', () => {
    expect(isPinnedToBottom({ scrollTop: 399.6, scrollHeight: 600, clientHeight: 200 })).toBe(true);
  });
});
