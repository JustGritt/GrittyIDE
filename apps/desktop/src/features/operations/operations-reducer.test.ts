import type { EventEnvelope, Operation, OperationId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  initialOperationsState,
  isTerminal,
  operationsReducer,
  selectOperations,
} from './operations-reducer';

const ID = 'op-1' as OperationId;

function op(overrides: Partial<Operation> = {}): Operation {
  return {
    id: ID,
    kind: 'operations.demo',
    actor: { type: 'USER' },
    status: 'CREATED',
    createdAt: 100,
    cancellationRequested: false,
    correlationId: 'corr-1' as Operation['correlationId'],
    ...overrides,
  };
}

function event(name: EventEnvelope['name'], operation: Operation, seq = 1): EventEnvelope {
  if (name === 'OperationProgress') {
    return {
      seq,
      actor: { type: 'USER' },
      name,
      payload: { operationId: operation.id, progress: { current: 2, total: 5 } },
    };
  }
  if (name === 'OperationCompleted') {
    return { seq, actor: { type: 'USER' }, name, payload: { operation } };
  }
  if (name === 'OperationFailed') {
    return {
      seq,
      actor: { type: 'USER' },
      name,
      payload: {
        operation,
        error: { code: 'Internal', message: 'boom', retryable: false },
      },
    };
  }
  if (name === 'OperationCancelled') {
    return { seq, actor: { type: 'USER' }, name, payload: { operation } };
  }
  return { seq, actor: { type: 'USER' }, name: 'OperationStarted', payload: { operation } };
}

describe('operationsReducer', () => {
  it('rehydrates from a list and orders newest first', () => {
    const state = operationsReducer(initialOperationsState, {
      type: 'rehydrate',
      operations: [op(), op({ id: 'op-2' as OperationId, createdAt: 200 })],
    });
    expect(selectOperations(state).map((o) => o.id)).toEqual(['op-2', 'op-1']);
  });

  it('never lets a stale CREATED snapshot overwrite a RUNNING event', () => {
    const running = op({ status: 'RUNNING', startedAt: 101 });
    let state = operationsReducer(initialOperationsState, {
      type: 'event',
      event: event('OperationStarted', running),
    });
    state = operationsReducer(state, { type: 'snapshot', operation: op() });
    expect(state.byId[ID]?.status).toBe('RUNNING');
    state = operationsReducer(state, { type: 'rehydrate', operations: [op()] });
    expect(state.byId[ID]?.status).toBe('RUNNING');
  });

  it('applies progress to running operations only', () => {
    const running = op({ status: 'RUNNING' });
    let state = operationsReducer(initialOperationsState, {
      type: 'event',
      event: event('OperationStarted', running),
    });
    state = operationsReducer(state, { type: 'event', event: event('OperationProgress', running) });
    expect(state.byId[ID]?.progress).toEqual({ current: 2, total: 5 });

    const cancelled = op({ status: 'CANCELLED', cancellationRequested: true, finishedAt: 300 });
    state = operationsReducer(state, {
      type: 'event',
      event: event('OperationCancelled', cancelled),
    });
    const afterCancel = state;
    state = operationsReducer(state, { type: 'event', event: event('OperationProgress', running) });
    expect(state).toBe(afterCancel);
    expect(state.byId[ID]?.status).toBe('CANCELLED');
    // Progress known before the terminal snapshot is kept for display.
    expect(state.byId[ID]?.progress).toEqual({ current: 2, total: 5 });
  });

  it('ignores progress for unknown operations', () => {
    const state = operationsReducer(initialOperationsState, {
      type: 'event',
      event: event('OperationProgress', op()),
    });
    expect(state).toBe(initialOperationsState);
  });

  it('records failures with their error', () => {
    const failed = op({
      status: 'FAILED',
      error: { code: 'Internal', message: 'boom', retryable: false },
    });
    const state = operationsReducer(initialOperationsState, {
      type: 'event',
      event: event('OperationFailed', failed),
    });
    expect(state.byId[ID]?.error?.code).toBe('Internal');
    expect(isTerminal('FAILED')).toBe(true);
    expect(isTerminal('RUNNING')).toBe(false);
  });
});
