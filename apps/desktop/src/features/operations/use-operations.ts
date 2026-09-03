import {
  type CoreError,
  type Operation,
  type OperationId,
  type StartDemoRequest,
  toCoreError,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useState } from 'react';
import { core } from '../../core/client';
import { initialOperationsState, operationsReducer, selectOperations } from './operations-reducer';

export function useOperations() {
  const [state, dispatch] = useReducer(operationsReducer, initialOperationsState);
  const [error, setError] = useState<CoreError | null>(null);

  useEffect(() => {
    // Subscribe before listing so nothing emitted in between is lost; the
    // reducer merges the list with any event that arrived first.
    const unsubscribe = core.subscribeAll((event) => dispatch({ type: 'event', event }));
    let active = true;
    core.operations.list().then(
      (operations) => {
        if (active) dispatch({ type: 'rehydrate', operations });
      },
      (cause: unknown) => {
        if (active) setError(toCoreError(cause));
      },
    );
    return () => {
      active = false;
      unsubscribe();
    };
  }, []);

  const run = useCallback(async (action: () => Promise<Operation>) => {
    try {
      const operation = await action();
      dispatch({ type: 'snapshot', operation });
      setError(null);
    } catch (cause: unknown) {
      setError(toCoreError(cause));
    }
  }, []);

  const startDemo = useCallback(
    (request: StartDemoRequest) => run(() => core.operations.startDemo(request)),
    [run],
  );
  const cancel = useCallback((id: OperationId) => run(() => core.operations.cancel(id)), [run]);
  const clearError = useCallback(() => setError(null), []);

  return { operations: selectOperations(state), error, startDemo, cancel, clearError };
}
