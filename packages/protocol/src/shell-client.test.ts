import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type { HealthReport } from './types';
import { PROTOCOL_VERSION } from './types';

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

describe('shell client', () => {
  it('sends the URL as the single request object and resolves with nothing', async () => {
    const transport = createFakeTransport();
    transport.handlers.set('core_health', () => health);
    transport.handlers.set('shell_open_external', () => null);
    const client = createCoreClient(transport);

    await expect(client.shell.openExternal('https://example.com/docs')).resolves.toBeNull();
    const call = transport.calls.find((c) => c.command === 'shell_open_external');
    expect(call?.args).toEqual({ request: { url: 'https://example.com/docs' } });
  });

  it('surfaces the core rejection as a CoreError', async () => {
    const transport = createFakeTransport();
    transport.handlers.set('core_health', () => health);
    transport.handlers.set('shell_open_external', () => {
      throw {
        code: 'InvalidInput',
        message: "url: scheme 'javascript' is not allowed (http and https only)",
        details: {
          field: 'url',
          reason: "scheme 'javascript' is not allowed (http and https only)",
        },
        retryable: false,
      };
    });
    const client = createCoreClient(transport);
    await expect(client.shell.openExternal('javascript:alert(1)')).rejects.toMatchObject({
      code: 'InvalidInput',
      details: { field: 'url' },
    });
  });
});
