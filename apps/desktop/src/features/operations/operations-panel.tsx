import type { Operation, OperationId, StartDemoRequest } from '@gritty/protocol';
import { type FormEvent, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { isTerminal } from './operations-reducer';
import { useOperations } from './use-operations';

const UNKNOWN_ID = 'does-not-exist' as OperationId;

/**
 * Development-only control surface for the operation model: start the demo
 * operation, watch progress, cancel, and force errors to see the envelope.
 *
 * Collapsed by default (T048): it is a debug affordance sharing a column with
 * the file tree, so it earns a row until asked for, not a panel.
 */
export function OperationsPanel() {
  const { operations, error, startDemo, cancel, clearError } = useOperations();
  const [steps, setSteps] = useState('20');
  const [stepMs, setStepMs] = useState('250');
  const [failAtStep, setFailAtStep] = useState('');

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const request: StartDemoRequest = { steps: Number(steps), stepMs: Number(stepMs) };
    if (failAtStep !== '') request.failAtStep = Number(failAtStep);
    void startDemo(request);
  }

  return (
    <details className="panel">
      <summary>Operations (development)</summary>
      <div className="panel__body">
        <form className="panel__form" onSubmit={onSubmit}>
          <label>
            steps
            <input value={steps} onChange={(e) => setSteps(e.target.value)} inputMode="numeric" />
          </label>
          <label>
            stepMs
            <input value={stepMs} onChange={(e) => setStepMs(e.target.value)} inputMode="numeric" />
          </label>
          <label>
            failAtStep
            <input
              value={failAtStep}
              onChange={(e) => setFailAtStep(e.target.value)}
              inputMode="numeric"
              placeholder="none"
            />
          </label>
          <button type="submit">Start demo</button>
          <button type="button" onClick={() => void cancel(UNKNOWN_ID)}>
            Cancel unknown id
          </button>
        </form>
        {error && <ErrorEnvelope error={error} onDismiss={clearError} />}
        {operations.length === 0 ? (
          <p className="panel__empty">No operations yet.</p>
        ) : (
          <ul className="ops">
            {operations.map((operation) => (
              <OperationRow key={operation.id} operation={operation} onCancel={cancel} />
            ))}
          </ul>
        )}
      </div>
    </details>
  );
}

function OperationRow({
  operation,
  onCancel,
}: {
  operation: Operation;
  onCancel: (id: OperationId) => Promise<void>;
}) {
  const terminal = isTerminal(operation.status);
  const { progress } = operation;
  return (
    <li className={`ops__row ops__row--${operation.status.toLowerCase()}`}>
      <div className="ops__head">
        <span className="ops__kind">{operation.kind}</span>
        <span className="ops__status">
          {operation.status}
          {operation.cancellationRequested && !terminal ? ' (cancelling)' : ''}
        </span>
        <code className="ops__id">{operation.id.slice(0, 8)}</code>
        {!terminal && (
          <button
            type="button"
            onClick={() => void onCancel(operation.id)}
            disabled={operation.cancellationRequested}
          >
            Cancel
          </button>
        )}
      </div>
      {progress && (
        <div className="ops__progress">
          {progress.total === undefined ? (
            <progress />
          ) : (
            <progress value={progress.current} max={progress.total} />
          )}
          <span>
            {progress.current}
            {progress.total !== undefined ? ` / ${progress.total}` : ''}
            {progress.message ? ` · ${progress.message}` : ''}
          </span>
        </div>
      )}
      {operation.error && (
        <p className="ops__error">
          {operation.error.code}: {operation.error.message}
        </p>
      )}
      <p className="ops__meta">
        created {formatTime(operation.createdAt)}
        {operation.finishedAt !== undefined ? `, finished ${formatTime(operation.finishedAt)}` : ''}
        {' · correlation '}
        {operation.correlationId.slice(0, 8)}
      </p>
    </li>
  );
}

function formatTime(millis: number): string {
  return new Date(millis).toLocaleTimeString();
}
