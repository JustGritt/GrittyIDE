/** @vitest-environment jsdom */
import type {
  GitStatusResponse,
  GitWriteResponse,
  HealthReport,
  RepositoryStatus,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';
import { PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useGit } from './use-git';

/**
 * The write half of `useGit` through the hook (T102-U), against a stubbed
 * transport — the pattern `use-explorer-force-replay.dom.test.tsx` set.
 *
 * The reducer tests prove the transitions. What only the hook can get wrong is
 * the wiring around them: which request the dirty-document replay sends, that a
 * write's response is converged on rather than followed by a `git.status`, and
 * that a draft outlives the panel unmounting.
 */

const WS = 'ws-1' as WorkspaceId;
const A = 'src/a.ts' as WorkspacePath;
const B = 'src/b.ts' as WorkspacePath;

const seam = vi.hoisted(() => {
  const handlers = new Map<string, (args?: Record<string, unknown>) => unknown>();
  return {
    handlers,
    transport: {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        const handler = handlers.get(command);
        if (!handler) {
          throw { code: 'Internal', message: `unstubbed command ${command}`, retryable: false };
        }
        return (await handler(args)) as T;
      },
      async listen() {
        return () => {};
      },
    },
  };
});

vi.mock('../../core/client', async () => {
  const { createCoreClient } = await import('@gritty/protocol');
  return { core: createCoreClient(seam.transport) };
});

const calls: { command: string; request: Record<string, unknown> }[] = [];

function stub(command: string, answer: (request: Record<string, unknown>) => unknown) {
  seam.handlers.set(command, (args) => {
    const request = (args?.request as Record<string, unknown>) ?? {};
    calls.push({ command, request });
    return answer(request);
  });
}

function repository(overrides: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    branch: 'main',
    detached: false,
    head: 'abcdef1234567890',
    state: 'CLEAN',
    entries: [
      { path: A, index: 'MODIFIED', worktree: 'UNMODIFIED', kind: 'FILE', conflicted: false },
      { path: B, index: 'UNMODIFIED', worktree: 'MODIFIED', kind: 'FILE', conflicted: false },
    ],
    total: 2,
    truncated: false,
    unrepresentable: 0,
    ...overrides,
  };
}

function ready(repo = repository()): GitWriteResponse {
  return { status: 'ready', result: { changed: [B], repository: repo, trashed: [] } };
}

function refuseOverDirtyDocuments(): never {
  throw {
    code: 'Conflict',
    message: '1 document(s) under the selection have unsaved changes',
    retryable: false,
    details: { workspaceId: WS, dirtyDocuments: [{ documentId: 'doc-1', path: B }] },
  };
}

let available = true;

beforeEach(() => {
  seam.handlers.clear();
  calls.length = 0;
  available = true;
  seam.handlers.set(
    'core_health',
    (): HealthReport => ({
      protocolVersion: PROTOCOL_VERSION,
      coreVersion: '0.1.0',
      os: 'macos',
      arch: 'aarch64',
      debug: true,
      shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
    }),
  );
  stub(
    'git_status',
    (): GitStatusResponse => ({
      status: 'ready',
      repository: repository(),
      write: available
        ? { available: true, version: 'git version 2.50.1' }
        : { available: false, reason: 'git was not found on PATH' },
    }),
  );
});

afterEach(cleanup);

async function mounted() {
  const rendered = renderHook(({ id }) => useGit(id), {
    initialProps: { id: WS as WorkspaceId | null },
  });
  await waitFor(() => expect(rendered.result.current.state.status.kind).toBe('ready'));
  return rendered;
}

function sent(command: string) {
  return calls.filter((call) => call.command === command);
}

