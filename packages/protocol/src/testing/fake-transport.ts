import type { Transport, Unlisten } from '../transport';

export interface InvokeCall {
  command: string;
  args: Record<string, unknown> | undefined;
}

export interface FakeTransport extends Transport {
  /** Rust command name → handler; a thrown/rejected value becomes the rejection. */
  handlers: Map<string, (args: Record<string, unknown> | undefined) => unknown>;
  calls: InvokeCall[];
  /** Deliver a payload to every listener as if the core emitted it. */
  emit(payload: unknown): void;
  listenCalls: number;
  unlistenCalls: number;
  /** Resolve pending `listen()` promises (only when created with `deferListen`). */
  resolveListen(): void;
}

export function createFakeTransport(options: { deferListen?: boolean } = {}): FakeTransport {
  const listeners = new Set<(payload: unknown) => void>();
  const pending: Array<() => void> = [];
  const transport: FakeTransport = {
    handlers: new Map(),
    calls: [],
    listenCalls: 0,
    unlistenCalls: 0,
    async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      transport.calls.push({ command, args });
      const handler = transport.handlers.get(command);
      if (!handler) throw `Command ${command} not found`;
      return (await handler(args)) as T;
    },
    listen(_channel, handler): Promise<Unlisten> {
      transport.listenCalls += 1;
      const register = (): Unlisten => {
        listeners.add(handler);
        return () => {
          transport.unlistenCalls += 1;
          listeners.delete(handler);
        };
      };
      if (!options.deferListen) return Promise.resolve(register());
      return new Promise((resolve) => {
        pending.push(() => resolve(register()));
      });
    },
    emit(payload) {
      for (const listener of [...listeners]) listener(payload);
    },
    resolveListen() {
      for (const resolve of pending.splice(0)) resolve();
    },
  };
  return transport;
}
