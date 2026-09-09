import type {
  AgentRunId,
  AgentTimeline,
  AgentTimelineAppendedPayload,
  CoreError,
} from '@gritty/protocol';
import { MAX_TIMELINE_ENTRIES } from './agent-timeline-model';

/**
 * The agent consoles open as editor tabs, and the one timeline being followed
 * (T233-C, ADR-026).
 *
 * This is `agent-rail-reducer.ts` after the rail became a tab. What changed is
 * the container — a list of open tabs with a preview among them, instead of a
 * single expanded/collapsed flag — and what did **not** change is every rule
 * about the follower: one at a time, closed before the next opens, nothing
 * optimistic about the timeline, and a late answer for a request that has moved
 * on is dropped.
 *
 * The roster is deliberately absent, as it was in the rail: the section, the
 * island and the console read one `AgentsController`, and a copy here would be
 * a second roster that could disagree with it.
 *
 * **Nothing here may ever reach `documents.state`.** That is the whole of why
 * ADR-026 costs no migration: `buildSessionTabs` walks `documents.state.order`,
 * so a console that is not in it can never become a `SessionTab` and never
 * reaches the non-nullable `TabRow.path` column T231 priced the alternative on.
 *
 * Pure, so opening, previewing, switching, closing and the "the session
 * vanished" case are tested without React, a core or a DOM.
 *
 * **A tab is keyed on a console key, not a session id** (T310-U). A run Gritty
 * owns is deliberately absent from the observed roster (T310-B §8), so it can
 * never arrive here as a session and the two lanes cannot share an id space.
 * The key says which lane the tab belongs to, and it is the same string
 * `roster-model.ts` already mints for the sidebar row — so the row a user
 * clicks and the tab it opens are the same identity rather than two spellings
 * of one.
 */

/**
 * A console tab's identity: `run:<id>` or `session:<id>`.
 *
 * A plain string rather than a branded type, because it is a *display*
 * identity assembled from two domains that never meet. `parseConsoleKey` is
 * what turns it back into whichever id the caller actually needs, and it is
 * the only thing that may.
 */
export type ConsoleKey = string;

/** Which lane a console key names, and the id inside it. */
export type ConsoleTarget =
  | { readonly kind: 'run'; readonly runId: AgentRunId }
  | { readonly kind: 'session'; readonly sessionId: string };

/** The key for a run Gritty owns. Agrees with `rosterRows`, which is asserted. */
export function runConsoleKey(runId: AgentRunId): ConsoleKey {
  return `run:${runId}`;
}

/** The key for a session Gritty merely observes. */
export function sessionConsoleKey(sessionId: string): ConsoleKey {
  return `session:${sessionId}`;
}

/**
 * The lane and id a key names, or `null` for one this build cannot read.
 *
 * Split on the *first* colon only, so an id containing one survives verbatim.
 * `null` rather than a throw: a key comes from a row, and a row from a newer
 * build could carry a prefix this one has no console for — nothing should open,
 * and nothing should crash.
 */
export function parseConsoleKey(key: ConsoleKey): ConsoleTarget | null {
  const colon = key.indexOf(':');
  if (colon <= 0) return null;
  const id = key.slice(colon + 1);
  if (id === '') return null;
  const prefix = key.slice(0, colon);
  if (prefix === 'run') return { kind: 'run', runId: id as AgentRunId };
  if (prefix === 'session') return { kind: 'session', sessionId: id };
  return null;
}

