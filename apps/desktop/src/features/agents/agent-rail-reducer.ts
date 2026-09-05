import type { AgentTimeline, CoreError } from '@gritty/protocol';

/**
 * Everything the rail holds that is not the roster (T231 §15).
 *
 * The roster itself is deliberately absent: the rail and the island read one
 * `AgentsController`, and a copy here would be a second roster that could
 * disagree with it. What is left is the rail's own state — whether it is open,
 * which session is selected, and the handle of the timeline the core is
 * following for it.
 *
 * Pure, so selection, switching and the "the session vanished" case are tested
 * without React, a core or a DOM.
 */
export interface AgentRailState {
  /** Expanded. Mirrored from the persisted layout blob, which owns it (T060). */
  readonly open: boolean;
  /**
   * The session whose timeline the core confirmed it is following. Nothing is
   * applied optimistically (T210): a row is not selected because it was
   * clicked, it is selected because `agents.openTimeline` returned.
   */
  readonly selectedId: string | null;
  /** The session `openTimeline` is in flight for, so a row can say so. */
  readonly pendingId: string | null;
  /** What that call returned. T232 renders it; T231 only holds it. */
  readonly timeline: AgentTimeline | null;
  /** A refusal, rendered as the envelope rather than swallowed (§16). */
  readonly error: CoreError | null;
}

export const initialAgentRailState: AgentRailState = {
  open: false,
  selectedId: null,
  pendingId: null,
  timeline: null,
  error: null,
};

export type AgentRailAction =
  /** Expand or collapse. Collapsing drops the follower with the view (§12). */
  | { readonly type: 'setOpen'; readonly open: boolean }
  /** A row was clicked: the request is in flight, nothing is selected yet. */
  | { readonly type: 'selecting'; readonly sessionId: string }
  | { readonly type: 'opened'; readonly sessionId: string; readonly timeline: AgentTimeline }
  | { readonly type: 'failed'; readonly sessionId: string; readonly error: CoreError }
  /** The selected row was clicked again, or the timeline was closed by hand. */
  | { readonly type: 'deselect' }
  /** The ids the roster now carries, so a selection that left it is dropped. */
  | { readonly type: 'roster'; readonly ids: readonly string[] }
  | { readonly type: 'clearError' };

/** Nothing followed, nothing pending, nothing to show for it. */
function cleared(state: AgentRailState): AgentRailState {
  if (state.selectedId === null && state.pendingId === null && state.timeline === null) {
    return state;
  }
  return { ...state, selectedId: null, pendingId: null, timeline: null };
}

export function agentRailReducer(state: AgentRailState, action: AgentRailAction): AgentRailState {
  switch (action.type) {
    case 'setOpen':
      if (action.open === state.open) return state;
      // An invisible follower is waste, and a timeline that kept growing
      // behind a collapsed rail would be a subscription nobody asked for.
      if (!action.open) return { ...cleared(state), open: false, error: null };
      return { ...state, open: true };

    case 'selecting':
      // The previous selection goes now rather than when the new one lands:
      // its follower is being closed, so showing it as still selected would
      // be a claim about a timeline that has stopped.
      return { ...cleared(state), pendingId: action.sessionId, error: null };

    case 'opened':
      // A late answer for a selection that has already moved on is dropped;
      // `pendingId` is what says which request is still the current one.
      if (state.pendingId !== action.sessionId) return state;
      return {
        ...state,
        selectedId: action.sessionId,
        pendingId: null,
        timeline: action.timeline,
        error: null,
      };

    case 'failed':
      if (state.pendingId !== action.sessionId) return state;
      // Nothing is selected: the core refused, so there is no timeline to
      // pretend otherwise about. The list keeps working around the envelope.
      return { ...state, pendingId: null, error: action.error };

    case 'deselect':
      return cleared(state);

    case 'roster': {
      // A session that *ended* is still on the roster — it stays selected and
      // its timeline simply stops growing, because removing a row under the
      // pointer is worse. This is the other case: the session left the roster
      // entirely, so there is no row left to be selected.
      const present = (id: string | null) => id !== null && action.ids.includes(id);
      const keepSelected = present(state.selectedId);
      const keepPending = present(state.pendingId);
      if (
        keepSelected === (state.selectedId !== null) &&
        keepPending === (state.pendingId !== null)
      )
        return state;
      return {
        ...state,
        selectedId: keepSelected ? state.selectedId : null,
        pendingId: keepPending ? state.pendingId : null,
        timeline: keepSelected ? state.timeline : null,
      };
    }

    case 'clearError':
      return state.error === null ? state : { ...state, error: null };

    default:
      return state;
  }
}

/** The session whose timeline the core is following, or is about to be. */
export function followedSession(state: AgentRailState): string | null {
  return state.pendingId ?? state.selectedId;
}
