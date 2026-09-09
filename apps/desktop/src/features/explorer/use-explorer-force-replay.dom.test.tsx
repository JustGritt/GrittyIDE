/** @vitest-environment jsdom */
import type {
  FileOperationResponse,
  HealthReport,
  ListDirectoryResponse,
  Workspace,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';
import { PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useExplorer } from './use-explorer';

/**
 * EXPLORER-FORCE-REPLAY: the dirty-document `Conflict` replays *the* refused
 * action.
 *
 * The bug this stands against: `deleteAnyway` existed, was returned, and was
 * never called, while the one notice in the UI wired the open/close replay for
 * every `Conflict` — so a refused delete offered a button that replayed
 * something else, or (because the open/close slot was usually `null`) nothing
 * at all. Both halves were invisible, because nothing exercised the hook's
 * replay.
 *
 * Through the hook rather than the reducer, because the refusal lives in a
 * `useRef` and the replay is a callback: neither is reducer state, and the
 * whole defect was in the wiring between them.
 */

const WORKSPACE: Workspace = {
  id: 'ws-1' as WorkspaceId,
  root: '/tmp/gritty-fixture',
  name: 'gritty-fixture',
  openedAt: 1,
};

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
      // No event drives this test: the refusal lives in a ref, not in
      // reducer state, so nothing here has to be woken by the bus.
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

/** Every request the core was asked to run, in order. */
const calls: { command: string; request: Record<string, unknown> }[] = [];

function record(command: string, args?: Record<string, unknown>): Record<string, unknown> {
  const request = (args?.request as Record<string, unknown>) ?? {};
  calls.push({ command, request });
  return request;
}

function conflict(): never {
  throw {
    code: 'Conflict',
    message: '1 document(s) under the selection have unsaved changes',
    retryable: false,
    details: {
      workspaceId: WORKSPACE.id,
      dirtyDocuments: [{ documentId: 'doc-1', path: 'notes.md' }],
    },
  };
}

const OK: FileOperationResponse = {
  status: 'ready',
  result: { changed: [], moved: [] },
} as unknown as FileOperationResponse;

describe('the dirty-document Conflict replay', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    calls.length = 0;
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
    seam.handlers.set('workspace_current', (): Workspace => WORKSPACE);
    seam.handlers.set('workspace_list', (args): ListDirectoryResponse => {
      const request = args?.request as { path: string } | undefined;
      const path = (request?.path ?? '') as WorkspacePath;
      return {
        status: 'ready',
        page: { path, entries: [], total: 0, truncated: false },
      };
    });
  });

  /** A hook with its workspace already loaded. */
  async function opened() {
    const rendered = renderHook(() => useExplorer());
    await waitFor(() => expect(rendered.result.current.state.workspace).not.toBeNull());
    return rendered;
  }

  it('replays a refused delete with force, with the same paths and mode', async () => {
    let refuse = true;
    seam.handlers.set('files_delete', (args): FileOperationResponse => {
      record('files_delete', args);
      if (refuse) conflict();
      return OK;
    });

    const rendered = await opened();
    await act(async () => {
      await rendered.result.current.deleteEntries(['notes.md', 'src/a.ts'], 'TRASH');
    });
    expect(rendered.result.current.error?.code).toBe('Conflict');
    expect(calls).toHaveLength(1);
    expect(calls[0]?.request.force).toBeUndefined();

    refuse = false;
    await act(async () => {
      await rendered.result.current.discardAndRetry();
    });

    // The regression: before the fix this replayed the open/close slot, so no
    // second `files.delete` was ever issued.
    expect(calls).toHaveLength(2);
    expect(calls[1]).toEqual({
      command: 'files_delete',
      request: {
        workspaceId: WORKSPACE.id,
        paths: ['notes.md', 'src/a.ts'],
        mode: 'TRASH',
        force: true,
      },
    });
    expect(rendered.result.current.error).toBeNull();
  });

  it('replays a refused open with force, keeping the path', async () => {
    let refuse = true;
    seam.handlers.set('workspace_open', (args): Workspace => {
      record('workspace_open', args);
      if (refuse) conflict();
      return WORKSPACE;
    });

    const rendered = await opened();
    await act(async () => {
      await rendered.result.current.openFolder('/tmp/other');
    });
    expect(rendered.result.current.error?.code).toBe('Conflict');

    refuse = false;
    await act(async () => {
      await rendered.result.current.discardAndRetry();
    });

    expect(calls).toHaveLength(2);
    expect(calls[1]).toEqual({
      command: 'workspace_open',
      request: { path: '/tmp/other', force: true },
    });
  });

  it('replays the most recent refusal when an open and a delete were both refused', async () => {
    seam.handlers.set('workspace_open', (args): Workspace => {
      record('workspace_open', args);
      conflict();
    });
    let refuseDelete = true;
    seam.handlers.set('files_delete', (args): FileOperationResponse => {
      record('files_delete', args);
      if (refuseDelete) conflict();
      return OK;
    });

    const rendered = await opened();
    await act(async () => {
      await rendered.result.current.openFolder('/tmp/other');
    });
    await act(async () => {
      await rendered.result.current.deleteEntries(['notes.md'], 'PERMANENT');
    });
    expect(calls).toHaveLength(2);

    refuseDelete = false;
    await act(async () => {
      await rendered.result.current.discardAndRetry();
    });

    // The delete refused second, so the delete is what "discard and continue"
    // means. One slot is what makes that unambiguous.
    expect(calls).toHaveLength(3);
    expect(calls[2]?.command).toBe('files_delete');
    expect(calls[2]?.request.force).toBe(true);
  });

  it('does nothing when no action was refused', async () => {
    const rendered = await opened();
    await act(async () => {
      await rendered.result.current.discardAndRetry();
    });
    expect(calls).toHaveLength(0);
  });

  it('forgets the refusal once the action succeeds', async () => {
    seam.handlers.set('files_delete', (args): FileOperationResponse => {
      record('files_delete', args);
      return OK;
    });

    const rendered = await opened();
    await act(async () => {
      await rendered.result.current.deleteEntries(['notes.md'], 'TRASH');
    });
    expect(calls).toHaveLength(1);

    await act(async () => {
      await rendered.result.current.discardAndRetry();
    });
    // A succeeded delete is not a refusal to replay; replaying it would delete
    // a second time on a button the user reached for a different reason.
    expect(calls).toHaveLength(1);
  });
});
