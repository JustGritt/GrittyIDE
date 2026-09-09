/** @vitest-environment jsdom */
import type {
  EventEnvelope,
  FindFilesResponse,
  FoundFiles,
  HealthReport,
  Operation,
  WorkspaceId,
} from '@gritty/protocol';
import { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { rowsOf } from './quick-open-model';
import { QUICK_OPEN_DEBOUNCE_MS, type QuickOpenDeps, useQuickOpen } from './use-quick-open';

/**
 * The controller's own behaviour (T130-V §4, §5, §6).
 *
 * Four things the pure modules cannot express and a string render cannot
 * either: that a keystroke does not become a walk, that a superseded answer is
 * dropped, that a *cancelled* one changes nothing at all, and that the
 * `pending` arm completes — including when it completes before anyone is
 * listening, which is the race that would otherwise hang the bar in `loading`.
 */

const WS = 'ws-1' as WorkspaceId;

const seam = vi.hoisted(() => {
  const handlers = new Map<string, (args?: Record<string, unknown>) => unknown>();
  const listeners = new Set<(payload: unknown) => void>();
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  return {
    handlers,
    listeners,
    calls,
    transport: {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        calls.push({ command, args });
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

function emit(name: string, payload: unknown): void {
  seq += 1;
  const envelope = { seq, actor: { type: 'SYSTEM' }, name, payload } as unknown as EventEnvelope;
  for (const listener of seam.listeners) listener(envelope);
}

function operation(id: string): Operation {
  return {
    id: id as Operation['id'],
    kind: 'workspace.findFiles' as Operation['kind'],
    actor: { type: 'USER' },
    workspaceId: WS,
    status: 'RUNNING' as Operation['status'],
    createdAt: 1 as Operation['createdAt'],
    cancellationRequested: false,
    correlationId: 'c1' as Operation['correlationId'],
  };
}

function found(...paths: string[]): FoundFiles {
  return {
    matches: paths.map((path) => ({ path, score: 100, ranges: [[0, 1]] })),
    truncated: false,
  };
}

function ready(...paths: string[]): FindFilesResponse {
  return { status: 'ready', matches: found(...paths).matches, truncated: false };
}

function deps(overrides: Partial<QuickOpenDeps> = {}): QuickOpenDeps {
  return {
    workspaceId: WS,
    hasWorkspace: true,
    document: null,
    terminalVisible: false,
    openFolder: () => {},
    saveDocument: () => {},
    toggleTerminal: () => {},
    openSettings: () => {},
    openFile: () => {},
    revealLine: () => {},
    onDismiss: () => {},
    ...overrides,
  };
}

function findCalls(): Record<string, unknown>[] {
  return seam.calls
    .filter((call) => call.command === 'workspace_find_files')
    .map((call) => (call.args?.request ?? {}) as Record<string, unknown>);
}

describe('the files query', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    seam.calls.length = 0;
    vi.useRealTimers();
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
    seam.handlers.set('workspace_find_files', () => ready('src/editor-pane.tsx'));
    seam.handlers.set('operations_cancel', (args) => {
      const request = args?.request as { operationId: string };
      return operation(request.operationId);
    });
  });

  it('costs one walk for a word typed, not one per keystroke', async () => {
    const { result } = renderHook(() => useQuickOpen(deps()));
    for (const raw of ['e', 'ep', 'epn']) {
      act(() => result.current.setRaw(raw));
    }
    await waitFor(() => expect(findCalls().length).toBeGreaterThan(0));
    // The debounce is the whole reason a per-query whole-tree walk is
    // affordable: three keystrokes must not be three walks.
    expect(findCalls()).toHaveLength(1);
    expect(findCalls()[0]?.query).toBe('epn');
  });

  it('draws the debounce window as a wait, never as "no match"', async () => {
    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('epn'));
    // Before the timer fires there is a term and no answer. Anything but
    // `loading` here is a lie the user watches being corrected.
    expect(result.current.list.kind).toBe('loading');
    await waitFor(() => expect(result.current.list.kind).toBe('results'));
  });

  it('drops a superseded answer rather than rendering it under the new term', async () => {
    // The first query is slow; the second is not. Without the sequence guard
    // the slow answer lands last and the rows disagree with the field.
    let releaseFirst: (() => void) | undefined;
    seam.handlers.set('workspace_find_files', (args) => {
      const request = args?.request as { query: string };
      if (request.query === 'slow') {
        return new Promise<FindFilesResponse>((resolve) => {
          releaseFirst = () => resolve(ready('src/slow.ts'));
        });
      }
      return ready('src/fast.ts');
    });

    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('slow'));
    await waitFor(() => expect(releaseFirst).toBeDefined());
    act(() => result.current.setRaw('fast'));
    await waitFor(() =>
      expect(rowsOf(result.current.list).map((row) => row.id)).toEqual(['file:0']),
    );
    const fastRows = rowsOf(result.current.list);
    expect(fastRows[0]).toMatchObject({ path: 'src/fast.ts' });

    await act(async () => {
      releaseFirst?.();
      await Promise.resolve();
    });
    // The slow answer arrived last and changed nothing.
    expect(rowsOf(result.current.list)[0]).toMatchObject({ path: 'src/fast.ts' });
  });

  it('leaves the rows alone when a query is cancelled, because that is not a failure', async () => {
    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('epn'));
    await waitFor(() => expect(result.current.list.kind).toBe('results'));

    // The next query is cancelled rather than answered.
    seam.handlers.set('workspace_find_files', () => {
      throw { code: 'Cancelled', message: 'file search cancelled', retryable: false };
    });
    act(() => result.current.setRaw('epnx'));
    await new Promise((resolve) => setTimeout(resolve, QUICK_OPEN_DEBOUNCE_MS * 3));
    // Not an error row. The rows are simply the ones from before.
    expect(result.current.list.kind).not.toBe('error');
  });

  it('surfaces a real failure as an error row, with the envelope', async () => {
    seam.handlers.set('workspace_find_files', () => {
      throw { code: 'Io', message: 'the workspace root went away', retryable: false };
    });
    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('epn'));
    await waitFor(() => expect(result.current.list.kind).toBe('error'));
    expect(result.current.list.kind === 'error' && result.current.list.error.code).toBe('Io');
  });

  it('asks nothing at all with no workspace open', async () => {
    const { result } = renderHook(() =>
      useQuickOpen(deps({ workspaceId: null, hasWorkspace: false })),
    );
    act(() => result.current.setRaw('epn'));
    await new Promise((resolve) => setTimeout(resolve, QUICK_OPEN_DEBOUNCE_MS * 3));
    expect(findCalls()).toHaveLength(0);
    expect(result.current.list).toMatchObject({ kind: 'empty', reason: 'noWorkspace' });
  });

  it('asks nothing for the other three modes', async () => {
    const { result } = renderHook(() => useQuickOpen(deps()));
    for (const raw of ['>save', ':12', '@sym']) {
      act(() => result.current.setRaw(raw));
      await new Promise((resolve) => setTimeout(resolve, QUICK_OPEN_DEBOUNCE_MS * 2));
    }
    expect(findCalls()).toHaveLength(0);
  });
});

