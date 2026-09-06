import {
  type AgentTimeline,
  type AgentTimelineAppendedPayload,
  type AgentTimelineEntry,
  CoreError,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type AgentRailAction,
  type AgentRailState,
  agentRailReducer,
  applyAppended,
  followedSession,
  initialAgentRailState,
} from './agent-rail-reducer';
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
function run(...actions: readonly AgentRailAction[]): AgentRailState {
  return actions.reduce(agentRailReducer, initialAgentRailState);
}

/** Open the rail and follow one session, the ordinary path. */
function following(sessionId: string): AgentRailState {
  return run(
    { type: 'setOpen', open: true },
    { type: 'selecting', sessionId },
    { type: 'opened', sessionId, timeline: timeline(sessionId) },
  );
}

describe('agent rail reducer', () => {
  it('starts collapsed with nothing selected', () => {
    expect(initialAgentRailState).toEqual({
      open: false,
      selectedId: null,
      pendingId: null,
      timeline: null,
      openingPath: null,
      error: null,
    });
  });

  it('does not select a row until the core answers', () => {
    const selecting = run({ type: 'setOpen', open: true }, { type: 'selecting', sessionId: 'a' });
    // Nothing optimistic (T210): the click is a request, not a state change.
    expect(selecting.selectedId).toBeNull();
    expect(selecting.pendingId).toBe('a');
    expect(selecting.timeline).toBeNull();

    const opened = agentRailReducer(selecting, {
      type: 'opened',
      sessionId: 'a',
      timeline: timeline('a'),
    });
    expect(opened.selectedId).toBe('a');
    expect(opened.pendingId).toBeNull();
    expect(opened.timeline?.sessionId).toBe('a');
  });

  it('drops the previous selection the moment another row is clicked', () => {
    // Its follower is being closed, so showing it as still selected would be a
    // claim about a timeline that has stopped.
    const switching = agentRailReducer(following('a'), { type: 'selecting', sessionId: 'b' });
    expect(switching.selectedId).toBeNull();
    expect(switching.timeline).toBeNull();
    expect(switching.pendingId).toBe('b');

    const landed = agentRailReducer(switching, {
      type: 'opened',
      sessionId: 'b',
      timeline: timeline('b'),
    });
    expect(landed.selectedId).toBe('b');
  });

  it('ignores an answer for a selection that has already moved on', () => {
    const state = run(
      { type: 'setOpen', open: true },
      { type: 'selecting', sessionId: 'a' },
      { type: 'selecting', sessionId: 'b' },
      // `a` finally answers, after the user asked for `b`.
      { type: 'opened', sessionId: 'a', timeline: timeline('a') },
    );
    expect(state.pendingId).toBe('b');
    expect(state.selectedId).toBeNull();

    const stale = agentRailReducer(state, { type: 'failed', sessionId: 'a', error: refusal });
    expect(stale).toBe(state);
  });

  it('renders a refusal instead of selecting the row anyway', () => {
    const failed = run(
      { type: 'setOpen', open: true },
      { type: 'selecting', sessionId: 'a' },
      { type: 'failed', sessionId: 'a', error: refusal },
    );
    expect(failed.selectedId).toBeNull();
    expect(failed.pendingId).toBeNull();
    expect(failed.error).toBe(refusal);

    expect(agentRailReducer(failed, { type: 'clearError' }).error).toBeNull();
  });

  it('closes the timeline when the rail collapses', () => {
    // An invisible follower is waste, and it is the rail's to close.
    const collapsed = agentRailReducer(following('a'), { type: 'setOpen', open: false });
    expect(collapsed).toEqual(initialAgentRailState);

    // Reopening restores an empty rail, not the selection it had before.
    expect(agentRailReducer(collapsed, { type: 'setOpen', open: true }).selectedId).toBeNull();
  });

  it('clears a selection that left the roster', () => {
    const gone = agentRailReducer(following('a'), { type: 'roster', ids: ['b', 'c'] });
    expect(gone.selectedId).toBeNull();
    expect(gone.timeline).toBeNull();
    expect(gone.open).toBe(true);
  });

  it('keeps a selected session that is merely still listed', () => {
    // A session that ended stays on the roster: the row goes to Ended and its
    // timeline stops growing, but it does not vanish under the pointer.
    const state = following('a');
    expect(agentRailReducer(state, { type: 'roster', ids: ['a', 'b'] })).toBe(state);
  });

  it('drops a pending selection that left the roster before it answered', () => {
    const state = run({ type: 'setOpen', open: true }, { type: 'selecting', sessionId: 'a' });
    const gone = agentRailReducer(state, { type: 'roster', ids: [] });
    expect(gone.pendingId).toBeNull();
    // A late answer for it can no longer select anything.
    expect(
      agentRailReducer(gone, { type: 'opened', sessionId: 'a', timeline: timeline('a') })
        .selectedId,
    ).toBeNull();
  });

  it('deselects the open row without touching the rail', () => {
    const closed = agentRailReducer(following('a'), { type: 'deselect' });
    expect(closed.selectedId).toBeNull();
    expect(closed.timeline).toBeNull();
    expect(closed.open).toBe(true);
  });

  it('names the session the core is following, pending or settled', () => {
    expect(followedSession(initialAgentRailState)).toBeNull();
    expect(
      followedSession(run({ type: 'setOpen', open: true }, { type: 'selecting', sessionId: 'a' })),
    ).toBe('a');
    expect(followedSession(following('a'))).toBe('a');
  });

  it('appends the delta rather than waiting for a snapshot', () => {
    // `AgentTimelineAppended` is a delta by design: a timeline grows without
    // bound, so the core never resends it. Anything that assumed a snapshot
    // would show one batch and lose everything before it.
    const state = run(
      { type: 'setOpen', open: true },
      { type: 'selecting', sessionId: 'a' },
      { type: 'opened', sessionId: 'a', timeline: { ...timeline('a'), entries: [said(0, 'one')] } },
      { type: 'appended', payload: batch('a', [said(1, 'two')]) },
      { type: 'appended', payload: batch('a', [said(2, 'three')]) },
    );
    expect(state.timeline?.entries.map((entry) => entry.seq)).toEqual([0, 1, 2]);
  });

  it('ignores a delta for a session it is not showing', () => {
    // The subscription is bound once for whatever is selected under it, so a
    // batch for another session reaches this reducer and must be dropped.
    const state = following('a');
    expect(agentRailReducer(state, { type: 'appended', payload: batch('b', [said(0, 'x')]) })).toBe(
      state,
    );
  });

  it('ignores a delta that arrives with nothing selected', () => {
    const state = run({ type: 'setOpen', open: true });
    expect(agentRailReducer(state, { type: 'appended', payload: batch('a', [said(0, 'x')]) })).toBe(
      state,
    );
  });

  it('records a touched file being opened, and the refusal when it is not', () => {
    const opening = agentRailReducer(following('a'), { type: 'openingFile', path: 'src/main.rs' });
    expect(opening.openingPath).toBe('src/main.rs');

    const failed = agentRailReducer(opening, {
      type: 'openFileFailed',
      path: 'src/main.rs',
      error: refusal,
    });
    // The list is history, not a live directory: the agent may have deleted the
    // file since it wrote it, and the row stays either way.
    expect(failed.openingPath).toBeNull();
    expect(failed.error).toBe(refusal);
    expect(failed.timeline).not.toBeNull();
  });

  it('leaves a newer open alone when an older one finally answers', () => {
    const state = run(
      { type: 'setOpen', open: true },
      { type: 'selecting', sessionId: 'a' },
      { type: 'opened', sessionId: 'a', timeline: timeline('a') },
      { type: 'openingFile', path: 'a.rs' },
      { type: 'openingFile', path: 'b.rs' },
    );
    expect(agentRailReducer(state, { type: 'openedFile', path: 'a.rs' })).toBe(state);
    expect(agentRailReducer(state, { type: 'openedFile', path: 'b.rs' }).openingPath).toBeNull();
  });

  it('returns the same state for a no-op, so React does not re-render', () => {
    const state = following('a');
    expect(agentRailReducer(state, { type: 'setOpen', open: true })).toBe(state);
    expect(agentRailReducer(state, { type: 'clearError' })).toBe(state);
    expect(agentRailReducer(state, { type: 'roster', ids: ['a'] })).toBe(state);
    expect(agentRailReducer(state, { type: 'deselect' })).not.toBe(state);
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
