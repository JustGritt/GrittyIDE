import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { COMMANDS } from './commands';
import { createFakeTransport } from './testing/fake-transport';
import type {
  CoreErrorShape,
  GitWriteResponse,
  GitWriteResult,
  HealthReport,
  Operation,
  RepositoryStatus,
  Workspace,
  WorkspacePath,
} from './types';
import { PROTOCOL_VERSION } from './types';

// T102-C: the write half of the git wire (ADR-029). The reads are covered by
// `git-client.test.ts`; what is worth testing here is different, because these
// four *change the repository*:
//
//   - the request carries an explicit, non-empty pathspec — there is no
//     "stage everything" to accidentally call;
//   - a discard's `Conflict` over a dirty buffer, and the `force` replay that
//     answers it, are a two-step protocol the UI has to get right;
//   - a hook's refusal has to reach the caller as a value it can render, with
//     the hook's own words intact.

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const workspaceId = 'w-1' as Workspace['id'];
const notes = 'notes.md' as WorkspacePath;

const repository: RepositoryStatus = {
  state: 'CLEAN',
  branch: 'main',
  detached: false,
  entries: [],
  total: 0,
  truncated: false,
  unrepresentable: 0,
};

const result: GitWriteResult = {
  changed: [notes],
  repository,
  trashed: [],
};

function client(overrides: Record<string, (args?: unknown) => unknown> = {}) {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  for (const command of ['git_stage', 'git_unstage', 'git_discard', 'git_commit']) {
    transport.handlers.set(command, () => ({ status: 'ready', result }) as GitWriteResponse);
  }
  for (const [command, handler] of Object.entries(overrides)) {
    transport.handlers.set(command, handler);
  }
  return { core: createCoreClient(transport), transport };
}

describe('the git write wire', () => {
  it('maps every write to its snake_case Tauri command', () => {
    // Tauri command identifiers cannot contain dots, so this table is the
    // single source of the mapping and a typo in it is a runtime-only failure.
    expect(COMMANDS['git.stage']).toBe('git_stage');
    expect(COMMANDS['git.unstage']).toBe('git_unstage');
    expect(COMMANDS['git.discard']).toBe('git_discard');
    expect(COMMANDS['git.commit']).toBe('git_commit');
  });

  it('sends the paths the caller named, and nothing else', async () => {
    const seen: unknown[] = [];
    const { core } = client({
      git_stage: (args) => {
        seen.push(args);
        return { status: 'ready', result } as GitWriteResponse;
      },
    });

    await core.git.stage({ workspaceId, paths: [notes] });
    expect(seen).toEqual([{ request: { workspaceId, paths: [notes] } }]);
    // No `all`, no `-A`, no glob: there is nothing on this request that could
    // widen it beyond what the user selected.
    const [first] = seen as [{ request: Record<string, unknown> }];
    expect(Object.keys(first.request).sort()).toEqual(['paths', 'workspaceId']);
  });

  it('carries a commit message and its three opt-in flags', async () => {
    const seen: unknown[] = [];
    const { core } = client({
      git_commit: (args) => {
        seen.push(args);
        return { status: 'ready', result } as GitWriteResponse;
      },
    });

    await core.git.commit({ workspaceId, message: 'fix: the thing', noVerify: true });
    expect(seen).toEqual([{ request: { workspaceId, message: 'fix: the thing', noVerify: true } }]);
  });

  it('answers pending for a commit whose hook outlives the budget', async () => {
    // The arm that exists because a `pre-commit` hook can run a test suite.
    const operation = {
      id: 'op-1',
      kind: 'git.write',
      status: 'RUNNING',
    } as unknown as Operation;
    const { core } = client({
      git_commit: () => ({ status: 'pending', operation }) as GitWriteResponse,
    });

    const response = await core.git.commit({ workspaceId, message: 'slow' });
    expect(response.status).toBe('pending');
    if (response.status !== 'pending') throw new Error('expected pending');
    // A value the caller handles, not an error it catches — and the id is what
    // `operations.cancel` needs to kill the hook.
    expect(response.operation.id).toBe('op-1');
  });

  it('replays a refused discard with force, and only then', async () => {
    // The two-step the UI has to get right: `Conflict` names the dirty
    // documents, and the *same* command is re-sent with `force`.
    const requests: Record<string, unknown>[] = [];
    const dirty: CoreErrorShape = {
      code: 'Conflict',
      message: '1 document(s) under the selection have unsaved changes',
      retryable: false,
      details: {
        workspaceId,
        dirtyDocuments: [{ documentId: 'doc-1', path: notes }],
      },
    };
    const { core } = client({
      git_discard: (args) => {
        const request = (args as { request: Record<string, unknown> }).request;
        requests.push(request);
        if (request.force !== true) throw dirty;
        return { status: 'ready', result } as GitWriteResponse;
      },
    });

    // Typed as the refusal rather than as the union `catch` would infer: the
    // point of the assertion is that a `Conflict` carries the dirty list.
    let refused: CoreErrorShape | undefined;
    try {
      await core.git.discard({ workspaceId, paths: [notes] });
    } catch (error: unknown) {
      refused = error as CoreErrorShape;
    }
    if (!refused) throw new Error('the discard should have been refused');
    expect(refused.code).toBe('Conflict');
    // The UI needs the list, not just the count, to say *which* buffers.
    expect(refused.details).toMatchObject({
      dirtyDocuments: [{ documentId: 'doc-1', path: notes }],
    });

    const forced = await core.git.discard({ workspaceId, paths: [notes], force: true });
    expect(forced.status).toBe('ready');
    // The replay is the same paths, not a widened or re-derived selection.
    expect(requests).toEqual([
      { workspaceId, paths: [notes] },
      { workspaceId, paths: [notes], force: true },
    ]);
  });

  it("hands a refusing hook's own words to the caller", async () => {
    // The small cruelty this must not commit is losing the message; the
    // adjacent one is swallowing the reason. `stderr` is the hook's.
    const refused: CoreErrorShape = {
      code: 'Conflict',
      message: 'git commit refused the write',
      retryable: false,
      details: { subcommand: 'commit', exitCode: 1, stderr: 'lint failed: notes.md' },
    };
    const { core } = client({
      git_commit: () => {
        throw refused;
      },
    });

    await expect(core.git.commit({ workspaceId, message: 'nope' })).rejects.toMatchObject({
      code: 'Conflict',
      details: { stderr: 'lint failed: notes.md' },
    });
  });

  it('reports a held index lock as retryable', async () => {
    // The one failure here worth trying again unchanged — and named rather
    // than retried in a loop, which is how a hang gets built.
    const locked: CoreErrorShape = {
      code: 'Conflict',
      message: 'another git process is holding the index lock; try again in a moment',
      retryable: true,
    };
    const { core } = client({
      git_stage: () => {
        throw locked;
      },
    });

    await expect(core.git.stage({ workspaceId, paths: [notes] })).rejects.toMatchObject({
      code: 'Conflict',
      retryable: true,
    });
  });

  it('reports a repository-less workspace as a value, like the reads do', async () => {
    const { core } = client({
      git_stage: () => ({ status: 'notARepository' }) as GitWriteResponse,
    });
    const response = await core.git.stage({ workspaceId, paths: [notes] });
    expect(response.status).toBe('notARepository');
  });
});
