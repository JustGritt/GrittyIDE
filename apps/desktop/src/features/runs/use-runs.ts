import {
  type AgentRun,
  type AgentRunId,
  type CoreError,
  toCoreError,
  type Workspace,
  type WorkstreamId,
} from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import { initialRunsState, type RunLog, runsReducer, selectLog, selectRuns } from './runs-reducer';

export interface RunsController {
  /** Newest first. Terminal runs stay until the workspace changes. */
  readonly runs: AgentRun[];
  /** False until the first `runs.list` lands, so "none" and "not yet" differ. */
  readonly loaded: boolean;
  /** The last command failure, until it is dismissed or another succeeds. */
  readonly error: CoreError | null;
  clearError(): void;
  /** True while a command is in flight; the panel disables its buttons. */
  readonly busy: boolean;
  startFake(workstreamId: WorkstreamId, script?: string): Promise<void>;
  cancel(runId: AgentRunId): Promise<void>;
  /** Fetch one run's events once, so an expanded row starts from the ring. */
  openEvents(runId: AgentRunId): Promise<void>;
  logFor(runId: AgentRunId): RunLog;
}

/**
 * The runs of the open workspace (T300-CMD, §17).
 *
 * Deliberately the same shape as `useWorkstreams`, and for the same reasons:
 * one subscription registered before anything is listed, so nothing emitted in
 * between is lost; every mutation answered by both a response and an event,
 * with the reducer taking whichever arrives first; and **nothing optimistic** —
 * a run the core refused to start must not appear to have started.
 *
 * A run belongs to the workspace it was started against, and closing that
 * workspace cancels it rather than orphaning it. So the workspace changing
 * empties this rather than leaving rows for work that is already over.
 */
export function useRuns(workspace: Workspace | null): RunsController {
  const [state, dispatch] = useReducer(runsReducer, initialRunsState);
  const [error, setError] = useState<CoreError | null>(null);
  const [inFlight, setInFlight] = useState(0);
  const workspaceId = workspace?.id ?? null;
  // Read inside callbacks that are created once.
  const workspaceRef = useRef(workspaceId);
  workspaceRef.current = workspaceId;

  useEffect(() => {
    const unsubscribe = core.subscribeAll((event) => dispatch({ type: 'event', event }));
    return unsubscribe;
  }, []);

  useEffect(() => {
    dispatch({ type: 'workspace', workspace });
    if (!workspace) return;
    let active = true;
    core.runs.list({ workspaceId: workspace.id }).then(
      (runs) => {
        if (active) dispatch({ type: 'loaded', runs });
      },
      (cause: unknown) => {
        if (active) setError(toCoreError(cause));
      },
    );
    return () => {
      active = false;
    };
  }, [workspace]);

  /**
   * One place where a command's failure becomes visible and its success clears
   * the last failure, so the panel never shows a stale complaint.
   */
  const run = useCallback(async <T>(work: () => Promise<T>): Promise<T | undefined> => {
    setInFlight((count) => count + 1);
    try {
      const result = await work();
      setError(null);
      return result;
    } catch (cause: unknown) {
      setError(toCoreError(cause));
      return undefined;
    } finally {
      setInFlight((count) => count - 1);
    }
  }, []);

  const startFake = useCallback(
    async (workstreamId: WorkstreamId, script?: string) => {
      const id = workspaceRef.current;
      if (!id) return;
      const started = await run(() =>
        core.runs.startFake(
          script ? { workspaceId: id, workstreamId, script } : { workspaceId: id, workstreamId },
        ),
      );
      if (started) dispatch({ type: 'run', run: started });
    },
    [run],
  );

  const cancel = useCallback(
    async (runId: AgentRunId) => {
      // The answer may still be non-terminal with `cancellationRequested` set:
      // cancellation is cooperative, and the state follows when the run
      // notices. The row shows "cancelling" until the event says otherwise.
      const cancelled = await run(() => core.runs.cancel({ runId }));
      if (cancelled) dispatch({ type: 'run', run: cancelled });
    },
    [run],
  );

  const openEvents = useCallback(
    async (runId: AgentRunId) => {
      const log = await run(() => core.runs.events({ runId }));
      if (log) dispatch({ type: 'log', log });
    },
    [run],
  );

  const runs = useMemo(() => selectRuns(state), [state]);
  const logFor = useCallback((runId: AgentRunId) => selectLog(state, runId), [state]);

  return {
    runs,
    loaded: state.loaded,
    error,
    clearError: useCallback(() => setError(null), []),
    busy: inFlight > 0,
    startFake,
    cancel,
    openEvents,
    logFor,
  };
}
