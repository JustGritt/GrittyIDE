/**
 * Records why Monaco's web workers did not start. The editor keeps working
 * without them (Monaco falls back to an in-process editor worker; language
 * features degrade), so the UI needs a visible warning with the details:
 * construction errors, `error` events on the worker, and the
 * `securitypolicyviolation` events that are the only place a CSP-blocked
 * worker script reports its directive and URI.
 */
export interface WorkerFailure {
  /** Monaco's worker label (`editorWorkerService`, `json`, `typescript`, …) or the CSP directive. */
  readonly label: string;
  readonly phase: 'create' | 'runtime' | 'csp';
  readonly reason: string;
}

export interface WorkerStatusSnapshot {
  readonly failures: readonly WorkerFailure[];
  /** Labels of workers that were constructed without throwing. */
  readonly started: readonly string[];
}

export type WorkerStatusListener = (snapshot: WorkerStatusSnapshot) => void;

/** The part of `Worker` this module needs; tests pass fakes. */
export type WorkerLike = Pick<Worker, 'addEventListener'>;

export interface WorkerStatusOptions {
  /** Diagnostic sink; defaults to `console.warn`. */
  log?: (message: string, context: Record<string, unknown>) => void;
  /** Where `securitypolicyviolation` events are observed; `null` disables (tests, non-browser). */
  violationTarget?: Pick<EventTarget, 'addEventListener' | 'removeEventListener'> | null;
}

const EMPTY: WorkerStatusSnapshot = { failures: [], started: [] };

function describeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

function describeErrorEvent(event: Event): string {
  if ('message' in event && typeof event.message === 'string' && event.message !== '') {
    const where =
      'filename' in event && typeof event.filename === 'string' && event.filename !== ''
        ? ` (${event.filename}:${'lineno' in event ? String(event.lineno) : '?'})`
        : '';
    return `${event.message}${where}`;
  }
  return 'worker error event without a message (script blocked or failed to load)';
}

function describeViolation(event: Event): { label: string; reason: string } {
  const violation = event as Partial<SecurityPolicyViolationEvent>;
  const directive =
    violation.effectiveDirective ?? violation.violatedDirective ?? 'unknown-directive';
  const blocked = violation.blockedURI ?? 'unknown URI';
  const source =
    violation.sourceFile !== undefined && violation.sourceFile !== ''
      ? ` at ${violation.sourceFile}:${String(violation.lineNumber ?? '?')}`
      : '';
  return { label: directive, reason: `CSP ${directive} blocked ${blocked}${source}` };
}

export class WorkerStatus {
  readonly #log: (message: string, context: Record<string, unknown>) => void;
  readonly #listeners = new Set<WorkerStatusListener>();
  readonly #violationTarget: Pick<EventTarget, 'addEventListener' | 'removeEventListener'> | null;
  #snapshot: WorkerStatusSnapshot = EMPTY;
  #disposed = false;

  constructor(options: WorkerStatusOptions = {}) {
    this.#log =
      options.log ?? ((message, context) => console.warn(`[gritty editor] ${message}`, context));
    this.#violationTarget = options.violationTarget ?? null;
    this.#violationTarget?.addEventListener('securitypolicyviolation', this.#onViolation);
  }

  /**
   * Run a worker factory under observation. A construction error is recorded
   * and rethrown so Monaco applies its own fallback.
   */
  track<W extends WorkerLike>(label: string, create: () => W): W {
    let worker: W;
    try {
      worker = create();
    } catch (error: unknown) {
      this.#fail({ label, phase: 'create', reason: describeError(error) });
      throw error;
    }
    worker.addEventListener('error', (event: Event) => {
      this.#fail({ label, phase: 'runtime', reason: describeErrorEvent(event) });
    });
    this.#update({ ...this.#snapshot, started: [...this.#snapshot.started, label] });
    return worker;
  }

  snapshot(): WorkerStatusSnapshot {
    return this.#snapshot;
  }

  subscribe(listener: WorkerStatusListener): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  dispose(): void {
    this.#disposed = true;
    this.#listeners.clear();
    this.#violationTarget?.removeEventListener('securitypolicyviolation', this.#onViolation);
  }

  readonly #onViolation = (event: Event): void => {
    const { label, reason } = describeViolation(event);
    this.#fail({ label, phase: 'csp', reason });
  };

  #fail(failure: WorkerFailure): void {
    this.#log('monaco worker failure', { ...failure });
    this.#update({ ...this.#snapshot, failures: [...this.#snapshot.failures, failure] });
  }

  #update(snapshot: WorkerStatusSnapshot): void {
    if (this.#disposed) return;
    this.#snapshot = snapshot;
    for (const listener of [...this.#listeners]) listener(snapshot);
  }
}
