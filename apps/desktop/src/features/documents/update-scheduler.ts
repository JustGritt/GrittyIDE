import type { DocumentId } from '@gritty/protocol';

export interface SchedulerTimers {
  set(callback: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

export interface UpdateSchedulerOptions<R> {
  debounceMs(documentId: DocumentId): number;
  /** Sends whatever the document has pending. Must not reject. */
  flush(documentId: DocumentId): Promise<R>;
  timers?: SchedulerTimers;
}

const REAL_TIMERS: SchedulerTimers = {
  set: (callback, ms) => setTimeout(callback, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/**
 * Debounces edits into `documents.update` calls and keeps them serialized per
 * document (ADR-011: a caller must await one update before sending the next,
 * otherwise the second is stale by construction). `settle` is what save and
 * close use: it waits for the update in flight, then flushes what was typed
 * meanwhile, and resolves with that flush's result.
 */
export class UpdateScheduler<R = void> {
  readonly #options: UpdateSchedulerOptions<R>;
  readonly #timers: SchedulerTimers;
  readonly #handles = new Map<DocumentId, unknown>();
  readonly #inFlight = new Map<DocumentId, Promise<R>>();

  constructor(options: UpdateSchedulerOptions<R>) {
    this.#options = options;
    this.#timers = options.timers ?? REAL_TIMERS;
  }

  /** (Re)start the debounce; the flush runs when the document stays quiet long enough. */
  touch(documentId: DocumentId): void {
    this.#clearTimer(documentId);
    const handle = this.#timers.set(() => {
      this.#handles.delete(documentId);
      this.settle(documentId).catch(() => undefined);
    }, this.#options.debounceMs(documentId));
    this.#handles.set(documentId, handle);
  }

  /** Flush now (after any update in flight) and resolve with the flush result. */
  settle(documentId: DocumentId): Promise<R> {
    this.#clearTimer(documentId);
    const previous = this.#inFlight.get(documentId);
    const chain = previous
      ? previous.then(
          () => undefined,
          () => undefined,
        )
      : Promise.resolve();
    const tracked: Promise<R> = chain
      .then(() => this.#options.flush(documentId))
      .finally(() => {
        if (this.#inFlight.get(documentId) === tracked) this.#inFlight.delete(documentId);
      });
    this.#inFlight.set(documentId, tracked);
    return tracked;
  }

  /** Drop the pending flush of a document (close, reload); an update in flight still completes. */
  cancel(documentId: DocumentId): void {
    this.#clearTimer(documentId);
    this.#inFlight.delete(documentId);
  }

  cancelAll(): void {
    for (const documentId of [...this.#handles.keys()]) this.#clearTimer(documentId);
    this.#inFlight.clear();
  }

  isScheduled(documentId: DocumentId): boolean {
    return this.#handles.has(documentId) || this.#inFlight.has(documentId);
  }

  #clearTimer(documentId: DocumentId): void {
    const handle = this.#handles.get(documentId);
    if (handle === undefined) return;
    this.#timers.clear(handle);
    this.#handles.delete(documentId);
  }
}
