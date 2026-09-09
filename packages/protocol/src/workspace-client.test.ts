import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { COMMANDS } from './commands';
import { CoreError } from './errors';
import { createFakeTransport } from './testing/fake-transport';
import type {
  FindFilesResponse,
  HealthReport,
  ListDirectoryResponse,
  Operation,
  RecentWorkspace,
  Workspace,
  WorkspaceId,
} from './types';
import { PROTOCOL_VERSION } from './types';

const WS_ID = 'ws-1' as WorkspaceId;

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'linux',
  arch: 'x86_64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const workspace: Workspace = { id: WS_ID, root: '/ws', name: 'ws', openedAt: 1 };

/** What `workspace.findFiles` hands back when the walk outlives its budget. */
const pendingWalk: Operation = {
  id: 'op-find-1' as Operation['id'],
  kind: 'workspace.findFiles' as Operation['kind'],
  actor: { type: 'USER' },
  workspaceId: WS_ID,
  status: 'RUNNING' as Operation['status'],
  createdAt: 1 as Operation['createdAt'],
  cancellationRequested: false,
  correlationId: 'corr-1' as Operation['correlationId'],
};

const recents: RecentWorkspace[] = [
  { root: '/home/me/ws', displayRoot: '~/ws', name: 'ws', lastOpenedAt: 2, exists: true },
  { root: '/gone', displayRoot: '/gone', name: 'gone', lastOpenedAt: 1, exists: false },
];

function setup() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set('workspace_open', (args) => {
    const request = args?.request as { path?: string };
    return request.path === undefined ? null : workspace;
  });
  transport.handlers.set('workspace_close', () => workspace);
  transport.handlers.set('workspace_current', () => null);
  transport.handlers.set('workspace_recents', () => recents);
  transport.handlers.set('workspace_forget', () => null);
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
  transport.handlers.set('workspace_find_files', (args) => {
    const request = args?.request as { query: string; limit?: number };
    if (request.limit !== undefined && (request.limit < 1 || request.limit > 200)) {
      throw {
        code: 'InvalidInput',
        message: 'limit: must be between 1 and 200',
        retryable: false,
      };
    }
    if (request.query === 'slow') {
      const response: FindFilesResponse = { status: 'pending', operation: pendingWalk };
      return response;
    }
    const response: FindFilesResponse = {
      status: 'ready',
      matches: [{ path: 'src/editor-pane.tsx', score: 93, ranges: [[4, 1] as const, [11, 2]] }],
      truncated: false,
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

  it('sends the recents limit only when given, and forget as a request object', async () => {
    const { client, transport } = setup();
    await expect(client.workspace.recents()).resolves.toEqual(recents);
    await client.workspace.recents(5);
    await client.workspace.forget('/gone');
    expect(
      transport.calls
        .filter((c) => c.command.startsWith('workspace_'))
        .map((c) => [c.command, c.args]),
    ).toEqual([
      // An omitted limit is an absent field, not `undefined`: the core's
      // default is the one that applies.
      ['workspace_recents', { request: {} }],
      ['workspace_recents', { request: { limit: 5 } }],
      ['workspace_forget', { request: { root: '/gone' } }],
    ]);
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

  it('maps findFiles to workspace_find_files and sends one request object', async () => {
    const { client, transport } = setup();
    // The mapping itself: a client method whose Rust name is wrong is a
    // command that rejects at runtime, and nothing else in the tree asserts it.
    expect(COMMANDS['workspace.findFiles']).toBe('workspace_find_files');

    await client.workspace.findFiles({ workspaceId: WS_ID, query: 'epn' });
    await client.workspace.findFiles({ workspaceId: WS_ID, query: 'epn', limit: 10 });
    expect(
      transport.calls
        .filter((call) => call.command === 'workspace_find_files')
        .map((call) => call.args),
    ).toEqual([
      // An omitted limit is an absent field, not `undefined`, so the core's
      // default of 50 is the one that applies.
      { request: { workspaceId: WS_ID, query: 'epn' } },
      { request: { workspaceId: WS_ID, query: 'epn', limit: 10 } },
    ]);
  });

  it('narrows the findFiles union on status, and carries the highlight spans', async () => {
    const { client } = setup();
    const ready = await client.workspace.findFiles({ workspaceId: WS_ID, query: 'epn' });
    if (ready.status !== 'ready') throw new Error(`expected ready, got ${ready.status}`);
    expect(ready.matches[0]?.path).toBe('src/editor-pane.tsx');
    // The spans arrive with the match; the UI never re-runs the matcher.
    expect(ready.matches[0]?.ranges).toEqual([
      [4, 1],
      [11, 2],
    ]);
    expect(ready.truncated).toBe(false);

    // The pending arm is the normal answer on a real tree, not an edge case:
    // a caller that only handles `ready` works on a fixture and not at scale.
    const pending = await client.workspace.findFiles({ workspaceId: WS_ID, query: 'slow' });
    if (pending.status !== 'pending') throw new Error(`expected pending, got ${pending.status}`);
    expect(pending.operation.id).toBe('op-find-1');
    expect(pending.operation.kind).toBe('workspace.findFiles');
  });

  it('rejects a limit outside 1..=200 as CoreError rather than clamping it', async () => {
    const { client } = setup();
    const error = await client.workspace
      .findFiles({ workspaceId: WS_ID, query: 'epn', limit: 5000 })
      .catch((thrown: unknown) => thrown);
    expect(error).toBeInstanceOf(CoreError);
    expect((error as CoreError).code).toBe('InvalidInput');
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
