import type { EventEnvelope, Operation, OperationStatus } from '@gritty/protocol';

/**
 * Operations as the UI knows them: rehydrated from `operations.list()` and
 * kept current by `Operation*` events. Pure, so it is unit-tested without React.
 */
export interface OperationsState {
  readonly byId: Readonly<Record<string, Operation>>;
}

export const initialOperationsState: OperationsState = { byId: {} };

export type OperationsAction =
  | { type: 'rehydrate'; operations: Operation[] }
  | { type: 'snapshot'; operation: Operation }
  | { type: 'event'; event: EventEnvelope };

/** Later lifecycle stages win; a stale `CREATED` snapshot never overwrites `RUNNING`. */
const RANK: Record<OperationStatus, number> = {
  CREATED: 0,
  RUNNING: 1,
  COMPLETED: 2,
  FAILED: 2,
  CANCELLED: 2,
  INTERRUPTED: 2,
};

export function isTerminal(status: OperationStatus): boolean {
  return RANK[status] === 2;
}

function merge(existing: Operation | undefined, incoming: Operation): Operation {
  if (!existing) return incoming;
  if (RANK[existing.status] > RANK[incoming.status]) return existing;
  if (incoming.progress === undefined && existing.progress !== undefined) {
    return { ...incoming, progress: existing.progress };
  }
  return incoming;
}

function upsert(state: OperationsState, operation: Operation): OperationsState {
  const merged = merge(state.byId[operation.id], operation);
  if (merged === state.byId[operation.id]) return state;
  return { byId: { ...state.byId, [operation.id]: merged } };
}

export function operationsReducer(
  state: OperationsState,
  action: OperationsAction,
): OperationsState {
  switch (action.type) {
    case 'rehydrate':
      return action.operations.reduce(upsert, state);
    case 'snapshot':
      return upsert(state, action.operation);
    case 'event': {
      const { event } = action;
      switch (event.name) {
        case 'OperationStarted':
        case 'OperationCompleted':
        case 'OperationFailed':
        case 'OperationCancelled':
          return upsert(state, event.payload.operation);
        case 'OperationProgress': {
          const existing = state.byId[event.payload.operationId];
          if (!existing || isTerminal(existing.status)) return state;
          return {
            byId: {
              ...state.byId,
              [existing.id]: { ...existing, progress: event.payload.progress },
            },
          };
        }
      }
    }
  }
  return state;
}

/** Newest first. */
export function selectOperations(state: OperationsState): Operation[] {
  return Object.values(state.byId).sort(
    (a, b) => b.createdAt - a.createdAt || b.id.localeCompare(a.id),
  );
}
