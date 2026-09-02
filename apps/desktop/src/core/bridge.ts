// The ONLY module allowed to import `@tauri-apps/api` during T000.
// T010 replaces this with the typed core protocol client in `packages/protocol`;
// UI code must never call Tauri primitives directly.
import { invoke } from '@tauri-apps/api/core';
import type { AppHealth } from './health';

export function fetchHealth(): Promise<AppHealth> {
  return invoke<AppHealth>('health');
}