describe('writes through useGit', () => {
  it('converges on the write response instead of reading the status behind it', async () => {
    stub('git_stage', () => ready());
    const rendered = await mounted();
    expect(sent('git_status')).toHaveLength(1);

    act(() => rendered.result.current.stage([B]));
    await waitFor(() => expect(rendered.result.current.state.write.kind).toBe('done'));

    expect(sent('git_stage')).toEqual([
      { command: 'git_stage', request: { workspaceId: WS, paths: [B] } },
    ]);
    expect(sent('git_status')).toHaveLength(1);
  });

  it('replays a discard refused over dirty documents with force and the same paths', async () => {
    let refuse = true;
    stub('git_discard', () => (refuse ? refuseOverDirtyDocuments() : ready()));
    const rendered = await mounted();

    act(() => rendered.result.current.discard([B]));
    await waitFor(() => expect(rendered.result.current.state.write.kind).toBe('failed'));
    expect(rendered.result.current.canDiscardAndRetry).toBe(true);
    expect(sent('git_discard')[0]?.request.force).toBeUndefined();

    refuse = false;
    act(() => rendered.result.current.discardAndRetry());
    await waitFor(() => expect(rendered.result.current.state.write.kind).toBe('done'));
    expect(sent('git_discard')).toHaveLength(2);
    expect(sent('git_discard')[1]?.request).toEqual({ workspaceId: WS, paths: [B], force: true });
  });

  it('offers no replay for a discard refused for any other reason, such as a held lock', async () => {
    stub('git_discard', () => {
      throw {
        code: 'Conflict',
        message: 'another git process is holding the index lock; try again in a moment',
        retryable: true,
      };
    });
    const rendered = await mounted();
    act(() => rendered.result.current.discard([B]));
    await waitFor(() => expect(rendered.result.current.state.write.kind).toBe('failed'));

    // A replay with `force` would push past a question nobody asked. Trying
    // again unchanged is the answer to this one, and it does not force.
    expect(rendered.result.current.canDiscardAndRetry).toBe(false);
    act(() => rendered.result.current.retryWrite());
    await waitFor(() => expect(sent('git_discard')).toHaveLength(2));
    expect(sent('git_discard')[1]?.request).toEqual({ workspaceId: WS, paths: [B] });
  });

  it('offers no replay for a Conflict that is not about unsaved work', async () => {
    stub('git_commit', () => {
      throw {
        code: 'Conflict',
        message: 'git commit refused the write',
        retryable: false,
        details: { subcommand: 'commit', exitCode: 1, stderr: 'lint failed' },
      };
    });
    const rendered = await mounted();
    act(() => rendered.result.current.changeDraft({ message: 'fix: it' }));
    act(() => rendered.result.current.commit());
    await waitFor(() => expect(rendered.result.current.state.write.kind).toBe('failed'));

    expect(rendered.result.current.canDiscardAndRetry).toBe(false);
    act(() => rendered.result.current.discardAndRetry());
    expect(sent('git_discard')).toHaveLength(0);
    // And the message the hook refused is still there to send again.
    expect(rendered.result.current.state.draft.message).toBe('fix: it');
  });

  it('sends amend and no-verify only when they were chosen', async () => {
    stub('git_commit', () => ({
      status: 'ready',
      result: { changed: [], repository: repository(), commit: 'f00ba1234567890', trashed: [] },
    }));
    const rendered = await mounted();
    act(() => rendered.result.current.changeDraft({ message: 'first' }));
    act(() => rendered.result.current.commit());
    await waitFor(() => expect(rendered.result.current.state.write.kind).toBe('done'));
    expect(sent('git_commit')[0]?.request).toEqual({ workspaceId: WS, message: 'first' });

    act(() =>
      rendered.result.current.changeDraft({ message: 'second', amend: true, noVerify: true }),
    );
    act(() => rendered.result.current.commit());
    await waitFor(() => expect(sent('git_commit')).toHaveLength(2));
    expect(sent('git_commit')[1]?.request).toEqual({
      workspaceId: WS,
      message: 'second',
      amend: true,
      noVerify: true,
    });
  });

  it('keeps a typed message when the panel unmounts and comes back, and forgets it once committed', async () => {
    stub('git_commit', () => ({
      status: 'ready',
      result: { changed: [], repository: repository(), commit: 'f00ba1234567890', trashed: [] },
    }));
    const first = await mounted();
    act(() => first.result.current.changeDraft({ message: 'half a thought' }));
    first.unmount();

    const second = await mounted();
    expect(second.result.current.state.draft.message).toBe('half a thought');

    act(() => second.result.current.commit());
    await waitFor(() => expect(second.result.current.state.write.kind).toBe('done'));
    expect(second.result.current.state.draft.message).toBe('');
    second.unmount();

    const third = await mounted();
    expect(third.result.current.state.draft.message).toBe('');
  });

  it('sends one commit for two clicks inside one frame', async () => {
    stub('git_commit', () => ({
      status: 'ready',
      result: { changed: [], repository: repository(), commit: 'f00ba1234567890', trashed: [] },
    }));
    const rendered = await mounted();
    act(() => rendered.result.current.changeDraft({ message: 'once', amend: true }));
    // Both calls read the same render, where nothing is in flight yet: a
    // double-click. With Amend ticked a second request would amend twice.
    act(() => {
      rendered.result.current.commit();
      rendered.result.current.commit();
    });
    await waitFor(() => expect(rendered.result.current.state.write.kind).toBe('done'));
    expect(sent('git_commit')).toHaveLength(1);
  });

  it('sends nothing while git cannot write here', async () => {
    available = false;
    stub('git_stage', () => ready());
    const rendered = await mounted();
    expect(rendered.result.current.state.availability?.available).toBe(false);
    act(() => rendered.result.current.stage([B]));
    expect(sent('git_stage')).toHaveLength(0);
    expect(rendered.result.current.state.write.kind).toBe('idle');
  });
});
