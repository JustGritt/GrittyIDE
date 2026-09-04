import type { UpdateSessionRequest } from '@gritty/protocol';
import type { SchedulerTimers } from '../documents/update-scheduler';

/** Tab order, active tab and view mode: written almost at once (a quit may follow). */
export const TABS_DELAY_MS = 100;
/** Cursor, selection and scroll: they move on every keystroke. */
export const VIEW_DELAY_MS = 1000;
/** Editor preferences and layout. */
export const SETTINGS_DELAY_MS = 500;

export type SessionField = 'tabs' | 'preferences' | 'layout';

export interface SessionWriterOptions {
  /** The current UI state; `null` fields are not sent. Read at send time. */
  snapshot(): UpdateSessionRequest;
  send(request: UpdateSessionRequest): Promise<void>;
  onError?(error: unknown): void;
  timers?: SchedulerTimers;
}

const REAL_TIMERS: SchedulerTimers = {
  set: (callback, ms) => setTimeout(callback, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/**
 * Coalesces `session.update` calls: every field change names a delay, the
 * earliest pending delay wins, one request carries every pending field, and
 * sends never overlap. Disabled until the startup restore settled, so a kill
 * during restoration cannot shrink the persisted session.
 */
export class SessionWriter {
  readonly #options: SessionWriterOptions;
  readonly #timers: SchedulerTimers;
  readonly #pending = new Set<SessionField>();
  #handle: unknown = undefined;
  #due: number | null = null;
  #enabled = false;
  #inFlight: Promise<void> = Promise.resolve();
  #now: () => number;

  constructor(options: SessionWriterOptions, now: () => number = () => Date.now()) {
    this.#options = options;
    this.#timers = options.timers ?? REAL_TIMERS;
    this.#now = now;
  }

  get enabled(): boolean {
    return this.#enabled;
  }

  enable(): void {
    this.#enabled = true;
  }

  disable(): void {
    this.#enabled = false;
    this.#clearTimer();
    this.#pending.clear();
  }

  /** Persist `field` after `delayMs` (sooner if something else is already due). */
  schedule(field: SessionField, delayMs: number): void {
    if (!this.#enabled) return;
    this.#pending.add(field);
    const due = this.#now() + delayMs;
    if (this.#due !== null && this.#due <= due) return;
    this.#clearTimer();
    this.#due = due;
    this.#handle = this.#timers.set(() => {
      this.#handle = undefined;
      this.#due = null;
      void this.#fire();
    }, delayMs);
  }

  /** Send whatever is pending now; resolves once the request settled. */
  flush(): Promise<void> {
    this.#clearTimer();
    return this.#fire();
  }

  #fire(): Promise<void> {
    if (this.#pending.size === 0) return this.#inFlight;
    const fields = new Set(this.#pending);
    this.#pending.clear();
    const chained = this.#inFlight.then(async () => {
      const snapshot = this.#options.snapshot();
      const request: UpdateSessionRequest = {};
      if (fields.has('tabs') && snapshot.tabs !== undefined && snapshot.workspaceId !== undefined) {
        request.workspaceId = snapshot.workspaceId;
        request.tabs = snapshot.tabs;
      }
      if (fields.has('preferences') && snapshot.preferences !== undefined) {
        request.preferences = snapshot.preferences;
      }
      if (fields.has('layout') && snapshot.layout !== undefined) request.layout = snapshot.layout;
      if (Object.keys(request).length === 0) return;
      try {
        await this.#options.send(request);
      } catch (error: unknown) {
        this.#options.onError?.(error);
      }
    });
    this.#inFlight = chained;
    return chained;
  }

  #clearTimer(): void {
    if (this.#handle === undefined) return;
    this.#timers.clear(this.#handle);
    this.#handle = undefined;
    this.#due = null;
  }
}
