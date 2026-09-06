import type {
  EventEnvelope,
  LeaseId,
  OwnershipLease,
  OwnershipMap,
  Workspace,
  WorkspaceId,
  WorkstreamId,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  EMPTY_MAP,
  initialOwnershipState,
  type OwnershipState,
  ownershipReducer,
} from './ownership-reducer';

function workspace(id: string): Workspace {
  return {
    id: id as WorkspaceId,
    root: `/repo/${id}`,
    name: id,
    openedAt: 1,
  } as Workspace;
}

function lease(id: string): OwnershipLease {
  return {
    id: id as LeaseId,
    workspaceId: 'w-1' as WorkspaceId,
    workstreamId: 'a' as WorkstreamId,
    target: { type: 'DIRECTORY', path: 'src/auth' },
    mode: 'WRITE',
    acquiredAt: 1,
    state: 'ACTIVE',
    stale: false,
  };
}

function map(...ids: string[]): OwnershipMap {
  return { leases: ids.map(lease), conflicts: [] };
}

function changed(payload: OwnershipMap, workspaceId = 'w-1'): EventEnvelope {
  return {
    name: 'OwnershipChanged',
    payload,
    seq: 1,
    workspaceId: workspaceId as WorkspaceId,
    actor: { type: 'USER' },
  } as EventEnvelope;
}

function opened(id: string): OwnershipState {
  return ownershipReducer(initialOwnershipState, { type: 'workspace', workspace: workspace(id) });
}

describe('ownership reducer', () => {
  it('starts empty and unloaded', () => {
    expect(initialOwnershipState.map).toEqual(EMPTY_MAP);
    expect(initialOwnershipState.loaded).toBe(false);
  });

  it('replaces the whole map rather than merging into it', () => {
    let state = opened('w-1');
    state = ownershipReducer(state, { type: 'loaded', map: map('l-1', 'l-2') });
    expect(state.loaded).toBe(true);
    // Two leases became one: a merge would have kept the released one on screen.
    state = ownershipReducer(state, { type: 'event', event: changed(map('l-2')) });
    expect(state.map.leases.map((entry) => entry.id)).toEqual(['l-2']);
  });

  it('an event alone is enough to be loaded', () => {
    // A map that arrives before the first list still beats an empty panel.
    const state = ownershipReducer(opened('w-1'), {
      type: 'event',
      event: changed(map('l-1')),
    });
    expect(state.loaded).toBe(true);
  });

  it('drops a map belonging to another workspace', () => {
    const state = ownershipReducer(opened('w-1'), {
      type: 'event',
      event: changed(map('elsewhere'), 'w-2'),
    });
    expect(state.map.leases).toEqual([]);
  });

  it('forgets the previous workspace on a switch, in both directions', () => {
    let state = ownershipReducer(opened('w-1'), { type: 'loaded', map: map('l-1') });
    state = ownershipReducer(state, { type: 'workspace', workspace: workspace('w-2') });
    expect(state.map).toEqual(EMPTY_MAP);
    expect(state.workspaceId).toBe('w-2');
    expect(state.loaded).toBe(false);

    state = ownershipReducer(state, { type: 'loaded', map: map('l-9') });
    state = ownershipReducer(state, {
      type: 'event',
      event: {
        name: 'WorkspaceClosed',
        payload: {},
        seq: 2,
        actor: { type: 'USER' },
      } as unknown as EventEnvelope,
    });
    expect(state.map).toEqual(EMPTY_MAP);
    expect(state.workspaceId).toBeNull();
  });

  it('reopening the same workspace keeps what is on screen', () => {
    const state = ownershipReducer(opened('w-1'), { type: 'loaded', map: map('l-1') });
    const again = ownershipReducer(state, { type: 'workspace', workspace: workspace('w-1') });
    expect(again).toBe(state);
  });

  it('ignores events it has no opinion about', () => {
    const state = ownershipReducer(opened('w-1'), { type: 'loaded', map: map('l-1') });
    const after = ownershipReducer(state, {
      type: 'event',
      event: {
        name: 'AgentSessionsChanged',
        payload: {},
        seq: 3,
        actor: { type: 'USER' },
      } as unknown as EventEnvelope,
    });
    expect(after).toBe(state);
  });
});
