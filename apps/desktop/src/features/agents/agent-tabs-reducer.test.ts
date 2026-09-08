import {
  type AgentTimeline,
  type AgentTimelineAppendedPayload,
  type AgentTimelineEntry,
  CoreError,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type AgentTabsAction,
  type AgentTabsState,
  agentTabsReducer,
  applyAppended,
  followedSession,
  initialAgentTabsState,
} from './agent-tabs-reducer';
import { MAX_TIMELINE_ENTRIES } from './agent-timeline-model';

function timeline(sessionId: string): AgentTimeline {
  return { sessionId, entries: [], dropped: 0, restarted: false, transcriptEvidence: true };
}

/** One assistant line at `seq`; the kind does not matter to the accumulation. */
function said(seq: number, text: string): AgentTimelineEntry {
  return { seq, kind: { type: 'ASSISTANT_TEXT', text }, sidechain: false };
}

/**
 * The prompt a session is stopped at, as the core sends it: outside the ring,
 * so `seq` is the sentinel the core stamps rather than a position in it.
 */
function prompt(tool: string, detail: string | null): AgentTimelineEntry {
  return {
    seq: Number.MAX_SAFE_INTEGER,
    kind: { type: 'PERMISSION_PENDING', tool, detail },
    sidechain: false,
  };
}

function batch(
  sessionId: string,
  entries: readonly AgentTimelineEntry[],
  extra: Partial<AgentTimelineAppendedPayload> = {},
): AgentTimelineAppendedPayload {
  return { sessionId, entries: [...entries], dropped: 0, restarted: false, ...extra };
}

// `openTimeline` on a session that left the roster between the click and the
// call is the ordinary refusal here, not an exotic one.
const refusal = new CoreError({
  code: 'NotFound',
  message: 'no such session',
  retryable: false,
});

/** Apply a run of actions from the initial state, which is how the UI gets there. */
function run(...actions: readonly AgentTabsAction[]): AgentTabsState {
  return actions.reduce(agentTabsReducer, initialAgentTabsState);
}

/** Open one console and let its timeline land — the ordinary path. */
function showing(sessionId: string, preview = true): AgentTabsState {
  return run(
    { type: 'open', sessionId, preview },
    { type: 'opened', sessionId, timeline: timeline(sessionId) },
  );
}

