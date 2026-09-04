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
  'workspace.recents': 'workspace_recents',
  'workspace.forget': 'workspace_forget',
  'documents.open': 'documents_open',
  'documents.close': 'documents_close',
  'documents.update': 'documents_update',
  'documents.save': 'documents_save',
  'documents.reload': 'documents_reload',
  'documents.status': 'documents_status',
  'documents.list': 'documents_list',
  'shell.openExternal': 'shell_open_external',
  'session.restore': 'session_restore',
  'session.candidates': 'session_candidates',
  'session.recover': 'session_recover',
  'session.discard': 'session_discard',
  'session.update': 'session_update',
  'terminal.spawn': 'terminal_spawn',
  'terminal.write': 'terminal_write',
  'terminal.resize': 'terminal_resize',
  'terminal.rename': 'terminal_rename',
  'terminal.kill': 'terminal_kill',
  'terminal.list': 'terminal_list',
  'terminal.profiles': 'terminal_profiles',
} as const;

export type CommandName = keyof typeof COMMANDS;

/** The naming rule the table above follows; tests keep both in sync. */
export function toRustCommandName(name: string): string {
  return name.replace('.', '_').replace(/[A-Z]/g, (upper) => `_${upper.toLowerCase()}`);
}
