export type Unlisten = () => void;

/**
 * The two primitives the client needs from the shell. `createTauriTransport`
 * implements them over `@tauri-apps/api`; tests use an in-memory fake.
 */
export interface Transport {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen(channel: string, handler: (payload: unknown) => void): Promise<Unlisten>;
}
