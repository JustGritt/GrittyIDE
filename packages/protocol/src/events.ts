import type { Transport, Unlisten } from './transport';
import type { EventEnvelope, EventName, EventOf } from './types';

export type Unsubscribe = () => void;

/** Non-fatal observations (sequence gaps, malformed payloads, throwing handlers). */
export type Diagnostic = (message: string, context: Record<string, unknown>) => void;

interface Subscription {
  name: EventName | null;
  handler: (event: EventEnvelope) => void;
}

/**
 * The first sequence number the core ever assigns (`EventBus::emit` increments
 * before it reads). A first envelope numbered above this is proof that events
 * were emitted before this client was listening.
 */
const FIRST_SEQ = 1;

/**
 * How long a command may wait for the listener before going anyway. Longer
 * than any plausible IPC round trip, short enough that a user never sees it:
 * this is a safety valve, not a synchronisation primitive.
 */
const DEFAULT_LISTEN_BUDGET_MS = 2_000;

/** Registration attempts before the channel is declared lost. */
const MAX_LISTEN_ATTEMPTS = 4;

/** First backoff step; each attempt doubles it. */
const LISTEN_RETRY_MS = 100;

/**
 * One transport listener on the core event channel, demultiplexed by `name`.
 * `subscribe` is synchronous and safe to call (and undo) before the underlying
 * listener has been established.
 *
 * Registering that listener is itself asynchronous — an IPC round trip — and
 * an event the core emits before it completes is delivered to nobody at all.
 * Two things follow, and they are why `start` and `ready` exist:
 *
 * - `start` begins registration as early as the client exists, rather than
 *   waiting for the first subscriber, and `ready` lets the command path hold
 *   until the channel is open. The client awaits it before every command, so
 *   an event *caused by a command* cannot be lost (ADR-009).
 * - What is lost anyway is reported. `seq` starts at 1, so a first envelope
 *   numbered higher means events were missed, and saying so is the difference
 *   between a bug you can see and the one this class shipped.
 */
export class EventHub {
  readonly #transport: Transport;
  readonly #channel: string;
  readonly #diagnostic: Diagnostic;
  readonly #subscriptions = new Set<Subscription>();
  #listening: Promise<Unlisten> | null = null;
  #unlisten: Unlisten | null = null;
  #disposed = false;
  #lastSeq: number | undefined;
  /** The transport listener is live; nothing emitted from now on is lost here. */
  #registered = false;
  /** Memoised so the budget below arms one timer, not one per command. */
  #ready: Promise<void> | null = null;
  #attempts = 0;
  readonly #budgetMs: number;

  constructor(
    transport: Transport,
    channel: string,
    diagnostic: Diagnostic,
    budgetMs: number = DEFAULT_LISTEN_BUDGET_MS,
  ) {
    this.#transport = transport;
    this.#channel = channel;
    this.#diagnostic = diagnostic;
    this.#budgetMs = budgetMs;
  }

  /**
   * Open the channel now. Called by `createCoreClient` at construction so the
   * registration round trip overlaps the compatibility probe instead of
   * queueing behind the first subscriber.
   */
  start(): void {
    if (this.#disposed) return;
    this.#ensureListening();
  }

  /**
   * Resolves once the listener is registered — or once the budget expires, or
   * once registration has failed for good.
   *
   * **It never rejects and it is never indefinite.** A dead event channel must
   * degrade to "commands work, events do not"; blocking every command on a
   * listener that will never arrive would turn a lost event into a frozen
   * window, which is the worse failure by every line of the priority order.
   */
  ready(): Promise<void> {
    if (this.#registered || this.#disposed) return Promise.resolve();
    this.#ensureListening();
    const listening = this.#listening;
    // Registration already failed past retrying: go, and let the diagnostic
    // that recorded it stand.
    if (!listening) return Promise.resolve();
    if (!this.#ready) this.#ready = this.#latch(listening);
    return this.#ready;
  }

  /**
   * Settles once — on registration, on a registration that failed, or on the
   * budget — and clears the timer behind it. One-shot rather than a `race` so
   * that a client never leaves a live timer armed for its whole lifetime:
   * every test in this package builds a client, and a stray two-second timer
   * is how a suite starts hanging under fake timers.
   */
  #latch(listening: Promise<Unlisten>): Promise<void> {
    return new Promise<void>((resolve) => {
      let timer: ReturnType<typeof setTimeout>;
      let settled = false;
      const settle = (expired = false): void => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        if (expired && !this.#registered && !this.#disposed) {
          this.#diagnostic('sending commands before the core event channel is listening', {
            channel: this.#channel,
            budgetMs: this.#budgetMs,
          });
        }
        resolve();
      };
      timer = setTimeout(() => settle(true), this.#budgetMs);
      // A safety valve must not be the reason a process stays alive.
      (timer as { unref?: () => void }).unref?.();
      listening.then(
        () => settle(),
        () => settle(),
      );
    });
  }