export interface AgentTabsState {
  /** Console keys with a tab, in the order they were opened. */
  readonly open: readonly ConsoleKey[];
  /**
   * The one tab opened for a look rather than to keep: at most one exists, and
   * opening another preview replaces it. `null` when every open tab is kept.
   * Exactly `DocumentsState.previewId`'s rule, for exactly its reason (T041).
   */
  readonly previewId: ConsoleKey | null;
  /**
   * The tab showing now, or `null` when a document is showing instead.
   *
   * This one *is* applied on the click, unlike the rail's `selectedId`. A tab
   * is a control the user operates and it has to answer immediately; what must
   * not be faked is the *feed*, which stays absent until the core answers. The
   * console says "Opening…" in between rather than drawing one it does not have.
   */
  readonly activeId: ConsoleKey | null;
  /**
   * The console whose feed is being fetched: `agents.timeline` for a session,
   * `runs.events` for a run. One at a time, because only the tab on screen has
   * a feed at all.
   */
  readonly pendingId: ConsoleKey | null;
  /**
   * The console whose feed request has landed.
   *
   * It exists because "is this console's feed already here" cannot be read off
   * `timeline` for both kinds: a run's events live in `useRuns`, not here, so a
   * run tab would otherwise re-fetch its ring on every click of a tab that is
   * already open. It is also what makes a *retry* work — a refusal clears it,
   * so clicking the tab again asks again.
   */
  readonly loadedId: ConsoleKey | null;
  /**
   * The timeline as it stands: what `openTimeline` returned, plus every
   * `AgentTimelineAppended` delta since (T232). Never a snapshot the core
   * resent. Only the active tab has one — an inactive console is not followed,
   * because an invisible follower is waste.
   *
   * Always `null` for a run's tab: a run has no timeline, its feed is the
   * event ring `useRuns` holds, and inventing one here would be the second
   * source of truth §6 rejects.
   */
  readonly timeline: AgentTimeline | null;
  /**
   * The touched file `documents.open` is in flight for, so its row can say so
   * and a second click cannot start a second open (T232 §11).
   */
  readonly openingPath: string | null;
  /** A refusal, rendered as the envelope rather than swallowed. */
  readonly error: CoreError | null;
}

export const initialAgentTabsState: AgentTabsState = {
  open: [],
  previewId: null,
  activeId: null,
  pendingId: null,
  loadedId: null,
  timeline: null,
  openingPath: null,
  error: null,
};

export type AgentTabsAction =
  /**
   * Open a console, or activate one that is already open. `preview` is the
   * single click; a deliberate open keeps the tab.
   */
  | { readonly type: 'open'; readonly key: ConsoleKey; readonly preview: boolean }
  /** Keep a preview tab, so the next single click no longer replaces it. */
  | { readonly type: 'keep'; readonly key: ConsoleKey }
  | { readonly type: 'close'; readonly key: ConsoleKey }
  /** A document tab was activated, so no console is showing any more. */
  | { readonly type: 'documents' }
  /** A session's `agents.timeline` landed. */
  | { readonly type: 'opened'; readonly key: ConsoleKey; readonly timeline: AgentTimeline }
  /**
   * A run's `runs.events` call came back. It carries no feed — the log went to
   * `useRuns`' own reducer — only whether there is one to draw, so a refusal
   * leaves the tab retryable instead of looking loaded and empty.
   */
  | { readonly type: 'openedRun'; readonly key: ConsoleKey; readonly loaded: boolean }
  | { readonly type: 'failed'; readonly key: ConsoleKey; readonly error: CoreError }
  /** An `AgentTimelineAppended` delta for whichever session it names. */
  | { readonly type: 'appended'; readonly payload: AgentTimelineAppendedPayload }
  /** A touched file was clicked: `documents.status` then `documents.open`. */
  | { readonly type: 'openingFile'; readonly path: string }
  | { readonly type: 'openedFile'; readonly path: string }
  | { readonly type: 'openFileFailed'; readonly path: string; readonly error: CoreError }
  /** Every console key that still has something behind it, so a stale tab goes. */
  | { readonly type: 'roster'; readonly keys: readonly ConsoleKey[] }
  | { readonly type: 'clearError' };

/** Nothing followed, nothing pending, nothing to show for it. */
function unfollowed(state: AgentTabsState): AgentTabsState {
  return { ...state, pendingId: null, loadedId: null, timeline: null, openingPath: null };
}

