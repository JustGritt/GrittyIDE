import { describe, expect, it } from 'vitest';
import { type AppHealth, describeHealth } from './health';

const sample: AppHealth = {
  appVersion: '0.1.0',
  tauriVersion: '2.11.5',
  webviewVersion: 'WebKit 620.1',
  core: { coreVersion: '0.1.0', os: 'macos', arch: 'aarch64', debug: true },
};

describe('describeHealth', () => {
  it('summarises core version, build type, platform and runtime', () => {
    expect(describeHealth(sample)).toBe('core 0.1.0 (debug) on macos/aarch64 via Tauri 2.11.5');
  });

  it('labels release builds', () => {
    const release = { ...sample, core: { ...sample.core, debug: false } };
    expect(describeHealth(release)).toContain('(release)');
  });
});
