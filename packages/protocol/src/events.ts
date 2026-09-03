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
 * One transport listener on the core event channel, demultiplexed by `name`.
 * `subscribe` is synchronous and safe to call (and undo) before the underlying
 * listener has been established.
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

  constructor(transport: Transport, channel: string, diagnostic: Diagnostic) {
    this.#transport = transport;
    this.#channel = channel;
    this.#diagnostic = diagnostic;
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
    if (this.#listening) return;
    const listening = this.#transport.listen(this.#channel, (payload) => this.#dispatch(payload));
    this.#listening = listening;
    listening
      .then((unlisten) => {
        if (this.#listening === listening) {
          this.#unlisten = unlisten;
        } else {
          // Disposed while the listener was being registered.
          unlisten();
        }
      })
      .catch((error: unknown) => {
        if (this.#listening === listening) this.#listening = null;
        this.#diagnostic('failed to listen on the core event channel', {
          channel: this.#channel,
          error: String(error),
        });
      });
  }

  #dispatch(payload: unknown): void {
    if (!isEnvelope(payload)) {
      this.#diagnostic('dropping malformed core event', { payload });
      return;
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
