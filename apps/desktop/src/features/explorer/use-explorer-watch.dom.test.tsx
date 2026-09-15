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

/**
 * FILES-PARTIAL, through the hook: a move that failed after doing part of its
 * work re-lists the loaded tree, because its error carries no `changed`; a
 * refusal before any work re-lists nothing.
 */
describe('useExplorer after a move that stopped part-way', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    listed.length = 0;
    seam.handlers.set('workspace_current', (): Workspace => WORKSPACE);
    seam.handlers.set('workspace_list', (args): ListDirectoryResponse => {
      const path = (args?.request as { path?: string } | undefined)?.path ?? '';
      listed.push(path);
      const entries = path === '' ? [dir('src'), dir('docs')] : [file(`${path}.txt`)];
      return {
        status: 'ready',
        page: { path: path as WorkspacePath, entries, total: entries.length, truncated: false },
      };
    });
  });

  async function openedTree() {
    const rendered = renderHook(() => useExplorer());
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

  it('re-lists every loaded directory once', async () => {
    seam.handlers.set('files_move', () => {
      throw {
        code: 'NotFound',
        message: 'b.txt does not exist (1 of 3 were already moved)',
        retryable: false,
        details: { completed: 1, requested: 3 },
      };
    });
    const rendered = await openedTree();

    await act(async () => {
      await rendered.result.current.transfer(['a.txt', 'b.txt', 'c.txt'], 'src', 'MOVE');
    });

    await waitFor(() => expect(timesListed('src')).toBe(1));
    expect(timesListed('')).toBe(1);
    expect(timesListed('docs')).toBe(0);
    expect(rendered.result.current.error?.code).toBe('NotFound');
  });

  it('re-lists nothing for a refusal before any work', async () => {
    seam.handlers.set('files_move', () => {
      throw { code: 'Conflict', message: 'src/a.txt already exists', retryable: false };
    });
    const rendered = await openedTree();

    await act(async () => {
      await rendered.result.current.transfer(['a.txt'], 'src', 'MOVE');
    });

    expect(rendered.result.current.error?.code).toBe('Conflict');
    expect(listed).toEqual([]);
  });
});

/**
 * EXPLORER-RELIST-QUEUE: stale directories re-list a few at a time.
 *
 * Listings are held until the test releases them, so "how many are in flight"
 * is a fact the test can read rather than a race it can lose.
 */
