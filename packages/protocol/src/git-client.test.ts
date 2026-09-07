import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { COMMANDS } from './commands';
import { createFakeTransport } from './testing/fake-transport';
import type {
  CoreErrorShape,
  DiffSummary,
  GitDiffResponse,
  GitStatusResponse,
  HealthReport,
  Operation,
  RepositoryStatus,
  Workspace,
} from './types';
import { PROTOCOL_VERSION } from './types';

// PROTO-W2-GIT: the two `git.*` commands. T100 built the core and nothing could
// call it — `commands.rs` had no `git.` in it at all. This covers the wire that
// reaches it: the names, the request each carries, and — the part worth having
// a test for — that all three arms of the response are *values* the caller has
// to handle, not two values and an error.

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const workspaceId = 'w-1' as Workspace['id'];

const repository: RepositoryStatus = {
  state: 'CLEAN',
  branch: 'main',
  detached: false,
  entries: [],
  total: 0,
  truncated: false,
  unrepresentable: 0,
};

const diff: DiffSummary = {
  target: 'HEAD',
  files: [],
  total: 0,
  truncated: false,
  unrepresentable: 0,
};

const operation: Operation = {
  id: 'op-1' as Operation['id'],
  kind: 'git.status',
  actor: { type: 'USER' },
  status: 'RUNNING',
  createdAt: 1_800_000_000_000,
  startedAt: 1_800_000_000_000,
  cancellationRequested: false,
  correlationId: 'corr-1' as Operation['correlationId'],
};

function client(overrides: Record<string, () => unknown> = {}) {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set(
    'git_status',
    () => ({ status: 'ready', repository }) as GitStatusResponse,
  );
  transport.handlers.set('git_diff', () => ({ status: 'ready', diff }) as GitDiffResponse);
  for (const [command, handler] of Object.entries(overrides)) {
    transport.handlers.set(command, handler);
  }
  return { transport, core: createCoreClient(transport) };
}

/** The one call that is not `core_health`. */
function commandCall(transport: ReturnType<typeof createFakeTransport>) {
  return transport.calls.find((call) => call.command !== 'core_health');
}

describe('git client', () => {
  it('names every git command in the table, under the naming rule', () => {
    expect(COMMANDS['git.status']).toBe('git_status');
    expect(COMMANDS['git.diff']).toBe('git_diff');
  });

  it('reads the status of the open workspace', async () => {
    const { transport, core } = client();
    const response = await core.git.status({ workspaceId });

    expect(response.status).toBe('ready');
    if (response.status === 'ready') {
      expect(response.repository.branch).toBe('main');
    }
    expect(commandCall(transport)).toMatchObject({
      command: 'git_status',
      args: { request: { workspaceId } },
    });
  });

  it('passes paging and the ignored-files opt-in through untouched', async () => {
    const { transport, core } = client();
    await core.git.status({
      workspaceId,
      cursor: 'page-2',
      pageSize: 100,
      includeIgnored: true,
    });

    expect(commandCall(transport)?.args).toEqual({
      request: { workspaceId, cursor: 'page-2', pageSize: 100, includeIgnored: true },
    });
  });

  it('diffs one file against a named revision', async () => {
    const { transport, core } = client();
    const response = await core.git.diff({
      workspaceId,
      target: 'origin/main',
      path: 'src/main.rs',
    });

    expect(response.status).toBe('ready');
    expect(commandCall(transport)).toMatchObject({
      command: 'git_diff',
      args: { request: { workspaceId, target: 'origin/main', path: 'src/main.rs' } },
    });
  });

  // The arm the whole tagged union exists for. A workspace that is not a
  // repository is an ordinary answer, and a caller that treated it as a failure
  // would show a red envelope on every plain folder.
  it('reports a workspace that is not a repository as a value, not a rejection', async () => {
    const { core } = client({
      git_status: () => ({ status: 'notARepository' }) as GitStatusResponse,
      git_diff: () => ({ status: 'notARepository' }) as GitDiffResponse,
    });

    await expect(core.git.status({ workspaceId })).resolves.toEqual({
      status: 'notARepository',
    });
    await expect(core.git.diff({ workspaceId })).resolves.toEqual({
      status: 'notARepository',
    });
  });

  // The other arm: a read that outlived its budget hands back an operation the
  // caller already knows how to watch and cancel. No new event family.
  it('hands back an operation when the read outlives its budget', async () => {
    const { core } = client({
      git_status: () => ({ status: 'pending', operation }) as GitStatusResponse,
    });

    const response = await core.git.status({ workspaceId });
    expect(response.status).toBe('pending');
    if (response.status === 'pending') {
      expect(response.operation.kind).toBe('git.status');
      expect(response.operation.status).toBe('RUNNING');
    }
  });

  it('rejects with the error envelope when the workspace is not open', async () => {
    const notFound: CoreErrorShape = {
      code: 'NotFound',
      message: 'no open workspace w-9',
      retryable: false,
    };
    const { core } = client({
      git_status: () => {
        throw notFound;
      },
    });

    await expect(core.git.status({ workspaceId: 'w-9' as Workspace['id'] })).rejects.toMatchObject({
      code: 'NotFound',
    });
  });
});