describe('the pending arm', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    seam.calls.length = 0;
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
    seam.handlers.set('operations_cancel', (args) => {
      const request = args?.request as { operationId: string };
      return operation(request.operationId);
    });
  });

  it('completes through OperationCompleted, which is the normal answer at scale', async () => {
    seam.handlers.set(
      'workspace_find_files',
      (): FindFilesResponse => ({ status: 'pending', operation: operation('op-1') }),
    );
    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('epn'));
    await waitFor(() => expect(findCalls()).toHaveLength(1));
    expect(result.current.list.kind).toBe('loading');

    act(() =>
      emit('OperationCompleted', {
        operation: { ...operation('op-1'), status: 'COMPLETED' },
        result: found('src/editor-pane.tsx'),
      }),
    );
    await waitFor(() => expect(result.current.list.kind).toBe('results'));
    expect(rowsOf(result.current.list)[0]).toMatchObject({ path: 'src/editor-pane.tsx' });
  });

  it('does not hang when the operation completed before anyone was listening', async () => {
    // The race worth closing: a walk that finishes a millisecond after its
    // budget emits its completion while the `pending` response is still
    // crossing the IPC. Without the buffer the bar stays in `loading` for ever.
    seam.handlers.set('workspace_find_files', (): FindFilesResponse => {
      emit('OperationCompleted', {
        operation: { ...operation('op-early'), status: 'COMPLETED' },
        result: found('src/early.ts'),
      });
      return { status: 'pending', operation: operation('op-early') };
    });
    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('epn'));
    await waitFor(() => expect(result.current.list.kind).toBe('results'));
    expect(rowsOf(result.current.list)[0]).toMatchObject({ path: 'src/early.ts' });
  });

  it('reports a failed operation as an error row', async () => {
    seam.handlers.set(
      'workspace_find_files',
      (): FindFilesResponse => ({ status: 'pending', operation: operation('op-fail') }),
    );
    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('epn'));
    await waitFor(() => expect(findCalls()).toHaveLength(1));

    act(() =>
      emit('OperationFailed', {
        operation: { ...operation('op-fail'), status: 'FAILED' },
        error: { code: 'Io', message: 'the root went away', retryable: false },
      }),
    );
    await waitFor(() => expect(result.current.list.kind).toBe('error'));
  });

  it('cancels the operation when the query is superseded', async () => {
    seam.handlers.set('workspace_find_files', (args): FindFilesResponse => {
      const request = args?.request as { query: string };
      return { status: 'pending', operation: operation(`op-${request.query}`) };
    });
    const { result } = renderHook(() => useQuickOpen(deps()));
    act(() => result.current.setRaw('first'));
    await waitFor(() => expect(findCalls()).toHaveLength(1));
    act(() => result.current.setRaw('second'));
    await waitFor(() => expect(findCalls()).toHaveLength(2));

    // One cancellation surface, and it is `operations.cancel` — there is no
    // `findFiles.cancel`.
    await waitFor(() =>
      expect(
        seam.calls.filter((call) => call.command === 'operations_cancel').map((call) => call.args),
      ).toContainEqual({ request: { operationId: 'op-first' } }),
    );
  });
});
