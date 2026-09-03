import type { HealthReport } from '@gritty/protocol';

/** One-line human summary; pure so it can be unit-tested without Tauri. */
export function describeHealth(health: HealthReport): string {
  const build = health.debug ? 'debug' : 'release';
  return `core ${health.coreVersion} (${build}) on ${health.os}/${health.arch} via ${health.shell.runtime} ${health.shell.runtimeVersion}, protocol ${health.protocolVersion}`;
}
