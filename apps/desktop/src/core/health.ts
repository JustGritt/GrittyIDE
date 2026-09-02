/** Mirrors `AppHealth` in apps/desktop/src-tauri/src/commands.rs (camelCase via serde). */
export interface AppHealth {
  appVersion: string;
  tauriVersion: string;
  webviewVersion: string | null;
  core: CoreHealth;
}

/** Mirrors `gritty_core::HealthReport`. */
export interface CoreHealth {
  coreVersion: string;
  os: string;
  arch: string;
  debug: boolean;
}

/** One-line human summary; pure so it can be unit-tested without Tauri. */
export function describeHealth(health: AppHealth): string {
  const build = health.core.debug ? 'debug' : 'release';
  return `core ${health.core.coreVersion} (${build}) on ${health.core.os}/${health.core.arch} via Tauri ${health.tauriVersion}`;
}
