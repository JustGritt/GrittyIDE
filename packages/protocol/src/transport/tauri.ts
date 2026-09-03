// The ONLY module in the repository allowed to import `@tauri-apps/api`
// (ADR-003; enforced by Biome `noRestrictedImports`).
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Transport } from '../transport';

export function createTauriTransport(): Transport {
  return {
    invoke: (command, args) => invoke(command, args),
    listen: (channel, handler) => listen<unknown>(channel, (event) => handler(event.payload)),
  };
}
