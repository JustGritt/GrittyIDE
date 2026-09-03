/**
 * Command names: `<domain>.<verb>` in TypeScript, `<domain>_<snake_verb>` in Rust
 * (Tauri command identifiers cannot contain dots). Commands that take arguments
 * take exactly one `request` object whose type is generated from Rust.
 */
export const COMMANDS = {
  'core.health': 'core_health',
  'operations.list': 'operations_list',
  'operations.cancel': 'operations_cancel',
  'operations.startDemo': 'operations_start_demo',
  'workspace.open': 'workspace_open',
  'workspace.close': 'workspace_close',
  'workspace.current': 'workspace_current',
  'workspace.list': 'workspace_list',
} as const;

export type CommandName = keyof typeof COMMANDS;

/** The naming rule the table above follows; tests keep both in sync. */
export function toRustCommandName(name: string): string {
  return name.replace('.', '_').replace(/[A-Z]/g, (upper) => `_${upper.toLowerCase()}`);
}