/**
 * Fold one `AgentTimelineAppended` delta into the timeline this client holds.
 *
 * Moved from `agent-rail-reducer.ts` unchanged — the feed's rules are T232's
 * and the console inherits them whole.
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

export function agentTabsReducer(state: AgentTabsState, action: AgentTabsAction): AgentTabsState {
  switch (action.type) {
    case 'open': {
      const { key, preview } = action;
      const existing = state.open.includes(key);
      let open = existing ? state.open : [...state.open, key];
      let previewId = state.previewId;
      if (preview) {
        // Only a *new* tab can become the preview, and a deliberate open keeps
        // the tab. Both halves matter for the double click, which fires its
        // single click first: either order of arrival lands on "kept".
        if (!existing) {
          // A look replaces a look. Closed here rather than asynchronously the
          // way `use-documents` does it, because a console has no unsaved
          // state to protect and nothing to await.
          if (previewId !== null && previewId !== key) {
            open = open.filter((id) => id !== previewId);
          }
          previewId = key;
        }
      } else if (previewId === key) {
        previewId = null;
      }
      // Activating a different console drops the previous timeline now rather
      // than when the new one lands: its follower is being closed, so keeping
      // it on screen would be a claim about a feed that has stopped.
      const switching = state.activeId !== key;
      return {
        ...state,
        open,
        previewId,
        activeId: key,
        pendingId: key,
        loadedId: switching ? null : state.loadedId,
        timeline: switching ? null : state.timeline,
        openingPath: switching ? null : state.openingPath,
        error: null,
      };
    }

    case 'keep':
      return state.previewId === action.key ? { ...state, previewId: null } : state;

    case 'close': {
      const open = state.open.filter((id) => id !== action.key);
      if (open.length === state.open.length) return state;
      const previewId = state.previewId === action.key ? null : state.previewId;
      if (state.activeId !== action.key) return { ...state, open, previewId };
      // Closing the tab you are looking at hands the area back to the
      // documents rather than guessing at another console. The follower goes
      // with it — the hook closes it, this drops what it produced.
      return { ...unfollowed(state), open, previewId, activeId: null };
    }

    case 'documents':
      // A document was activated. The console is not on screen, so its
      // follower is waste — the rail made the same call when it collapsed.
      return state.activeId === null ? state : { ...unfollowed(state), activeId: null };

    case 'opened':
      // A late answer for a tab that has already moved on is dropped;
      // `pendingId` is what says which request is still the current one.
      if (state.pendingId !== action.key) return state;
      return {
        ...state,
        pendingId: null,
        loadedId: action.key,
        timeline: action.timeline,
        error: null,
      };

    case 'openedRun':
      if (state.pendingId !== action.key) return state;
      // No timeline is set: a run's feed is the ring `useRuns` holds, and this
      // only records that the console stopped waiting for it. A call that was
      // refused leaves `loadedId` null so the next click asks again.
      return { ...state, pendingId: null, loadedId: action.loaded ? action.key : null };

    case 'failed':
      if (state.pendingId !== action.key) return state;
      // The tab stays — the user opened it and it is theirs to close — but
      // there is no feed to pretend otherwise about.
      return { ...state, pendingId: null, loadedId: null, error: action.error };

    case 'appended': {
      // A delta for a session this console is not showing: the subscription is
      // machine-wide and the core may be following something for someone else.
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
      // The refusal is the console's: nothing was opened, so no editor pane
      // exists to carry it. The feed keeps updating around it.
      return { ...state, openingPath: null, error: action.error };

    case 'roster': {
      // A session that merely *ended* is still on the roster, and a run that
      // merely finished is still listed, so either tab stays and its feed
      // simply stops growing — closing a tab under the pointer is worse. This
      // is the other case: the thing left its list entirely (a session no
      // longer discoverable, a run whose workspace closed), so there is
      // nothing left for the tab to be about.
      const open = state.open.filter((id) => action.keys.includes(id));
      if (open.length === state.open.length) return state;
      const previewId =
        state.previewId !== null && open.includes(state.previewId) ? state.previewId : null;
      if (state.activeId !== null && !open.includes(state.activeId)) {
        return { ...unfollowed(state), open, previewId, activeId: null };
      }
      return { ...state, open, previewId };
    }

    case 'clearError':
      return state.error === null ? state : { ...state, error: null };

    default:
      return state;
  }
}

/**
 * The session whose timeline the core is following, or is about to be.
 *
 * The pending request wins, which is what makes "close the previous one before
 * opening the next" correct while a request is still in flight.
 *
 * A *run* tab follows nothing — its events arrive on the machine-wide
 * `AgentRunEventAppended`, with no per-run subscription to open or close — so a
 * pending run returns `null` and the caller closes whatever was open without
 * opening anything in its place.
 */
export function followedSession(state: AgentTabsState): string | null {
  if (state.pendingId !== null) {
    const target = parseConsoleKey(state.pendingId);
    return target?.kind === 'session' ? target.sessionId : null;
  }
  return state.timeline?.sessionId ?? null;
}
