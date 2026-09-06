import {
  type AgentProvider,
  type ConflictId,
  type ConflictResolution,
  type CoreError,
  type Decision,
  type DecisionId,
  type DecisionOption,
  type LeaseId,
  type OwnershipAcquisition,
  type OwnershipCheck,
  type OwnershipMap,
  type OwnershipTarget,
  toCoreError,
  type Workspace,
  type Workstream,
  type WorkstreamAgentLink,
  type WorkstreamId,
  type WorkstreamState,
} from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import { initialOwnershipState, ownershipReducer } from './ownership-reducer';
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
  /**
   * Ask a question against a piece of work. Resolves with the decision the core
   * created, or `undefined` when it refused — the same shape as `acquire`, so a
   * form can close on success and stay open with the envelope on a refusal
   * rather than inferring the outcome from `error`.
   */
  raise(
    workstreamId: WorkstreamId,
    question: string,
    options?: DecisionOption[],
  ): Promise<Decision | undefined>;
  resolve(decisionId: DecisionId, chosen: string, rationale?: string): Promise<void>;
  dismiss(decisionId: DecisionId, rationale?: string): Promise<void>;
  /** Who owns what in this workspace (T325). Replaced whole on every change. */
  readonly ownership: OwnershipMap;
  readonly ownershipLoaded: boolean;
  /**
   * Ask for write ownership. Resolves with the core's answer — `GRANTED` or
   * `CONFLICTED` — rather than throwing on a refusal: a conflict is a value
   * the queue draws, not an error the user has to dismiss.
   */
  acquire(
    workstreamId: WorkstreamId,
    target: OwnershipTarget,
    note?: string,
  ): Promise<OwnershipAcquisition | undefined>;
  release(leaseId: LeaseId): Promise<void>;
  checkOwnership(target: OwnershipTarget): Promise<OwnershipCheck | undefined>;
  resolveConflict(conflictId: ConflictId, action: ConflictResolution): Promise<void>;
  /**
   * Which agent sessions are attached to which workstream (T223).
   *
   * Held here rather than beside the roster because the island and the queue
   * both draw it, and T210's rule is that the two surfaces read one controller.
   * It is the *stored* link and nothing else: joining it against the live
   * roster is `agent-links.ts`'s job, so a session that has gone renders as
   * "not running" instead of vanishing.
   */
  readonly agentLinks: WorkstreamAgentLink[];
  readonly agentLinksLoaded: boolean;
  linkAgent(workstreamId: WorkstreamId, provider: AgentProvider, sessionId: string): Promise<void>;
  unlinkAgent(
    workstreamId: WorkstreamId,
    provider: AgentProvider,
    sessionId: string,
  ): Promise<void>;
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
  const [ownership, dispatchOwnership] = useReducer(ownershipReducer, initialOwnershipState);
  // A plain list rather than a reducer: nothing emits an event for a link, so
  // there is nothing to reconcile — the command that changed it returns the
  // fresh set, and this replaces that workstream's slice with it (T223 §14).
  const [links, setLinks] = useState<WorkstreamAgentLink[]>([]);
  const [linksLoaded, setLinksLoaded] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  const [inFlight, setInFlight] = useState(0);
  const workspaceId = workspace?.id ?? null;
  // Read inside the subscription, which is registered once.
  const workspaceRef = useRef(workspaceId);
  workspaceRef.current = workspaceId;

  useEffect(() => {
    // One subscription for both reducers, so the roster and the ownership map
    // can never be a beat apart.
    const unsubscribe = core.subscribeAll((event) => {
      dispatch({ type: 'event', event });
      dispatchOwnership({ type: 'event', event });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    dispatch({ type: 'workspace', workspace });
    dispatchOwnership({ type: 'workspace', workspace });
    setLinks([]);
    setLinksLoaded(false);
    if (!workspace) return;
    let active = true;
    // All four in one pass: the queue needs decisions and conflicts, and
    // asking for them separately would show a roster with no reason for its
    // BLOCKED rows.
    Promise.all([
      core.workstreams.list({ workspaceId: workspace.id }),
      core.decisions.list({ workspaceId: workspace.id }),
      core.ownership.list({ workspaceId: workspace.id }),
      core.workstreams.agentLinks({ workspaceId: workspace.id }),
    ]).then(
      ([workstreams, decisions, map, agentLinks]) => {
        if (!active) return;
        dispatch({ type: 'loaded', workstreams, decisions });
        dispatchOwnership({ type: 'loaded', map });
        setLinks(agentLinks);
        setLinksLoaded(true);
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
      if (done) {
        dispatch({ type: 'removed', workstreamId });
        // The database cascaded its links away; this is the renderer catching
        // up, not a second source of truth.
        setLinks((current) => current.filter((link) => link.workstreamId !== workstreamId));
      }
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
      return decision;
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

  const acquire = useCallback(
    async (workstreamId: WorkstreamId, target: OwnershipTarget, note?: string) => {
      const id = workspaceRef.current;
      if (!id) return undefined;
      return run(() =>
        core.ownership.acquire(
          note
            ? { workspaceId: id, workstreamId, target, note }
            : { workspaceId: id, workstreamId, target },
        ),
      );
    },
    [run],
  );

  const releaseLease = useCallback(
    async (leaseId: LeaseId) => {
      const id = workspaceRef.current;
      if (!id) return;
      await run(() => core.ownership.release({ workspaceId: id, leaseId }));
    },
    [run],
  );

  const checkOwnership = useCallback(
    async (target: OwnershipTarget) => {
      const id = workspaceRef.current;
      if (!id) return undefined;
      return run(() => core.ownership.check({ workspaceId: id, target }));
    },
    [run],
  );

  /** Replace one workstream's slice with what the core just read back. */
  const applyLinks = useCallback(
    (workstreamId: WorkstreamId, fresh: readonly WorkstreamAgentLink[]) => {
      setLinks((current) => [
        ...current.filter((link) => link.workstreamId !== workstreamId),
        ...fresh,
      ]);
    },
    [],
  );

  const linkAgent = useCallback(
    async (workstreamId: WorkstreamId, provider: AgentProvider, sessionId: string) => {
      const id = workspaceRef.current;
      if (!id) return;
      const change = await run(() =>
        core.workstreams.linkAgent({ workspaceId: id, workstreamId, provider, sessionId }),
      );
      // Nothing optimistic (T210's rule): the core's own snapshot is what
      // lands, and a duplicate simply returns the set that was already there.
      if (change) applyLinks(workstreamId, change.links);
    },
    [run, applyLinks],
  );

  const unlinkAgent = useCallback(
    async (workstreamId: WorkstreamId, provider: AgentProvider, sessionId: string) => {
      const id = workspaceRef.current;
      if (!id) return;
      const change = await run(() =>
        core.workstreams.unlinkAgent({ workspaceId: id, workstreamId, provider, sessionId }),
      );
      if (change) applyLinks(workstreamId, change.links);
    },
    [run, applyLinks],
  );

  const resolveConflict = useCallback(
    async (conflictId: ConflictId, action: ConflictResolution) => {
      const id = workspaceRef.current;
      if (!id) return;
      await run(() => core.ownership.resolve({ workspaceId: id, conflictId, action }));
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
    // The map arrives whole on every change, so there is nothing to merge and
    // nothing that can disagree with the event that produced it.
    ownership: ownership.map,
    ownershipLoaded: ownership.loaded,
    acquire,
    release: releaseLease,
    checkOwnership,
    resolveConflict,
    agentLinks: links,
    agentLinksLoaded: linksLoaded,
    linkAgent,
    unlinkAgent,
  };
}
