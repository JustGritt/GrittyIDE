import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { COMMANDS } from './commands';
import { createFakeTransport } from './testing/fake-transport';
import type {
  BlobContent,
  CommitPage,
  CoreErrorShape,
  DiffSummary,
  GitDiffResponse,
  GitLogResponse,
  GitShowResponse,
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

const commits: CommitPage = {
  start: 'HEAD',
  resolved: 'a'.repeat(40),
  commits: [
    {
      id: 'a'.repeat(40),
      shortId: 'aaaaaaa',
      parents: ['b'.repeat(40)],
      author: { name: 'Ada', email: 'ada@example.com', time: 1_800_000_000, offsetSeconds: 0 },
      committer: { name: 'Ada', email: 'ada@example.com', time: 1_800_000_000, offsetSeconds: 0 },
      subject: 'the change',
    },
  ],
  truncated: false,
};

const blob: BlobContent = {
  path: 'src/main.rs',
  revision: 'HEAD',
  resolved: 'a'.repeat(40),
  blob: 'c'.repeat(40),
  size: 13,
  text: 'fn main() {}\n',
  encoding: 'utf-8',
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
  transport.handlers.set('git_log', () => ({ status: 'ready', page: commits }) as GitLogResponse);
  transport.handlers.set('git_show', () => ({ status: 'ready', blob }) as GitShowResponse);
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
    expect(COMMANDS['git.log']).toBe('git_log');
    expect(COMMANDS['git.show']).toBe('git_show');
  });

  it('walks a page of history, and passes the cursor and path filter through', async () => {
    const { transport, core } = client();
    const response = await core.git.log({
      workspaceId,
      start: 'origin/main',
      path: 'src/main.rs',
      cursor: 'aaaa',
      pageSize: 20,
    });

    expect(response.status).toBe('ready');
    if (response.status === 'ready') {
      expect(response.page.commits[0]?.subject).toBe('the change');
      // A commit page deliberately has no `total`: counting every commit means
      // walking the whole history to render a number beside twenty rows.
      expect(response.page).not.toHaveProperty('total');
    }
    expect(commandCall(transport)).toMatchObject({
      command: 'git_log',
      args: {
        request: {
          workspaceId,
          start: 'origin/main',
          path: 'src/main.rs',
          cursor: 'aaaa',
          pageSize: 20,
        },
      },
    });
  });

  it('reads one path at one revision, which is the original side of a diff', async () => {
    const { transport, core } = client();
    const response = await core.git.show({
      workspaceId,
      path: 'src/main.rs',
      revision: 'HEAD~1',
    });

    expect(response.status).toBe('ready');
    if (response.status === 'ready') {
      expect(response.blob.text).toBe('fn main() {}\n');
      expect(response.blob.encoding).toBe('utf-8');
    }
    expect(commandCall(transport)).toMatchObject({
      command: 'git_show',
      args: { request: { workspaceId, path: 'src/main.rs', revision: 'HEAD~1' } },
    });
  });

  // Binary and oversized blobs are values with a reason, never errors — the
  // same bargain `notARepository` makes one arm over.
  it('reports a blob it cannot decode as a labelled value', async () => {
    const { core } = client({
      git_show: () =>
        ({
          status: 'ready',
          blob: { ...blob, text: undefined, noText: 'BINARY', encoding: 'binary' },
        }) as GitShowResponse,
    });

    const response = await core.git.show({ workspaceId, path: 'logo.png' });
    expect(response.status).toBe('ready');
    if (response.status === 'ready') {
      expect(response.blob.noText).toBe('BINARY');
      expect(response.blob.text).toBeUndefined();
      // The size survives, so a caller that gets no text still knows how big it was.
      expect(response.blob.size).toBe(13);
    }
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
      git_log: () => ({ status: 'notARepository' }) as GitLogResponse,
      git_show: () => ({ status: 'notARepository' }) as GitShowResponse,
    });

    await expect(core.git.log({ workspaceId })).resolves.toEqual({
      status: 'notARepository',
    });
    await expect(core.git.show({ workspaceId, path: 'a.txt' })).resolves.toEqual({
      status: 'notARepository',
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
