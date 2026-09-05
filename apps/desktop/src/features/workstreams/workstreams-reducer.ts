import type {
  Decision,
  DecisionId,
  EventEnvelope,
  Workspace,
  WorkspaceId,
  Workstream,
  WorkstreamId,
} from '@gritty/protocol';

/**
 * Workstreams and their decisions as the UI knows them (T210).
 *
 * The core is authoritative: every command returns the entity it changed and
 * emits an event carrying the same thing, so this reducer only ever *puts*
 * what it is given. It never derives a state transition itself — that is the
 * state machine's job, in Rust, and a second copy here would drift.
 *
 * Pure, so it is unit-tested without React or a database.
 */
export interface WorkstreamsState {
  readonly byId: Readonly<Record<string, Workstream>>;
  /** Newest first, the order `workstreams.list` returns. */
  readonly order: readonly WorkstreamId[];
  readonly decisionsById: Readonly<Record<string, Decision>>;
  /** Oldest first, the order `decisions.list` returns. */
  readonly decisionOrder: readonly DecisionId[];
  readonly workspaceId: WorkspaceId | null;
  /** Whether the first load for this workspace has landed. */
  readonly loaded: boolean;
}

export const initialWorkstreamsState: WorkstreamsState = {
  byId: {},
  order: [],
  decisionsById: {},
  decisionOrder: [],
  workspaceId: null,
  loaded: false,
};

export type WorkstreamsAction =
  | { type: 'workspace'; workspace: Workspace | null }
  | { type: 'loaded'; workstreams: Workstream[]; decisions: Decision[] }
  | { type: 'workstream'; workstream: Workstream }
  | { type: 'removed'; workstreamId: WorkstreamId }
  | { type: 'decision'; decision: Decision }
  | { type: 'event'; event: EventEnvelope };

function putWorkstream(state: WorkstreamsState, workstream: Workstream): WorkstreamsState {
  // A workstream from another workspace would be a bug, but a stray event
  // arriving during a workspace switch is not: drop it rather than show work
  // that belongs somewhere else.
  if (state.workspaceId !== null && workstream.workspaceId !== state.workspaceId) return state;
  const known = state.byId[workstream.id] !== undefined;
  return {
    ...state,
    byId: { ...state.byId, [workstream.id]: workstream },
    // Newest first, matching the core's ordering, so a new one appears on top.
    order: known ? state.order : [workstream.id, ...state.order],
  };
}

function putDecision(state: WorkstreamsState, decision: Decision): WorkstreamsState {
  if (state.byId[decision.workstreamId] === undefined) return state;
  const known = state.decisionsById[decision.id] !== undefined;
  return {
    ...state,
    decisionsById: { ...state.decisionsById, [decision.id]: decision },
    decisionOrder: known ? state.decisionOrder : [...state.decisionOrder, decision.id],
  };
}

function removeWorkstream(state: WorkstreamsState, workstreamId: WorkstreamId): WorkstreamsState {
  if (state.byId[workstreamId] === undefined) return state;
  const { [workstreamId]: _gone, ...byId } = state.byId;
  // Its decisions went with it in the database (cascade); they must go here
  // too, or the queue would hold questions nothing can answer.
  const decisionOrder = state.decisionOrder.filter(
    (id) => state.decisionsById[id]?.workstreamId !== workstreamId,
  );
  const decisionsById = Object.fromEntries(
    decisionOrder.map((id) => [id, state.decisionsById[id]]).filter(([, value]) => value),
  ) as Record<string, Decision>;
  return {
    ...state,
    byId,
    order: state.order.filter((id) => id !== workstreamId),
    decisionsById,
    decisionOrder,
  };
}

export function workstreamsReducer(
  state: WorkstreamsState,
  action: WorkstreamsAction,
): WorkstreamsState {
  switch (action.type) {
    case 'workspace': {
      const workspaceId = action.workspace?.id ?? null;
      if (workspaceId === state.workspaceId) return state;
      // Workstreams are per workspace and durable: the previous workspace's
      // are not shown, and not lost either — they are in the database.
      return { ...initialWorkstreamsState, workspaceId };
    }
    case 'loaded': {
      // A whole-list replacement, in the order the core gave: this is the one
      // place the UI is allowed to forget what it had.
      const mine = action.workstreams.filter(
        (workstream) => state.workspaceId === null || workstream.workspaceId === state.workspaceId,
      );
      const loaded: WorkstreamsState = {
        ...state,
        byId: Object.fromEntries(mine.map((workstream) => [workstream.id, workstream])),
        order: mine.map((workstream) => workstream.id),
        decisionsById: {},
        decisionOrder: [],
        loaded: true,
      };
      return action.decisions.reduce(putDecision, loaded);
    }
    case 'workstream':
      return putWorkstream(state, action.workstream);
    case 'removed':
      return removeWorkstream(state, action.workstreamId);
    case 'decision':
      return putDecision(state, action.decision);
    case 'event': {
      const { event } = action;
      switch (event.name) {
        case 'WorkstreamCreated':
        case 'WorkstreamChanged':
          return putWorkstream(state, event.payload.workstream);
        case 'WorkstreamRemoved':
          return removeWorkstream(state, event.payload.workstreamId);
        case 'DecisionRequested':
        case 'DecisionResolved':
          return putDecision(state, event.payload.decision);
        case 'WorkspaceOpened':
          return event.payload.workspace.id === state.workspaceId
            ? state
            : { ...initialWorkstreamsState, workspaceId: event.payload.workspace.id };
        case 'WorkspaceClosed':
          return { ...initialWorkstreamsState };
      }
      return state;
    }
  }
}

/** Newest first, as the core lists them. */
export function selectWorkstreams(state: WorkstreamsState): Workstream[] {
  return state.order.map((id) => state.byId[id]).filter((item) => item !== undefined);
}

/** Oldest first, as the core lists them. */
export function selectDecisions(state: WorkstreamsState): Decision[] {
  return state.decisionOrder
    .map((id) => state.decisionsById[id])
    .filter((item) => item !== undefined);
}

export function selectOpenDecisions(state: WorkstreamsState): Decision[] {
  return selectDecisions(state).filter((decision) => decision.state === 'OPEN');
}