describe('agent console tabs', () => {
  it('starts with no tabs and a document showing', () => {
    expect(initialAgentTabsState.open).toEqual([]);
    expect(initialAgentTabsState.activeId).toBeNull();
    expect(initialAgentTabsState.timeline).toBeNull();
  });

  it('activates the tab on the click but shows no feed until the core answers', () => {
    // A tab is a control and has to answer at once; the timeline is the thing
    // that must not be faked.
    const opening = run({ type: 'open', sessionId: 's1', preview: true });
    expect(opening.activeId).toBe('s1');
    expect(opening.open).toEqual(['s1']);
    expect(opening.pendingId).toBe('s1');
    expect(opening.timeline).toBeNull();

    const open = agentTabsReducer(opening, {
      type: 'opened',
      sessionId: 's1',
      timeline: timeline('s1'),
    });
    expect(open.timeline?.sessionId).toBe('s1');
    expect(open.pendingId).toBeNull();
  });

  describe("preview semantics, exactly a file's (T041)", () => {
    it('replaces the previous preview rather than accumulating tabs', () => {
      const state = run(
        { type: 'open', sessionId: 's1', preview: true },
        { type: 'open', sessionId: 's2', preview: true },
      );
      expect(state.open).toEqual(['s2']);
      expect(state.previewId).toBe('s2');
    });

    it('keeps a tab on the second, deliberate open', () => {
      // The double click fires its single click first, so this is the order
      // that actually arrives.
      const state = run(
        { type: 'open', sessionId: 's1', preview: true },
        { type: 'keep', sessionId: 's1' },
        { type: 'open', sessionId: 's2', preview: true },
      );
      expect(state.open).toEqual(['s1', 's2']);
      expect(state.previewId).toBe('s2');
    });

    it('does not make an open tab a preview by clicking it again', () => {
      const state = run(
        { type: 'open', sessionId: 's1', preview: true },
        { type: 'keep', sessionId: 's1' },
        { type: 'open', sessionId: 's2', preview: true },
        { type: 'open', sessionId: 's1', preview: true },
      );
      expect(state.previewId).toBe('s2');
      expect(state.open).toEqual(['s1', 's2']);
    });
  });

  it('drops the previous feed the moment another console is shown', () => {
    // Its follower is being closed, so keeping it on screen would be a claim
    // about a feed that has stopped.
    const state = agentTabsReducer(showing('s1', false), {
      type: 'open',
      sessionId: 's2',
      preview: true,
    });
    expect(state.activeId).toBe('s2');
    expect(state.timeline).toBeNull();
    expect(state.pendingId).toBe('s2');
  });

  it('ignores an answer for a console that has already moved on', () => {
    const state = run(
      { type: 'open', sessionId: 's1', preview: false },
      { type: 'open', sessionId: 's2', preview: false },
      { type: 'opened', sessionId: 's1', timeline: timeline('s1') },
    );
    expect(state.timeline).toBeNull();
    expect(state.pendingId).toBe('s2');
  });

  it('keeps the tab but shows no feed when the core refuses', () => {
    const state = run(
      { type: 'open', sessionId: 's1', preview: true },
      { type: 'failed', sessionId: 's1', error: refusal },
    );
    // The user opened the tab and it is theirs to close; what is absent is the
    // feed, not the tab.
    expect(state.open).toEqual(['s1']);
    expect(state.activeId).toBe('s1');
    expect(state.timeline).toBeNull();
    expect(state.error?.code).toBe('NotFound');
  });

  describe('handing the pane back', () => {
    it('drops the follower when a document is activated', () => {
      const state = agentTabsReducer(showing('s1'), { type: 'documents' });
      expect(state.activeId).toBeNull();
      expect(state.timeline).toBeNull();
      // The tab stays open: it was not closed, it is just not on screen.
      expect(state.open).toEqual(['s1']);
    });

    it('shows the documents again when the active console is closed', () => {
      const state = agentTabsReducer(showing('s1'), { type: 'close', sessionId: 's1' });
      expect(state.open).toEqual([]);
      expect(state.activeId).toBeNull();
      expect(state.timeline).toBeNull();
    });

    it('leaves the shown console alone when another tab is closed', () => {
      const state = run(
        { type: 'open', sessionId: 's1', preview: false },
        { type: 'open', sessionId: 's2', preview: false },
        { type: 'opened', sessionId: 's2', timeline: timeline('s2') },
        { type: 'close', sessionId: 's1' },
      );
      expect(state.open).toEqual(['s2']);
      expect(state.activeId).toBe('s2');
      expect(state.timeline?.sessionId).toBe('s2');
    });
  });

  describe('a session leaving the roster', () => {
    it('closes its tab', () => {
      const state = agentTabsReducer(showing('s1'), { type: 'roster', ids: ['s2'] });
      expect(state.open).toEqual([]);
      expect(state.activeId).toBeNull();
      expect(state.timeline).toBeNull();
    });

    it('keeps a session that merely ended, because it is still listed', () => {
      // Its feed stops growing, which is what happened. Closing the tab under
      // the pointer would be worse.
      const state = agentTabsReducer(showing('s1'), { type: 'roster', ids: ['s1'] });
      expect(state.open).toEqual(['s1']);
      expect(state.timeline?.sessionId).toBe('s1');
    });

    it('leaves an unrelated tab showing', () => {
      const state = run(
        { type: 'open', sessionId: 's1', preview: false },
        { type: 'open', sessionId: 's2', preview: false },
        { type: 'opened', sessionId: 's2', timeline: timeline('s2') },
        { type: 'roster', ids: ['s2'] },
      );
      expect(state.open).toEqual(['s2']);
      expect(state.activeId).toBe('s2');
    });
  });

  it('names the session the core is following, pending or settled', () => {
    expect(followedSession(initialAgentTabsState)).toBeNull();
    expect(followedSession(run({ type: 'open', sessionId: 's1', preview: true }))).toBe('s1');
    expect(followedSession(showing('s1'))).toBe('s1');
    expect(followedSession(agentTabsReducer(showing('s1'), { type: 'documents' }))).toBeNull();
  });

  it('appends the delta rather than waiting for a snapshot', () => {
    const state = agentTabsReducer(showing('s1'), {
      type: 'appended',
      payload: batch('s1', [said(1, 'one')]),
    });
    expect(state.timeline?.entries).toHaveLength(1);
  });

  it('ignores a delta for a session it is not showing', () => {
    // The subscription is machine-wide; the reducer is what filters it.
    const base = showing('s1');
    expect(agentTabsReducer(base, { type: 'appended', payload: batch('s2', [said(1, 'x')]) })).toBe(
      base,
    );
  });

  it('ignores a delta that arrives with no console showing', () => {
    expect(
      agentTabsReducer(initialAgentTabsState, {
        type: 'appended',
        payload: batch('s1', [said(1, 'x')]),
      }),
    ).toBe(initialAgentTabsState);
  });

  it('records a touched file being opened, and the refusal when it is not', () => {
    const opening = agentTabsReducer(showing('s1'), { type: 'openingFile', path: 'src/a.ts' });
    expect(opening.openingPath).toBe('src/a.ts');
    const failed = agentTabsReducer(opening, {
      type: 'openFileFailed',
      path: 'src/a.ts',
      error: refusal,
    });
    expect(failed.openingPath).toBeNull();
    expect(failed.error?.code).toBe('NotFound');
  });

  it('leaves a newer open alone when an older one finally answers', () => {
    const state = run(
      { type: 'open', sessionId: 's1', preview: false },
      { type: 'opened', sessionId: 's1', timeline: timeline('s1') },
      { type: 'openingFile', path: 'src/a.ts' },
      { type: 'openingFile', path: 'src/b.ts' },
      { type: 'openedFile', path: 'src/a.ts' },
    );
    expect(state.openingPath).toBe('src/b.ts');
  });

  it('returns the same state for a no-op, so React does not re-render', () => {
    const base = showing('s1');
    expect(agentTabsReducer(base, { type: 'clearError' })).toBe(base);
    expect(agentTabsReducer(base, { type: 'close', sessionId: 'nobody' })).toBe(base);
    expect(agentTabsReducer(base, { type: 'keep', sessionId: 'nobody' })).toBe(base);
    expect(agentTabsReducer(base, { type: 'roster', ids: ['s1'] })).toBe(base);
  });
});

