export { type CoreClient, type CoreClientOptions, createCoreClient } from './client';
export { COMMANDS, type CommandName } from './commands';
export { CoreError, toCoreError } from './errors';
export type { Diagnostic, Unsubscribe } from './events';
export type { Transport, Unlisten } from './transport';
export { createTauriTransport } from './transport/tauri';
export * from './types';
export { assertCompatible, majorVersion } from './version';
