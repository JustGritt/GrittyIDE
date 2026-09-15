import {
  type AgentRun,
  type AgentRunId,
  type AgentRunPermissionVerdict,
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
  /**
   * Start a real Claude Code run (T310-B). Resolves with the run the core
   * created, or `undefined` when it refused, so the launcher can open the run's
   * console on success and keep its form on a refusal (RUN-LAUNCHER).
   */
  startClaude(workstreamId: WorkstreamId, prompt?: string): Promise<AgentRun | undefined>;
  /**
   * Say something to a run that is already running.
   *
   * Nothing optimistic: resolving means the text was queued, not that the
   * agent has seen it. The user's own words reach the feed when
   * `PROMPT_STARTED` arrives, which is the run confirming them.
   */
  prompt(runId: AgentRunId, text: string): Promise<void>;
  /**
   * Answer the permission ask a run is stopped at (T310-ANSWER).
   *
   * The run that comes back no longer carries the ask, so the card goes at
   * once; the run itself leaves `BLOCKED` when its agent acknowledges, which
   * arrives as an event.
   */
  answerPermission(
    runId: AgentRunId,
    askId: string,
    verdict: AgentRunPermissionVerdict,
  ): Promise<void>;
  /** The ask whose answer is in flight, so its card's buttons go quiet. */
  readonly answering: string | null;
  cancel(runId: AgentRunId): Promise<void>;
  /**
   * Fetch one run's events once, so an opened console starts from the ring.
   *
   * Resolves `true` when the log landed. The console needs the answer rather
   * than the log: a refusal must leave its tab retryable instead of looking
   * loaded and empty, and the refusal itself is already on `error`.
   */
  openEvents(runId: AgentRunId): Promise<boolean>;
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
  const [answering, setAnswering] = useState<string | null>(null);
  const workspaceId = workspace?.id ?? null;
  // Read inside callbacks that are created once.
  const workspaceRef = useRef(workspaceId);
  workspaceRef.current = workspaceId;

  useEffect(() => {
    // Two typed subscriptions rather than `subscribeAll`: `runsReducer` reads
    // exactly these two names and returns its input state for every other one
    // (`runs-reducer.ts:162-172`). Subscribing to everything therefore turned
    // each unrelated domain event in the application — terminal output,
    // watcher batches, agent timelines — into a dispatch and a render pass
    // this feature had already decided to ignore. Narrowing here keeps that
    // traffic out of React entirely, which is where ADR-032 decision 2 wants
    // it, and it costs one extra unsubscribe.
    const unsubscribeRun = core.subscribe('AgentRunChanged', (event) =>
      dispatch({ type: 'event', event }),
    );
    const unsubscribeEvents = core.subscribe('AgentRunEventAppended', (event) =>
      dispatch({ type: 'event', event }),
    );
    return () => {
      unsubscribeRun();
      unsubscribeEvents();
    };
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

  const startClaude = useCallback(
    async (workstreamId: WorkstreamId, prompt?: string) => {
      const id = workspaceRef.current;
      if (!id) return undefined;
      const started = await run(() =>
        core.runs.start({
          workspaceId: id,
          workstreamId,
          provider: 'CLAUDE_CODE',
          ...(prompt ? { prompt } : {}),
        }),
      );
      if (started) dispatch({ type: 'run', run: started });
      return started;
    },
    [run],
  );

  const prompt = useCallback(
    async (runId: AgentRunId, text: string) => {
      // The snapshot comes back unchanged — accepting a prompt records no
      // event and changes no state — so this dispatches for the refusal path,
      // where the error is what the panel needs to show.
      const answered = await run(() => core.runs.prompt({ runId, text }));
      if (answered) dispatch({ type: 'run', run: answered });
    },
    [run],
  );

  const answerPermission = useCallback(
    async (runId: AgentRunId, askId: string, verdict: AgentRunPermissionVerdict) => {
      setAnswering(askId);
      try {
        const answered = await run(() => core.runs.answerPermission({ runId, askId, verdict }));
        if (answered) dispatch({ type: 'run', run: answered });
      } finally {
        setAnswering((current) => (current === askId ? null : current));
      }
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
      if (!log) return false;
      dispatch({ type: 'log', log });
      return true;
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
    startClaude,
    prompt,
    answerPermission,
    answering,
    cancel,
    openEvents,
    logFor,
  };
}
