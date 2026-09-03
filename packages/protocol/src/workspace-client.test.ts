import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { CoreError } from './errors';
import { createFakeTransport } from './testing/fake-transport';
import type { HealthReport, ListDirectoryResponse, Workspace, WorkspaceId } from './types';

const WS_ID = 'ws-1' as WorkspaceId;

const health: HealthReport = {
  protocolVersion: '1.0.0',
  coreVersion: '0.1.0',
  os: 'linux',
  arch: 'x86_64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const workspace: Workspace = { id: WS_ID, root: '/ws', name: 'ws', openedAt: 1 };

function setup() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set('workspace_open', (args) => {
    const request = args?.request as { path?: string };
    return request.path === undefined ? null : workspace;
  });
  transport.handlers.set('workspace_close', () => workspace);
  transport.handlers.set('workspace_current', () => null);
  transport.handlers.set('workspace_list', (args) => {
    const request = args?.request as { path: string };
    if (request.path === '..') {
      throw { code: 'InvalidInput', message: "path: must not contain '..'", retryable: false };
    }
    const response: ListDirectoryResponse = {
      status: 'ready',
      page: { path: request.path, entries: [], total: 0, truncated: false },
    };
    return response;
  });
  return { transport, client: createCoreClient(transport) };
}

describe('workspace commands', () => {
  it('opens through the picker (no path) or a given path', async () => {
    const { client, transport } = setup();
    await expect(client.workspace.open()).resolves.toBeNull();
    await expect(client.workspace.open('/ws')).resolves.toEqual(workspace);
    const opens = transport.calls.filter((c) => c.command === 'workspace_open');
    expect(opens.map((c) => c.args)).toEqual([{ request: {} }, { request: { path: '/ws' } }]);
  });

  it('wraps close, current and list arguments as request objects', async () => {
    const { client, transport } = setup();
    await client.workspace.current();
    await client.workspace.close(WS_ID);
    const listing = await client.workspace.list({ workspaceId: WS_ID, path: 'src', cursor: 'fx' });
    expect(listing.status).toBe('ready');
    expect(transport.calls.map((c) => c.command)).toEqual([
      'core_health',
      'workspace_current',
      'workspace_close',
      'workspace_list',
    ]);
    expect(transport.calls[1]?.args).toBeUndefined();
    expect(transport.calls[2]?.args).toEqual({ request: { workspaceId: WS_ID } });
    expect(transport.calls[3]?.args).toEqual({
      request: { workspaceId: WS_ID, path: 'src', cursor: 'fx' },
    });
  });

  it('surfaces path policy rejections as CoreError', async () => {
    const { client } = setup();
    const error = await client.workspace
      .list({ workspaceId: WS_ID, path: '..' })
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(CoreError);
    expect((error as CoreError).code).toBe('InvalidInput');
  });
});