  subscribe<N extends EventName>(name: N, handler: (event: EventOf<N>) => void): Unsubscribe {
    return this.#add({ name, handler: handler as (event: EventEnvelope) => void });
  }

  subscribeAll(handler: (event: EventEnvelope) => void): Unsubscribe {
    return this.#add({ name: null, handler });
  }

  /** Stop listening; existing subscriptions are dropped. */
  dispose(): void {
    this.#disposed = true;
    this.#subscriptions.clear();
    this.#registered = false;
    this.#ready = null;
    // A listener still being registered is released by `#ensureListening`
    // once it resolves and notices it is no longer current.
    this.#listening = null;
    const unlisten = this.#unlisten;
    this.#unlisten = null;
    unlisten?.();
  }

  #add(subscription: Subscription): Unsubscribe {
    if (this.#disposed) {
      throw new Error('EventHub is disposed');
    }
    this.#subscriptions.add(subscription);
    this.#ensureListening();
    return () => {
      this.#subscriptions.delete(subscription);
    };
  }

  #ensureListening(): void {
    if (this.#listening || this.#disposed) return;
    this.#attempts += 1;
    const listening = this.#transport.listen(this.#channel, (payload) => this.#dispatch(payload));
    this.#listening = listening;
    listening
      .then((unlisten) => {
        if (this.#listening === listening) {
          this.#unlisten = unlisten;
          this.#registered = true;
        } else {
          // Disposed while the listener was being registered.
          unlisten();
        }
      })
      .catch((error: unknown) => {
        if (this.#listening === listening) this.#listening = null;
        this.#diagnostic('failed to listen on the core event channel', {
          channel: this.#channel,
          attempt: this.#attempts,
          error: String(error),
        });
        this.#retry();
      });
  }

  /**
   * Try again after a failed registration, with a bounded backoff.
   *
   * Without this, one rejected `listen()` left every subscriber already added
   * permanently deaf: `#ensureListening` cleared its promise, but only a *new*
   * `subscribe` call would have tried again, and the hooks that make up this
   * UI subscribe exactly once, at mount.
   */
  #retry(): void {
    if (this.#disposed) return;
    if (this.#attempts >= MAX_LISTEN_ATTEMPTS) {
      this.#diagnostic('giving up on the core event channel; events will not arrive', {
        channel: this.#channel,
        attempts: this.#attempts,
      });
      return;
    }
    const delay = LISTEN_RETRY_MS * 2 ** (this.#attempts - 1);
    const timer = setTimeout(() => this.#ensureListening(), delay);
    (timer as { unref?: () => void }).unref?.();
  }

  #dispatch(payload: unknown): void {
    if (!isEnvelope(payload)) {
      this.#diagnostic('dropping malformed core event', { payload });
      return;
    }
    if (this.#lastSeq === undefined && payload.seq > FIRST_SEQ) {
      // The gap the discontinuity check below cannot see, having no previous
      // envelope to compare against.
      //
      // Deliberately worded as what it is and not as loss. The core is built
      // during Tauri's `setup`, before the window is even shown, so a healthy
      // cold start reaches here with a handful of events already numbered —
      // and a WebView reload with a great many. Calling that "lost" would fire
      // a warning on every launch, and ADR-009's revisit trigger is literally
      // "`seq` gaps are observed in practice": arming it with noise is worse
      // than staying quiet. Turning this into proof of loss needs a baseline
      // the client does not have; see the follow-up in the task file.
      this.#diagnostic('core events preceded this listener', {
        channel: this.#channel,
        preceding: payload.seq - FIRST_SEQ,
        firstSeen: payload.seq,
      });
    }
    if (this.#lastSeq !== undefined && payload.seq !== this.#lastSeq + 1) {
      this.#diagnostic('core event sequence discontinuity', {
        expected: this.#lastSeq + 1,
        received: payload.seq,
        name: payload.name,
      });
    }
    if (this.#lastSeq === undefined || payload.seq > this.#lastSeq) this.#lastSeq = payload.seq;
    for (const subscription of [...this.#subscriptions]) {
      if (subscription.name !== null && subscription.name !== payload.name) continue;
      try {
        subscription.handler(payload);
      } catch (error: unknown) {
        this.#diagnostic('core event handler threw', {
          name: payload.name,
          seq: payload.seq,
          error: String(error),
        });
      }
    }
  }
}

function isEnvelope(value: unknown): value is EventEnvelope {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.name === 'string' &&
    typeof candidate.seq === 'number' &&
    typeof candidate.actor === 'object' &&
    candidate.actor !== null &&
    'payload' in candidate
  );
}