describe('applying a timeline delta', () => {
  const held: AgentTimeline = {
    sessionId: 'a',
    entries: [said(0, 'one'), said(1, 'two')],
    dropped: 0,
    restarted: false,
    transcriptEvidence: true,
  };

  it('appends by seq', () => {
    const next = applyAppended(held, batch('a', [said(2, 'three')]));
    expect(next.entries.map((entry) => entry.seq)).toEqual([0, 1, 2]);
  });

  it('does not append an entry it already holds', () => {
    // `openTimeline` forgets the batch it answered with, so this should never
    // happen; if it does, drawing the same turn twice is worse than dropping it.
    const next = applyAppended(held, batch('a', [said(1, 'two'), said(2, 'three')]));
    expect(next.entries.map((entry) => entry.seq)).toEqual([0, 1, 2]);
  });

  it('reports a drop as a running total instead of splicing around it', () => {
    const next = applyAppended(held, batch('a', [said(2, 'three')], { dropped: 7 }));
    // The entries either side of the gap are kept exactly as they arrived: the
    // core counted what it lost so the UI can draw the loss, not hide it.
    expect(next.entries.map((entry) => entry.seq)).toEqual([0, 1, 2]);
    expect(next.dropped).toBe(7);
    // A second batch adds to the total rather than replacing it.
    expect(applyAppended(next, batch('a', [said(3, 'four')], { dropped: 2 })).dropped).toBe(9);
  });

  it('drops what it held on a restart rather than joining two transcripts', () => {
    const next = applyAppended(held, batch('a', [said(9, 'fresh')], { restarted: true }));
    expect(next.entries.map((entry) => entry.seq)).toEqual([9]);
    // Sticky, matching the core's own snapshot: the reset happened, and a later
    // quiet tick does not unhappen it.
    expect(next.restarted).toBe(true);
    expect(applyAppended(next, batch('a', [said(10, 'more')])).restarted).toBe(true);
  });

  it('does not grow past the ring the core keeps', () => {
    const entries = Array.from({ length: MAX_TIMELINE_ENTRIES }, (_, index) =>
      said(index, `line ${index}`),
    );
    const full: AgentTimeline = { ...held, entries };
    const next = applyAppended(
      full,
      // What the core reports for exactly this overflow: it dropped the oldest
      // three when the same three entries pushed its ring past the bound.
      batch('a', [said(500, 'a'), said(501, 'b'), said(502, 'c')], { dropped: 3 }),
    );
    expect(next.entries).toHaveLength(MAX_TIMELINE_ENTRIES);
    expect(next.entries[0]?.seq).toBe(3);
    // Counted once, not once per side.
    expect(next.dropped).toBe(3);
  });

  it('leaves transcript evidence to the core', () => {
    // The delta does not carry it, and reconstructing it here would be the UI
    // asserting something the core did not send (a T230 follow-up).
    const blind: AgentTimeline = { ...held, transcriptEvidence: false };
    expect(applyAppended(blind, batch('a', [said(2, 'three')])).transcriptEvidence).toBe(false);
  });

  it('takes the permission prompt whole, and lets a later batch replace it', () => {
    const next = applyAppended(held, batch('a', [], { pending: prompt('Bash', 'cargo test') }));
    expect(next.pending?.kind).toEqual({
      type: 'PERMISSION_PENDING',
      tool: 'Bash',
      detail: 'cargo test',
    });
    // A state, not a delta: the second prompt replaces the first rather than
    // queueing behind it.
    const second = applyAppended(next, batch('a', [], { pending: prompt('Edit', 'src/lib.rs') }));
    expect(second.pending?.kind).toMatchObject({ tool: 'Edit' });
  });

  it('clears the prompt when a batch carries none', () => {
    const waiting = applyAppended(held, batch('a', [], { pending: prompt('Bash', null) }));
    const answered = applyAppended(waiting, batch('a', [said(2, 'three')]));
    // Absent means the session left the prompt. Keeping it would leave a row in
    // the feed forever claiming to be waiting on someone who already answered.
    expect(answered.pending).toBeUndefined();
  });
});
