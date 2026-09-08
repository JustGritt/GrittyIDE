import {
  type AgentTimeline,
  type CoreError,
  toCoreError,
  type WorkspaceId,
  type WorkspacePath,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef } from 'react';
import { core } from '../../core/client';
import {
  type AgentTabsState,
  agentTabsReducer,
  followedSession,
  initialAgentTabsState,
} from './agent-tabs-reducer';
import { openTouchedFile, type TouchedFileDocuments } from './open-touched-file';
import type { AgentsController } from './use-agents';

export interface AgentConsoleOptions {
  /** The one roster the section, the island and the console share. */
  readonly agents: AgentsController;
  /**
   * How a file the agent touched reaches the editor (T232). The document store,
   * narrowed to the two calls the console is allowed to make.
   */
  readonly documents: TouchedFileDocuments;
  /**
   * The open workspace, which is what a `FileTouched` path was resolved
   * against. `null` with no workspace open, and then nothing is openable — the
   * core sends no paths in that case either.
   */
  readonly workspaceId: WorkspaceId | null;
}

export interface AgentConsoleController {
  /** Session ids with a tab, in the order they were opened. */
  readonly openTabs: readonly string[];
  readonly previewId: string | null;
  /** The console showing now, or `null` when a document is showing. */
  readonly activeId: string | null;
  readonly pendingId: string | null;
  readonly timeline: AgentTimeline | null;
  /** The touched file being opened right now, so its row can say so. */
  readonly openingPath: string | null;
  /** False with no workspace open: there is nothing for a path to be inside. */
  readonly canOpenFiles: boolean;
  readonly error: CoreError | null;
  clearError(): void;
  /**
   * Open a console and show it, or show one already open. A single click, in
   * both senses: from the sidebar it opens a preview tab, and from the tab
   * strip it merely activates one — the same call, because a tab that already
   * exists is not made a preview by being clicked.
   */
  show(sessionId: string): void;
  /** Keep a preview tab, so the next single click no longer replaces it. */
  keep(sessionId: string): void;
  close(sessionId: string): void;
  /** A document tab was activated; the console stops being on screen. */
  showDocuments(): void;
  /** Open a file the agent touched, in the editor it is sitting in. */
  openFile(path: WorkspacePath): void;
}

/**
 * The agent consoles open as tabs, and the one follower they are allowed to run
 * between them (T233-C).
 *
 * `use-agent-rail.ts`, moved. Every guarantee it made is kept, because they are
 * the ones that stop a follower being orphaned rather than anything to do with
 * the rail being a column:
 *
 * - **Exactly one timeline is open at a time.** Switching consoles closes the
 *   previous before opening the next; so does closing the tab, and so does
 *   activating a document.
 * - **Nothing about the feed is optimistic.** The tab activates on the click,
 *   because a tab is a control and has to answer; the timeline appears when
 *   `agents.openTimeline` returns, and a refusal surfaces as the envelope.
 * - **Late answers are dropped**, by `pendingId`, in the reducer.
 * - **The asynchronous paths read `latest.current`**, never the state they
 *   closed over, or a rapid switch would act on a roster that has moved.
 */
