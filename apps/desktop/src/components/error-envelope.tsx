import type { CoreError } from '@gritty/protocol';

/** Renders every field of the error envelope so a failure is always inspectable. */
export function ErrorEnvelope({ error, onDismiss }: { error: CoreError; onDismiss?: () => void }) {
  return (
    <div className="envelope" role="alert">
      <dl className="envelope__fields">
        <dt>code</dt>
        <dd>{error.code}</dd>
        <dt>message</dt>
        <dd>{error.message}</dd>
        <dt>retryable</dt>
        <dd>{String(error.retryable)}</dd>
        <dt>correlationId</dt>
        <dd>{error.correlationId ?? 'none'}</dd>
        {error.details !== undefined && (
          <>
            <dt>details</dt>
            <dd>
              <pre>{JSON.stringify(error.details, null, 2)}</pre>
            </dd>
          </>
        )}
      </dl>
      {onDismiss && (
        <button type="button" onClick={onDismiss}>
          Dismiss
        </button>
      )}
    </div>
  );
}
