import type { EventEnvelope, OwnershipMap, Workspace, WorkspaceId } from '@gritty/protocol';

/**
 * The ownership map as the UI knows it (T325).
 *
 * The core publishes the **whole** map on every change, so this reducer only
 * ever replaces. That is not laziness: the map is small and bounded, and a
 * consumer that missed an event converges on the next one instead of
 * accumulating a divergent copy. Compare `AgentTimelineAppended`, which is a
 * delta precisely because a transcript is not bounded.
 *
 * Pure, so it is unit-tested without React or a database.
 */
export interface OwnershipState {
  readonly map: OwnershipMap;
  readonly workspaceId: WorkspaceId | null;
  /** Whether the first load for this workspace has landed. */
  readonly loaded: boolean;
}

export const EMPTY_MAP: OwnershipMap = { leases: [], conflicts: [] };

export const initialOwnershipState: OwnershipState = {
  map: EMPTY_MAP,
  workspaceId: null,
  loaded: false,
};

export type OwnershipAction =
  | { type: 'workspace'; workspace: Workspace | null }
  | { type: 'loaded'; map: OwnershipMap }
  | { type: 'event'; event: EventEnvelope };

export function ownershipReducer(state: OwnershipState, action: OwnershipAction): OwnershipState {
  switch (action.type) {
    case 'workspace': {
      const workspaceId = action.workspace?.id ?? null;
      if (workspaceId === state.workspaceId) return state;
      // Leases are per workspace and durable: the previous workspace's are not
      // shown, and not lost either — they are in the database.
      return { ...initialOwnershipState, workspaceId };
    }
    case 'loaded':
      return { ...state, map: action.map, loaded: true };
    case 'event': {
      const { event } = action;
      switch (event.name) {
        case 'OwnershipChanged':
          // A map from another workspace would be a bug, but a stray event
          // arriving during a workspace switch is not: drop it rather than
          // show ownership that belongs somewhere else.
          if (state.workspaceId !== null && event.workspaceId !== state.workspaceId) return state;
          return { ...state, map: event.payload, loaded: true };
        case 'WorkspaceOpened':
          return event.payload.workspace.id === state.workspaceId
            ? state
            : { ...initialOwnershipState, workspaceId: event.payload.workspace.id };
        case 'WorkspaceClosed':
          return { ...initialOwnershipState };
      }
      return state;
    }
  }
}