export function useAgentConsole({
  agents,
  documents,
  workspaceId,
}: AgentConsoleOptions): AgentConsoleController {
  const [state, dispatch] = useReducer(agentTabsReducer, initialAgentTabsState);

  // Read by the asynchronous paths, which must act on the state as it is when
  // they run rather than as it was when they were created.
  const latest = useRef<AgentTabsState>(state);
  latest.current = state;

  // The timeline arrives as deltas, never as a snapshot: a timeline grows
  // without bound, so the core appends rather than resending (rule 8). The
  // subscription is bound once and the reducer decides whether a batch is for
  // the console on screen — one listener, whatever is showing under it.
  useEffect(() => {
    return core.subscribe('AgentTimelineAppended', (event) =>
      dispatch({ type: 'appended', payload: event.payload }),
    );
  }, []);

  /**
   * Stop following whatever is open. `closeTimeline` is idempotent and its
   * failure has no consequence the user can act on — the follower is bounded
   * in the core either way — so it is deliberately not raised as an error the
   * way a failed `openTimeline` is.
   */
  const stopFollowing = useCallback((sessionId: string | null) => {
    if (!sessionId) return;
    void core.agents.closeTimeline({ sessionId }).catch(() => {});
  }, []);

  const show = useCallback(
    (sessionId: string) => {
      const current = latest.current;
      // Already the console on screen, with a feed or one on the way. Clicking
      // its tab again is not a request to restart the follower.
      if (
        current.activeId === sessionId &&
        (current.timeline !== null || current.pendingId === sessionId)
      ) {
        return;
      }
      stopFollowing(followedSession(current));
      dispatch({ type: 'open', sessionId, preview: true });
      core.agents.openTimeline({ sessionId }).then(
        (timeline) => dispatch({ type: 'opened', sessionId, timeline }),
        // `NotFound` is the ordinary case here: a session can leave the roster
        // between the click and this call.
        (cause: unknown) => dispatch({ type: 'failed', sessionId, error: toCoreError(cause) }),
      );
    },
    [stopFollowing],
  );

  const keep = useCallback((sessionId: string) => {
    dispatch({ type: 'keep', sessionId });
  }, []);

  const close = useCallback(
    (sessionId: string) => {
      if (followedSession(latest.current) === sessionId) stopFollowing(sessionId);
      dispatch({ type: 'close', sessionId });
    },
    [stopFollowing],
  );

  const showDocuments = useCallback(() => {
    const current = latest.current;
    if (current.activeId === null) return;
    stopFollowing(followedSession(current));
    dispatch({ type: 'documents' });
  }, [stopFollowing]);

  // A tab whose session left the roster has nothing left to be about. A
  // session that merely *ended* is still listed, so its tab stays: the feed
  // stops growing, which is what happened, and the tab does not vanish under
  // the pointer.
  const { roster } = agents;
  useEffect(() => {
    const ids = roster.sessions.map((session) => session.id);
    const following = followedSession(latest.current);
    if (following && !ids.includes(following)) stopFollowing(following);
    dispatch({ type: 'roster', ids });
  }, [roster, stopFollowing]);

  // The shell going away takes the follower with it, for the same reason
  // closing the tab does.
  useEffect(() => {
    return () => {
      const following = followedSession(latest.current);
      if (following) void core.agents.closeTimeline({ sessionId: following }).catch(() => {});
    };
  }, []);

  /**
   * The point of the whole surface (T232 §9-§12): the file the agent just
   * changed, open in the editor the console is sitting in.
   *
   * Only ever called with a `WorkspacePath` the core put on a `FileTouched`
   * entry, which it does only for files inside the open workspace. A file
   * outside it carries no path at all, so there is nothing for this to be
   * called with (ADR-010) — the protocol enforces the boundary, not the
   * renderer's restraint.
   */
  const openFile = useCallback(
    (path: WorkspacePath) => {
      if (workspaceId === null) return;
      // A second click while the first open is in flight would race two
      // `documents.status` calls for no gain.
      if (latest.current.openingPath !== null) return;
      dispatch({ type: 'openingFile', path });
      openTouchedFile(documents, workspaceId, path).then(
        (result) => {
          if (result.ok) dispatch({ type: 'openedFile', path });
          // `NotFound` is the ordinary failure: the list is history, and the
          // agent may have deleted the file since it wrote it.
          else dispatch({ type: 'openFileFailed', path, error: result.error });
        },
        (cause: unknown) => dispatch({ type: 'openFileFailed', path, error: toCoreError(cause) }),
      );
    },
    [documents, workspaceId],
  );

  return {
    openTabs: state.open,
    previewId: state.previewId,
    activeId: state.activeId,
    pendingId: state.pendingId,
    timeline: state.timeline,
    openingPath: state.openingPath,
    canOpenFiles: workspaceId !== null,
    error: state.error,
    clearError: useCallback(() => dispatch({ type: 'clearError' }), []),
    show,
    keep,
    close,
    showDocuments,
    openFile,
  };
}
