import type { HealthReport } from '@gritty/protocol';
import { PROTOCOL_VERSION } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { describeHealth } from './health';

const sample: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: {
    appVersion: '0.1.0',
    runtime: 'tauri',
    runtimeVersion: '2.11.5',
    webviewVersion: 'WebKit 620.1',
  },
};

describe('describeHealth', () => {
  it('summarises core version, build type, platform, runtime and protocol', () => {
    expect(describeHealth(sample)).toBe(
      `core 0.1.0 (debug) on macos/aarch64 via tauri 2.11.5, protocol ${PROTOCOL_VERSION}`,
    );
  });

  it('labels release builds', () => {
    expect(describeHealth({ ...sample, debug: false })).toContain('(release)');
  });
});
