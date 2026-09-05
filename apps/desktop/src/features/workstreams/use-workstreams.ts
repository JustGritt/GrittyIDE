import {
  type CoreError,
  type Decision,
  type DecisionId,
  type DecisionOption,
  toCoreError,
  type Workspace,
  type Workstream,
  type WorkstreamId,
  type WorkstreamState,
} from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import {
  initialWorkstreamsState,
  selectDecisions,
  selectWorkstreams,
  workstreamsReducer,
} from './workstreams-reducer';

export interface WorkstreamsController {
  readonly workstreams: Workstream[];
  readonly decisions: Decision[];
  readonly loaded: boolean;
  /** The last command failure, until it is dismissed or another succeeds. */
  readonly error: CoreError | null;
  clearError(): void;
  /** True while a command is in flight; the island disables its buttons. */
  readonly busy: boolean;
  create(title: string, goal?: string): Promise<void>;
  update(workstreamId: WorkstreamId, patch: { title?: string; goal?: string }): Promise<void>;
  transition(workstreamId: WorkstreamId, to: WorkstreamState): Promise<void>;
  remove(workstreamId: WorkstreamId): Promise<void>;
  raise(workstreamId: WorkstreamId, question: string, options?: DecisionOption[]): Promise<void>;
  resolve(decisionId: DecisionId, chosen: string, rationale?: string): Promise<void>;
  dismiss(decisionId: DecisionId, rationale?: string): Promise<void>;
}

/**
 * Workstreams for the open workspace (T210).
 *
 * Every mutation goes to the core and comes back as both a response and an
 * event; the reducer takes whichever arrives first and ignores the duplicate.
 * Nothing is applied optimistically: a workstream the core refused must not
 * appear to have moved, and the refusals here are exactly the ones the user
 * needs to see (an illegal transition, an already-answered decision).
 */
export function useWorkstreams(workspace: Workspace | null): WorkstreamsController {
  const [state, dispatch] = useReducer(workstreamsReducer, initialWorkstreamsState);
  const [error, setError] = useState<CoreError | null>(null);
  const [inFlight, setInFlight] = useState(0);
  const workspaceId = workspace?.id ?? null;
  // Read inside the subscription, which is registered once.
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
    // Both lists in one pass: the queue needs decisions, and asking for them
    // separately would show a roster with no reason for its BLOCKED rows.
    Promise.all([
      core.workstreams.list({ workspaceId: workspace.id }),
      core.decisions.list({ workspaceId: workspace.id }),
    ]).then(
      ([workstreams, decisions]) => {
        if (active) dispatch({ type: 'loaded', workstreams, decisions });
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
   * One place where a command's failure becomes visible and its success
   * clears the last failure, so the island never shows a stale complaint.
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

  const workstreams = useMemo(() => selectWorkstreams(state), [state]);
  const decisions = useMemo(() => selectDecisions(state), [state]);

  const create = useCallback(
    async (title: string, goal?: string) => {
      const id = workspaceRef.current;
      if (!id) return;
      const workstream = await run(() =>
        core.workstreams.create(
          goal ? { workspaceId: id, title, goal } : { workspaceId: id, title },
        ),
      );
      if (workstream) dispatch({ type: 'workstream', workstream });
    },
    [run],
  );

  const update = useCallback(
    async (workstreamId: WorkstreamId, patch: { title?: string; goal?: string }) => {
      const id = workspaceRef.current;
      if (!id) return;
      const workstream = await run(() =>
        core.workstreams.update({ workspaceId: id, workstreamId, ...patch }),
      );
      if (workstream) dispatch({ type: 'workstream', workstream });
    },
    [run],
  );

  const transition = useCallback(
    async (workstreamId: WorkstreamId, to: WorkstreamState) => {
      const id = workspaceRef.current;
      if (!id) return;
      const workstream = await run(() =>
        core.workstreams.transition({ workspaceId: id, workstreamId, to }),
      );
      if (workstream) dispatch({ type: 'workstream', workstream });
    },
    [run],
  );

  const remove = useCallback(
    async (workstreamId: WorkstreamId) => {
      const id = workspaceRef.current;
      if (!id) return;
      const done = await run(async () => {
        await core.workstreams.remove({ workspaceId: id, workstreamId });
        return true;
      });
      if (done) dispatch({ type: 'removed', workstreamId });
    },
    [run],
  );

  const raise = useCallback(
    async (workstreamId: WorkstreamId, question: string, options: DecisionOption[] = []) => {
      const id = workspaceRef.current;
      if (!id) return;
      const decision = await run(() =>
        core.decisions.raise({ workspaceId: id, workstreamId, question, options }),
      );
      if (decision) dispatch({ type: 'decision', decision });
    },
    [run],
  );

  const resolve = useCallback(
    async (decisionId: DecisionId, chosen: string, rationale?: string) => {
      const id = workspaceRef.current;
      if (!id) return;
      const decision = await run(() =>
        core.decisions.resolve(
          rationale
            ? { workspaceId: id, decisionId, chosen, rationale }
            : { workspaceId: id, decisionId, chosen },
        ),
      );
      if (decision) dispatch({ type: 'decision', decision });
    },
    [run],
  );

  const dismiss = useCallback(
    async (decisionId: DecisionId, rationale?: string) => {
      const id = workspaceRef.current;
      if (!id) return;
      const decision = await run(() =>
        core.decisions.dismiss(
          rationale ? { workspaceId: id, decisionId, rationale } : { workspaceId: id, decisionId },
        ),
      );
      if (decision) dispatch({ type: 'decision', decision });
    },
    [run],
  );

  return {
    workstreams,
    decisions,
    loaded: state.loaded,
    error,
    clearError: useCallback(() => setError(null), []),
    busy: inFlight > 0,
    create,
    update,
    transition,
    remove,
    raise,
    resolve,
    dismiss,
  };
}
