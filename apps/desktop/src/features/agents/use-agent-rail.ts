import { type AgentTimeline, type CoreError, toCoreError } from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef } from 'react';
import { core } from '../../core/client';
import {
  type AgentRailState,
  agentRailReducer,
  followedSession,
  initialAgentRailState,
} from './agent-rail-reducer';
import type { AgentsController } from './use-agents';

export interface AgentRailOptions {
  /** The one roster the rail and the island share (T231 §7). */
  readonly agents: AgentsController;
  /** Controlled by the session layout blob (T060); the local state follows it. */
  readonly open: boolean;
  onOpenChange(open: boolean): void;
}

export interface AgentRailController {
  readonly open: boolean;
  readonly selectedId: string | null;
  readonly pendingId: string | null;
  readonly timeline: AgentTimeline | null;
  readonly error: CoreError | null;
  clearError(): void;
  setOpen(open: boolean): void;
  toggle(): void;
  /** Follow this session, or stop following it if it is the selected one. */
  select(sessionId: string): void;
}

/**
 * The rail's own state, and the one follower it is allowed to run (T231).
 *
 * Exactly one timeline is open at a time: switching rows closes the previous
 * one before opening the next, and collapsing the rail closes whatever was
 * open. Nothing is optimistic — a row becomes selected when the core says the
 * follower started, and a refusal surfaces as the error envelope.
 */
export function useAgentRail({
  agents,
  open,
  onOpenChange,
}: AgentRailOptions): AgentRailController {
  const [state, dispatch] = useReducer(agentRailReducer, initialAgentRailState);

  // The blob is restored after the first render (T060), so the persisted value
  // is mirrored in rather than seeded from — the same bridge `useExplorer`
  // uses for `showHidden`. The toggle writes outward through `onOpenChange`,
  // so this only ever re-applies a value the reducer already holds.
  useEffect(() => {
    dispatch({ type: 'setOpen', open });
  }, [open]);

  // Read by the asynchronous paths, which must act on the state as it is when
  // they run rather than as it was when they were created.
  const latest = useRef<AgentRailState>(state);
  latest.current = state;

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

  const select = useCallback(
    (sessionId: string) => {
      const current = latest.current;
      const following = followedSession(current);
      if (following === sessionId && current.selectedId === sessionId) {
        // Clicking the open row closes it: one fewer follower, and the
        // placeholder body stops claiming to be showing something.
        dispatch({ type: 'deselect' });
        stopFollowing(sessionId);
        return;
      }
      stopFollowing(following);
      dispatch({ type: 'selecting', sessionId });
      core.agents.openTimeline({ sessionId }).then(
        (timeline) => dispatch({ type: 'opened', sessionId, timeline }),
        // `NotFound` is the ordinary case here: a session can leave the roster
        // between the click and this call.
        (cause: unknown) => dispatch({ type: 'failed', sessionId, error: toCoreError(cause) }),
      );
    },
    [stopFollowing],
  );

  const setOpen = useCallback(
    (next: boolean) => {
      if (!next) stopFollowing(followedSession(latest.current));
      dispatch({ type: 'setOpen', open: next });
      onOpenChange(next);
    },
    [onOpenChange, stopFollowing],
  );

  // A selection that left the roster has no row left to belong to. A session
  // that merely *ended* is still listed, so it stays selected: its timeline
  // stops growing, which is what happened, and the row does not vanish under
  // the pointer.
  const { roster } = agents;
  useEffect(() => {
    const ids = roster.sessions.map((session) => session.id);
    const following = followedSession(latest.current);
    if (following && !ids.includes(following)) stopFollowing(following);
    dispatch({ type: 'roster', ids });
  }, [roster, stopFollowing]);

  // The rail going away takes its follower with it, for the same reason
  // collapsing does.
  useEffect(() => {
    return () => {
      const following = followedSession(latest.current);
      if (following) void core.agents.closeTimeline({ sessionId: following }).catch(() => {});
    };
  }, []);

  return {
    open: state.open,
    selectedId: state.selectedId,
    pendingId: state.pendingId,
    timeline: state.timeline,
    error: state.error,
    clearError: useCallback(() => dispatch({ type: 'clearError' }), []),
    setOpen,
    toggle: useCallback(() => setOpen(!latest.current.open), [setOpen]),
    select,
  };
}
