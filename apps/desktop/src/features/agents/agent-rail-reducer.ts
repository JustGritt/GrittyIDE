import type { AgentTimeline, AgentTimelineAppendedPayload, CoreError } from '@gritty/protocol';
import { MAX_TIMELINE_ENTRIES } from './agent-timeline-model';

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
  /**
   * The timeline as it stands now: what `openTimeline` returned, plus every
   * `AgentTimelineAppended` delta since (T232). Never a snapshot the core
   * resent — a timeline grows without bound, so the core sends deltas and this
   * is where they accumulate.
   */
  readonly timeline: AgentTimeline | null;
  /**
   * The touched file `documents.open` is in flight for, so its row can say so
   * and a second click cannot start a second open (T232 §11).
   */
  readonly openingPath: string | null;
  /** A refusal, rendered as the envelope rather than swallowed (§16). */
  readonly error: CoreError | null;
}

export const initialAgentRailState: AgentRailState = {
  open: false,
  selectedId: null,
  pendingId: null,
  timeline: null,
  openingPath: null,
  error: null,
};

export type AgentRailAction =
  /** Expand or collapse. Collapsing drops the follower with the view (§12). */
  | { readonly type: 'setOpen'; readonly open: boolean }
  /** A row was clicked: the request is in flight, nothing is selected yet. */
  | { readonly type: 'selecting'; readonly sessionId: string }
  | { readonly type: 'opened'; readonly sessionId: string; readonly timeline: AgentTimeline }
  | { readonly type: 'failed'; readonly sessionId: string; readonly error: CoreError }
  /** An `AgentTimelineAppended` delta for whichever session it names. */
  | { readonly type: 'appended'; readonly payload: AgentTimelineAppendedPayload }
  /** A touched file was clicked: `documents.status` then `documents.open`. */
  | { readonly type: 'openingFile'; readonly path: string }
  | { readonly type: 'openedFile'; readonly path: string }
  | { readonly type: 'openFileFailed'; readonly path: string; readonly error: CoreError }
  /** The selected row was clicked again, or the timeline was closed by hand. */
  | { readonly type: 'deselect' }
  /** The ids the roster now carries, so a selection that left it is dropped. */
  | { readonly type: 'roster'; readonly ids: readonly string[] }
  | { readonly type: 'clearError' };

/** Nothing followed, nothing pending, nothing to show for it. */
function cleared(state: AgentRailState): AgentRailState {
  if (
    state.selectedId === null &&
    state.pendingId === null &&
    state.timeline === null &&
    state.openingPath === null
  ) {
    return state;
  }
  return { ...state, selectedId: null, pendingId: null, timeline: null, openingPath: null };
}

/**
 * Fold one `AgentTimelineAppended` delta into the timeline this client holds.
 *
 * The event is a **delta, not a snapshot** — deliberately, because a timeline
 * grows without bound and resending it per tick would be the unbounded
 * forwarding rule 8 forbids. So this appends, keyed on `seq`, and never assumes
 * it has been handed the whole timeline. A consumer that missed an event calls
 * `agents.timeline` to resynchronise; nothing here tries to detect that, since
 * a gap the client invented would be indistinguishable from one the core
 * reported.
 *
 * The two losses the core reports are carried forward, not repaired:
 *
 * - `restarted` means the transcript this view was built from is gone, so the
 *   entries held are dropped rather than spliced onto the new ones. `seq` keeps
 *   increasing across the reset, so appending still works afterwards.
 * - `dropped` accumulates. The snapshot's value is a running total and each
 *   batch's is the delta since — `openTimeline` discards the pending batch it
 *   already answered with, so the two never count the same entry twice.
 */
export function applyAppended(
  timeline: AgentTimeline,
  batch: AgentTimelineAppendedPayload,
): AgentTimeline {
  const base = batch.restarted ? [] : timeline.entries;
  const last = base[base.length - 1]?.seq ?? null;
  // Defence in depth rather than an expected case: `openTimeline` forgets the
  // batch it answered with, so a redelivered entry should never arrive. If one
  // does, appending it would draw the same turn twice.
  const fresh = last === null ? batch.entries : batch.entries.filter((entry) => entry.seq > last);
  const combined = [...base, ...fresh];
  // The core's ring is what bounds this; trimming here only stops a client
  // that somehow ran ahead of it from growing forever. In the ordinary case
  // `overflow` is exactly `batch.dropped` — the same entries, counted once by
  // each side — which is why the larger of the two is taken rather than a sum.
  const overflow = Math.max(0, combined.length - MAX_TIMELINE_ENTRIES);
  return {
    ...timeline,
    entries: overflow > 0 ? combined.slice(overflow) : combined,
    dropped: timeline.dropped + Math.max(batch.dropped, overflow),
    // Sticky, matching the core's own snapshot: the reset happened, and a later
    // quiet tick does not unhappen it.
    restarted: timeline.restarted || batch.restarted,
    // Replaced, never appended: the permission prompt is a *state*, so every
    // batch carries its whole current value and `undefined` means the session
    // is not stopped at one any more (T221). Merging it the way entries are
    // merged would leave an answered prompt in the feed forever, claiming to
    // be waiting — which is exactly why the core holds it beside the ring
    // rather than in it.
    pending: batch.pending,
  };
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

    case 'appended': {
      // A delta for a session this rail is not showing: the subscription is
      // machine-wide and another surface may be following something else.
      if (state.timeline === null || state.timeline.sessionId !== action.payload.sessionId) {
        return state;
      }
      return { ...state, timeline: applyAppended(state.timeline, action.payload) };
    }

    case 'openingFile':
      return { ...state, openingPath: action.path, error: null };

    case 'openedFile':
      // A late answer for a file that is no longer the one being opened leaves
      // the newer request's spinner alone.
      if (state.openingPath !== action.path) return state;
      return { ...state, openingPath: null };

    case 'openFileFailed':
      if (state.openingPath !== action.path) return state;
      // The refusal is the rail's, not the editor pane's: nothing was opened,
      // so nothing there could carry it. The feed keeps updating around it.
      return { ...state, openingPath: null, error: action.error };

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
        openingPath: keepSelected ? state.openingPath : null,
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
