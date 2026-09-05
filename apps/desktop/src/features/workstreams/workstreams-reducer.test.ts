import type {
  Decision,
  DecisionId,
  EventEnvelope,
  Workspace,
  WorkspaceId,
  Workstream,
  WorkstreamId,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  initialWorkstreamsState,
  selectDecisions,
  selectOpenDecisions,
  selectWorkstreams,
  type WorkstreamsState,
  workstreamsReducer,
} from './workstreams-reducer';

const WORKSPACE = 'ws-1' as WorkspaceId;
const OTHER = 'ws-2' as WorkspaceId;

function workspace(id: WorkspaceId = WORKSPACE): Workspace {
  return { id, root: '/repo', name: 'repo', openedAt: 1 };
}

function workstream(id: string, patch: Partial<Workstream> = {}): Workstream {
  return {
    id: id as WorkstreamId,
    workspaceId: WORKSPACE,
    title: id,
    state: 'DRAFT',
    allowedTransitions: ['READY', 'ABANDONED'],
    openDecisions: 0,
    createdAt: 1,
    updatedAt: 1,
    ...patch,
  };
}

function decision(id: string, workstreamId: string, patch: Partial<Decision> = {}): Decision {
  return {
    id: id as DecisionId,
    workstreamId: workstreamId as WorkstreamId,
    question: `question ${id}`,
    options: [],
    state: 'OPEN',
    raisedBy: { type: 'USER' },
    raisedAt: 5,
    ...patch,
  };
}

function event(name: string, payload: unknown): EventEnvelope {
  return { name, payload, seq: 1, actor: { type: 'USER' } } as unknown as EventEnvelope;
}

/** A workspace open with two workstreams and one open question on the first. */
function loaded(): WorkstreamsState {
  const opened = workstreamsReducer(initialWorkstreamsState, {
    type: 'workspace',
    workspace: workspace(),
  });
  return workstreamsReducer(opened, {
    type: 'loaded',
    workstreams: [workstream('b', { createdAt: 2 }), workstream('a')],
    decisions: [decision('d1', 'a')],
  });
}

describe('workstreams reducer', () => {
  it('keeps the order the core gave', () => {
    const state = loaded();
    expect(selectWorkstreams(state).map((item) => item.id)).toEqual(['b', 'a']);
    expect(state.loaded).toBe(true);
    expect(selectDecisions(state)).toHaveLength(1);
  });

  it('puts a new workstream on top and updates one in place', () => {
    const created = workstreamsReducer(loaded(), {
      type: 'workstream',
      workstream: workstream('c'),
    });
    expect(selectWorkstreams(created).map((item) => item.id)).toEqual(['c', 'b', 'a']);

    const moved = workstreamsReducer(created, {
      type: 'event',
      event: event('WorkstreamChanged', {
        workstream: workstream('a', { state: 'READY' }),
      }),
    });
    expect(selectWorkstreams(moved).map((item) => item.id)).toEqual(['c', 'b', 'a']);
    expect(moved.byId.a?.state).toBe('READY');
  });

  it('takes the command response and its event without duplicating', () => {
    const answered = workstream('a', { state: 'BLOCKED', openDecisions: 1 });
    const once = workstreamsReducer(loaded(), { type: 'workstream', workstream: answered });
    const twice = workstreamsReducer(once, {
      type: 'event',
      event: event('WorkstreamChanged', { workstream: answered }),
    });
    expect(selectWorkstreams(twice).map((item) => item.id)).toEqual(['b', 'a']);
  });

  it('removes a workstream with the decisions that belonged to it', () => {
    const withTwo = workstreamsReducer(loaded(), {
      type: 'decision',
      decision: decision('d2', 'b'),
    });
    const removed = workstreamsReducer(withTwo, {
      type: 'event',
      event: event('WorkstreamRemoved', { workstreamId: 'a' as WorkstreamId, title: 'a' }),
    });
    expect(selectWorkstreams(removed).map((item) => item.id)).toEqual(['b']);
    expect(selectDecisions(removed).map((item) => item.id)).toEqual(['d2']);
  });

  it('ignores a decision for a workstream it does not have', () => {
    const state = workstreamsReducer(loaded(), {
      type: 'decision',
      decision: decision('stray', 'gone'),
    });
    expect(selectDecisions(state).map((item) => item.id)).toEqual(['d1']);
  });

  it('ignores a workstream from another workspace', () => {
    const state = workstreamsReducer(loaded(), {
      type: 'workstream',
      workstream: workstream('elsewhere', { workspaceId: OTHER }),
    });
    expect(selectWorkstreams(state).map((item) => item.id)).toEqual(['b', 'a']);
  });

  it('starts over when the workspace changes, and forgets nothing durable', () => {
    const switched = workstreamsReducer(loaded(), {
      type: 'event',
      event: event('WorkspaceOpened', { workspace: workspace(OTHER) }),
    });
    expect(selectWorkstreams(switched)).toEqual([]);
    expect(switched.workspaceId).toBe(OTHER);
    expect(switched.loaded).toBe(false);

    const closed = workstreamsReducer(loaded(), {
      type: 'event',
      event: event('WorkspaceClosed', { workspaceId: WORKSPACE, root: '/repo', reason: 'USER' }),
    });
    expect(closed).toEqual(initialWorkstreamsState);
  });

  it('reopening the same workspace does not throw the roster away', () => {
    const state = loaded();
    const again = workstreamsReducer(state, {
      type: 'event',
      event: event('WorkspaceOpened', { workspace: workspace() }),
    });
    expect(again).toBe(state);
  });

  it('separates open questions from answered ones', () => {
    const answered = workstreamsReducer(loaded(), {
      type: 'event',
      event: event('DecisionResolved', {
        decision: decision('d1', 'a', { state: 'RESOLVED', chosen: 'yes' }),
      }),
    });
    expect(selectDecisions(answered)).toHaveLength(1);
    expect(selectOpenDecisions(answered)).toHaveLength(0);
  });

  it('leaves unrelated events alone', () => {
    const state = loaded();
    expect(
      workstreamsReducer(state, {
        type: 'event',
        event: event('TerminalOutput', { terminalId: 't', data: 'hi', truncatedBytes: 0 }),
      }),
    ).toBe(state);
  });
});
