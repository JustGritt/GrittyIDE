/** @vitest-environment jsdom */
import type {
  DirEntry,
  EventEnvelope,
  HealthReport,
  ListDirectoryResponse,
  Workspace,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';
import { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useExplorer } from './use-explorer';

/**
 * T070 §12, through the hook rather than the reducer: a watcher batch actually
 * re-lists, with the same `workspace.list` a manual reload makes.
 *
 * The reducer is pure and names the stale nodes; this is the half that runs
 * them, and the acceptance criterion it stands for is "create/delete inside an
 * expanded, loaded directory updates it unasked, and the same inside a
 * collapsed one does nothing". Counting the calls per path is what makes both
 * halves of that sentence assertable.
 */

const WORKSPACE: Workspace = {
  id: 'ws-1' as WorkspaceId,
  root: '/tmp/gritty-fixture',
  name: 'gritty-fixture',
  openedAt: 1,
};

function dir(name: string): DirEntry {
  return { name, kind: 'dir', hidden: false, ignoredHint: false };
}

function file(name: string): DirEntry {
  return { name, kind: 'file', hidden: false, ignoredHint: false };
}

const seam = vi.hoisted(() => {
  const handlers = new Map<string, (args?: Record<string, unknown>) => unknown>();
  const listeners = new Set<(payload: unknown) => void>();
  return {
    handlers,
    listeners,
    transport: {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        const handler = handlers.get(command);
        if (!handler) {
          throw { code: 'Internal', message: `unstubbed command ${command}`, retryable: false };
        }
        return (await handler(args)) as T;
      },
      async listen(channel: string, handler: (payload: unknown) => void) {
        if (channel === CORE_EVENT_CHANNEL) listeners.add(handler);
        return () => {
          listeners.delete(handler);
        };
      },
    },
  };
});

vi.mock('../../core/client', async () => {
  const { createCoreClient } = await import('@gritty/protocol');
  return { core: createCoreClient(seam.transport) };
});

/** Never reset: the mocked client opens its channel once, at import. */
let seq = 0;

function emitEntriesChanged(directories: string[], overflowed = false): void {
  seq += 1;
  const envelope = {
    seq,
    actor: { type: 'SYSTEM' },
    name: 'WorkspaceEntriesChanged',
    workspaceId: WORKSPACE.id,
    payload: { workspaceId: WORKSPACE.id, directories, overflowed },
  } as unknown as EventEnvelope;
  for (const listener of seam.listeners) listener(envelope);
}

/** One `workspace.list` call per path, counted. */
const listed: string[] = [];

function timesListed(path: string): number {
  return listed.filter((candidate) => candidate === path).length;
}

describe('useExplorer under a watched workspace', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    listed.length = 0;
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
      const path = request?.path ?? '';
      listed.push(path);
      const entries = path === '' ? [dir('src'), dir('docs')] : [file(`${path}.txt`)];
      return {
        status: 'ready',
        page: { path: path as WorkspacePath, entries, total: entries.length, truncated: false },
      };
    });
  });

  /** Root loaded, `src` expanded and loaded, `docs` never opened. */
  async function openedTree() {
    const rendered = renderHook(() => useExplorer());
    // `not.toBeNull()` alone would pass on an absent node, which is how the
    // first draft of this file asserted nothing at all.
    await waitFor(() =>
      expect(rendered.result.current.state.nodes['']?.children).toEqual(['src', 'docs']),
    );
    const src = rendered.result.current.state.nodes.src;
    if (!src) throw new Error('the root listing produced no `src` node');
    await act(async () => {
      rendered.result.current.toggle(src);
    });
    await waitFor(() =>
      expect(rendered.result.current.state.nodes.src?.children).toEqual(['src/src.txt']),
    );
    listed.length = 0;
    return rendered;
  }

  it('re-lists a loaded directory a batch names, and leaves an unopened one alone', async () => {
    const rendered = await openedTree();

    await act(async () => {
      emitEntriesChanged(['src', 'docs']);
    });
    await waitFor(() => expect(timesListed('src')).toBe(1));
    // `docs` was never expanded: nothing about it is on screen to correct.
    expect(timesListed('docs')).toBe(0);
    expect(timesListed('')).toBe(0);
    expect(rendered.result.current.state.stale).toEqual([]);
  });

  it('re-lists every loaded node when the batch overflowed', async () => {
    await openedTree();

    await act(async () => {
      emitEntriesChanged([], true);
    });
    await waitFor(() => expect(timesListed('src')).toBe(1));
    expect(timesListed('')).toBe(1);
    // Bounded by what the user expanded, never by the size of the storm.
    expect(timesListed('docs')).toBe(0);
  });

  it('lists each stale path once, however many batches named it', async () => {
    await openedTree();

    await act(async () => {
      emitEntriesChanged(['src']);
      emitEntriesChanged(['src']);
      emitEntriesChanged(['src']);
    });
    await waitFor(() => expect(timesListed('src')).toBeGreaterThan(0));
    expect(timesListed('src')).toBe(1);
  });
});
