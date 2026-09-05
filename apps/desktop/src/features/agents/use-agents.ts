import { type AgentRoster, type CoreError, toCoreError } from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useState } from 'react';
import { core } from '../../core/client';
import { agentsReducer, initialAgentsState } from './agents-reducer';

export interface AgentsController {
  readonly roster: AgentRoster;
  readonly loaded: boolean;
  readonly error: CoreError | null;
  clearError(): void;
  readonly busy: boolean;
  /** Force a rescan now, rather than waiting for the core's next poll. */
  refresh(): Promise<void>;
}

/**
 * The agent sessions running on this machine (T220).
 *
 * Machine-wide and independent of the open workspace, so unlike
 * `useWorkstreams` this takes no workspace and does not reload when one
 * changes: what it lists is other people's terminals, which do not care which
 * project Gritty has open. `inWorkspace` on each session is what the UI groups
 * by, and it is recomputed by the core on every scan.
 *
 * The core polls and publishes a roster only when it actually differs, so this
 * subscribes rather than polling itself; `refresh` exists for the user who does
 * not want to wait out an interval.
 */
export function useAgents(): AgentsController {
  const [state, dispatch] = useReducer(agentsReducer, initialAgentsState);
  const [error, setError] = useState<CoreError | null>(null);
  const [inFlight, setInFlight] = useState(0);

  useEffect(() => {
    // Typed subscription rather than `subscribeAll`: this feature reads exactly
    // one event, and the narrowed envelope is already an `EventEnvelope`.
    return core.subscribe('AgentSessionsChanged', (event) => dispatch({ type: 'event', event }));
  }, []);

  useEffect(() => {
    let active = true;
    core.agents.list().then(
      (roster) => {
        if (active) dispatch({ type: 'loaded', roster });
      },
      (cause: unknown) => {
        // Discovery is a convenience: a core that cannot scan must not stop
        // the editor from working, so this only records the failure.
        if (active) setError(toCoreError(cause));
      },
    );
    return () => {
      active = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    setInFlight((count) => count + 1);
    try {
      const roster = await core.agents.refresh();
      dispatch({ type: 'loaded', roster });
      setError(null);
    } catch (cause: unknown) {
      setError(toCoreError(cause));
    } finally {
      setInFlight((count) => count - 1);
    }
  }, []);

  return {
    roster: state.roster,
    loaded: state.loaded,
    error,
    clearError: useCallback(() => setError(null), []),
    busy: inFlight > 0,
    refresh,
  };
}