describe('useExplorer re-listing many stale directories', () => {
  afterEach(cleanup);

  const FOLDERS = Array.from({ length: 10 }, (_, index) => `d${index}`);
  /** Listings waiting for `releaseOne`, in the order they were asked for. */
  const held: { path: string; release: () => void }[] = [];
  let holding = false;
  let active = 0;
  let maxActive = 0;

  beforeEach(() => {
    seam.handlers.clear();
    listed.length = 0;
    held.length = 0;
    holding = false;
    active = 0;
    maxActive = 0;
    seam.handlers.set('workspace_current', (): Workspace => WORKSPACE);
    seam.handlers.set('workspace_list', (args) => {
      const path = (args?.request as { path?: string } | undefined)?.path ?? '';
      const entries = path === '' ? FOLDERS.map(dir) : [file(`${path}.txt`)];
      const response: ListDirectoryResponse = {
        status: 'ready',
        page: { path: path as WorkspacePath, entries, total: entries.length, truncated: false },
      };
      if (!holding) return response;
      listed.push(path);
      active += 1;
      maxActive = Math.max(maxActive, active);
      return new Promise<ListDirectoryResponse>((resolve) => {
        held.push({
          path,
          release: () => {
            active -= 1;
            resolve(response);
          },
        });
      });
    });
  });

  /** Root and all ten folders loaded; then every listing is held. */
  async function openedWideTree() {
    const rendered = renderHook(() => useExplorer());
    await waitFor(() => expect(rendered.result.current.state.nodes['']?.children).toHaveLength(10));
    for (const folder of FOLDERS) {
      const node = rendered.result.current.state.nodes[folder];
      if (!node) throw new Error(`no ${folder} node`);
      await act(async () => {
        rendered.result.current.toggle(node);
      });
    }
    await waitFor(() =>
      expect(FOLDERS.every((f) => rendered.result.current.state.nodes[f]?.children)).toBe(true),
    );
    holding = true;
    return rendered;
  }

  /** Release held listings one at a time until none are left or waiting. */
  async function drain() {
    for (let guard = 0; guard < 100; guard += 1) {
      const next = held.shift();
      if (!next) {
        // A freed slot starts the next listing on a later render.
        await act(async () => {
          await Promise.resolve();
        });
        if (held.length === 0) return;
        continue;
      }
      await act(async () => {
        next.release();
        await Promise.resolve();
      });
    }
    throw new Error('the queue never drained');
  }

  it('never runs more than four at once, and lists each directory once', async () => {
    await openedWideTree();

    await act(async () => {
      emitEntriesChanged([], true);
    });
    await waitFor(() => expect(held.length).toBeGreaterThan(0));
    expect(active).toBe(4);

    await drain();
    expect(maxActive).toBe(4);
    expect([...listed].sort()).toEqual(['', ...FOLDERS].sort());
  });

  it('lists the root before any folder under it, whatever order the batch named them in', async () => {
    await openedWideTree();

    await act(async () => {
      emitEntriesChanged(['d3', 'd1', '']);
    });
    await waitFor(() => expect(held.length).toBe(3));
    expect(listed[0]).toBe('');
    await drain();
  });

  it('lists a directory again when a batch names it while its listing is in flight', async () => {
    await openedWideTree();

    await act(async () => {
      emitEntriesChanged(['d3']);
    });
    await waitFor(() => expect(held.map((entry) => entry.path)).toEqual(['d3']));
    await act(async () => {
      emitEntriesChanged(['d3']);
    });
    // Still one in flight: the second naming waits for the first to settle.
    await act(async () => {
      await Promise.resolve();
    });
    expect(held.map((entry) => entry.path)).toEqual(['d3']);

    await drain();
    expect(listed).toEqual(['d3', 'd3']);
  });
});

/**
 * WATCH-DEGRADED, through the hook: the notice's state comes from the query on
 * open (for a renderer that subscribed after the event) and from the event.
 */
describe('useExplorer and a degraded watch', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    listed.length = 0;
    seam.handlers.set('workspace_current', (): Workspace => WORKSPACE);
    seam.handlers.set('workspace_list', (args): ListDirectoryResponse => {
      const path = (args?.request as { path?: string } | undefined)?.path ?? '';
      return {
        status: 'ready',
        page: { path: path as WorkspacePath, entries: [], total: 0, truncated: false },
      };
    });
  });

  function emitWatchStatus(degraded: string[]): void {
    seq += 1;
    const envelope = {
      seq,
      actor: { type: 'SYSTEM' },
      name: 'WorkspaceWatchStatusChanged',
      workspaceId: WORKSPACE.id,
      payload: { workspaceId: WORKSPACE.id, degraded },
    } as unknown as EventEnvelope;
    for (const listener of seam.listeners) listener(envelope);
  }

  it('asks on open, so a reason reported before it subscribed still shows', async () => {
    const asked: unknown[] = [];
    seam.handlers.set('workspace_watch_status', (args) => {
      asked.push(args?.request);
      return { workspaceId: WORKSPACE.id, degraded: ['BACKEND_UNAVAILABLE'] };
    });
    const rendered = renderHook(() => useExplorer());

    await waitFor(() =>
      expect(rendered.result.current.state.watchDegraded).toEqual(['BACKEND_UNAVAILABLE']),
    );
    expect(asked).toEqual([{ workspaceId: WORKSPACE.id }]);
    expect(rendered.result.current.error).toBeNull();
  });

  it('adds what the event reports, and a failed query is not an error', async () => {
    // Unstubbed: `workspace_watch_status` rejects.
    const rendered = renderHook(() => useExplorer());
    await waitFor(() => expect(rendered.result.current.state.workspace).not.toBeNull());

    await act(async () => {
      emitWatchStatus(['DIRECTORY_LIMIT']);
    });
    expect(rendered.result.current.state.watchDegraded).toEqual(['DIRECTORY_LIMIT']);
    expect(rendered.result.current.error).toBeNull();
  });
});
