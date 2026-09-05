import { type AgentTimeline, CoreError } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type AgentRailAction,
  type AgentRailState,
  agentRailReducer,
  followedSession,
  initialAgentRailState,
} from './agent-rail-reducer';

function timeline(sessionId: string): AgentTimeline {
  return { sessionId, entries: [], dropped: 0, restarted: false, transcriptEvidence: true };
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

  it('returns the same state for a no-op, so React does not re-render', () => {
    const state = following('a');
    expect(agentRailReducer(state, { type: 'setOpen', open: true })).toBe(state);
    expect(agentRailReducer(state, { type: 'clearError' })).toBe(state);
    expect(agentRailReducer(state, { type: 'roster', ids: ['a'] })).toBe(state);
    expect(agentRailReducer(state, { type: 'deselect' })).not.toBe(state);
  });
});
