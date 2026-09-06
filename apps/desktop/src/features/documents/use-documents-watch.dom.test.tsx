/** @vitest-environment jsdom */
import type {
  ContentHash,
  Document,
  DocumentContent,
  DocumentId,
  EventEnvelope,
  HealthReport,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';
import { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useDocuments } from './use-documents';

/**
 * T070 §13: the watcher's batch runs the on-demand external-change check.
 *
 * This is the whole of "watcher-driven checks arrive with T070 through the
 * same path" (ADR-011): the branch calls `documents.status`, the same command
 * the window-focus listener calls, and nothing in the core's document module
 * changes. Two things are worth asserting and are asserted below — that a
 * batch reaches the check at all, and that it reaches it *once* per event,
 * because a per-path fan-out would be the unbounded forwarding rule 8 forbids
 * turned into a command storm instead of an event storm.
 */

const WORKSPACE = 'ws-1' as WorkspaceId;
const PATH = 'src/main.rs' as WorkspacePath;

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

function document(): Document {
  return {
    id: 'doc-1' as DocumentId,
    workspaceId: WORKSPACE,
    path: PATH,
    encoding: 'utf-8',
    languageId: 'rust',
    savedRevision: 1,
    savedHash: 'sha256:00' as ContentHash,
    currentVersion: 1,
    dirty: false,
    readOnly: false,
    binary: false,
    largeFileMode: false,
    externalModificationState: 'UNCHANGED',
    size: 12,
  };
}

/**
 * Never reset: the client is created once when its module is mocked in, and
 * its hub reports a `seq` that goes backwards as a discontinuity.
 */
let seq = 0;

/** Deliver one envelope the way the core's channel does. */
function emit(event: Omit<EventEnvelope, 'seq' | 'actor'>): void {
  seq += 1;
  const envelope = { seq, actor: { type: 'SYSTEM' }, ...event } as EventEnvelope;
  for (const listener of seam.listeners) listener(envelope);
}

function entriesChanged(directories: string[], overflowed = false) {
  return {
    name: 'WorkspaceEntriesChanged',
    workspaceId: WORKSPACE,
    payload: { workspaceId: WORKSPACE, directories, overflowed },
  } as Omit<EventEnvelope, 'seq' | 'actor'>;
}

describe('useDocuments under a watched workspace', () => {
  // Without this the previous test's hook stays mounted and subscribed, and
  // answers the next test's events.
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    // `seam.listeners` is deliberately not cleared: the mocked client opens
    // its channel once, when the module is first imported.
    // The client handshakes before its first command.
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
  });

  it('re-checks the disk when the watcher reports a change, once per batch', async () => {
    const status = vi.fn((): Document[] => [
      { ...document(), externalModificationState: 'MODIFIED_EXTERNALLY' },
    ]);
    seam.handlers.set(
      'documents_open',
      (): DocumentContent => ({ document: document(), text: 'x' }),
    );
    seam.handlers.set('documents_status', status);

    const { result } = renderHook(() => useDocuments());
    await act(async () => {
      await result.current.openFile(WORKSPACE, PATH);
    });
    expect(result.current.state.order).toHaveLength(1);

    await act(async () => {
      emit(entriesChanged(['src']));
    });
    await waitFor(() => expect(status).toHaveBeenCalledTimes(1));
    // The batch named one directory; the check is per batch, not per path.
    await act(async () => {
      emit(entriesChanged(['src', 'docs', 'assets']));
    });
    await waitFor(() => expect(status).toHaveBeenCalledTimes(2));

    await waitFor(() =>
      expect(
        result.current.state.tabs['doc-1' as DocumentId]?.document.externalModificationState,
      ).toBe('MODIFIED_EXTERNALLY'),
    );
  });

  it('does not call the core when nothing is open', async () => {
    const status = vi.fn((): Document[] => []);
    seam.handlers.set('documents_status', status);

    renderHook(() => useDocuments());
    await act(async () => {
      emit(entriesChanged(['src'], true));
    });
    // Nothing is open, so there is nothing whose disk state could have moved.
    expect(status).not.toHaveBeenCalled();
  });
});
