// The one core client instance the UI talks to. All commands and events go
// through `@gritty/protocol`; nothing in apps/desktop touches Tauri directly.
import { createCoreClient, createTauriTransport } from '@gritty/protocol';

export const core = createCoreClient(createTauriTransport());
