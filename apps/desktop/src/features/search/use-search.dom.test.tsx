/** @vitest-environment jsdom */
import type {
  EventEnvelope,
  HealthReport,
  Operation,
  OperationId,
  SearchMatch,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';
import { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SEARCH_KIND } from './search-reducer';
import { SEARCH_DEBOUNCE_MS, useSearch } from './use-search';

/**
 * `useSearch` through a stubbed transport (T080-U), the pattern
 * `use-explorer-watch.dom.test.tsx` set.
 *
 * The reducer tests prove what a run believes. What only the hook can get
 * wrong is what it *sends*: one query per pause rather than per keystroke, a
 * cancel for the search it superseded, a Stop pressed before the operation
 * existed, and a workspace change — and that a response racing its own events
 * still ends with the matches on screen.
 */

const WS = 'ws-1' as WorkspaceId;
const WS2 = 'ws-2' as WorkspaceId;

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

function emit(body: Omit<EventEnvelope, 'seq' | 'actor'>): void {
  seq += 1;
  const envelope = { seq, actor: { type: 'SYSTEM' }, ...body } as EventEnvelope;
  for (const listener of seam.listeners) listener(envelope);
}

function operation(id: string): Operation {
  return {
    id: id as OperationId,
    kind: SEARCH_KIND,
    actor: { type: 'USER' },
    workspaceId: WS,
    status: 'RUNNING',
    createdAt: 1,
    cancellationRequested: false,
    correlationId: 'corr' as Operation['correlationId'],
  };
}

function match(path: string, line: number): SearchMatch {
  return {
    path: path as WorkspacePath,
    kind: 'content',
    line,
    column: 1,
    snippet: 'needle',
    truncated: false,
    matchStart: 0,
    matchLength: 6,
  };
}

function batch(id: string, matches: SearchMatch[]): void {
  emit({
    name: 'SearchMatchesFound',
    payload: { operationId: id as OperationId, matches, truncatedMatches: 0 },
  });
}

function cancelledEvent(id: string): void {
  emit({ name: 'OperationCancelled', payload: { operation: operation(id) } });
}

/** Every `search.query` request, and a way to answer each one when the test says so. */
const queries: {
  request: Record<string, unknown>;
  answer: (id: string) => void;
  refuse: (error: unknown) => void;
}[] = [];
const cancels: string[] = [];

beforeEach(() => {
  seam.handlers.clear();
  queries.length = 0;
  cancels.length = 0;
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
  seam.handlers.set('search_query', (args) => {
    const request = (args?.request as Record<string, unknown>) ?? {};
    return new Promise((resolve, reject) => {
      queries.push({ request, answer: (id) => resolve(operation(id)), refuse: reject });
    });
  });
  seam.handlers.set('operations_cancel', (args) => {
    const request = args?.request as { operationId: string };
    cancels.push(request.operationId);
    return { ...operation(request.operationId), cancellationRequested: true };
  });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function answered(index: number, id: string) {
  await waitFor(() => expect(queries.length).toBeGreaterThan(index));
  await act(async () => {
    queries[index]?.answer(id);
  });
}

describe('typing', () => {
  it('sends one query for a pause, with the text the field has when it fires', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { result } = renderHook(() => useSearch(WS));
    act(() => result.current.setQuery('n'));
    act(() => result.current.setQuery('nee'));
    act(() => result.current.setQuery('needle'));
    await act(async () => {
      vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS - 10);
    });
    expect(queries).toHaveLength(0);
    await act(async () => {
      vi.advanceTimersByTime(20);
    });
    await waitFor(() => expect(queries).toHaveLength(1));
    expect(queries[0]?.request).toMatchObject({
      workspaceId: WS,
      query: 'needle',
      options: { caseSensitive: false, wholeWord: false, useRegex: false, scope: 'both' },
    });
  });

  it('clears at once when the field is emptied, cancelling what was running', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('needle');
      result.current.submit();
    });
    await answered(0, 'op-1');
    act(() => batch('op-1', [match('a.ts', 1)]));
    await waitFor(() => expect(result.current.run?.matchCount).toBe(1));
    act(() => result.current.setQuery(''));
    expect(result.current.run).toBeNull();
    await waitFor(() => expect(cancels).toEqual(['op-1']));
    expect(queries).toHaveLength(1);
  });

  it('re-runs at once when a toggle changes', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => result.current.setQuery('needle'));
    act(() => result.current.setToggle('useRegex', true));
    await waitFor(() => expect(queries).toHaveLength(1));
    expect(queries[0]?.request.options).toMatchObject({ useRegex: true });
  });
});

describe('a run', () => {
  it('streams, completes, and stops forwarding once it has ended', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('needle');
      result.current.submit();
    });
    await answered(0, 'op-1');
    act(() => batch('op-1', [match('a.ts', 1), match('b.ts', 2)]));
    await waitFor(() => expect(result.current.run?.files).toHaveLength(2));
    act(() =>
      emit({
        name: 'OperationCompleted',
        payload: { operation: { ...operation('op-1'), status: 'COMPLETED' } },
      }),
    );
    await waitFor(() => expect(result.current.run?.phase).toBe('completed'));
    const settled = result.current.run;
    act(() => batch('op-1', [match('late.ts', 1)]));
    expect(result.current.run).toBe(settled);
  });

  it('keeps the matches that arrived before the response (ADR-009)', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('needle');
      result.current.submit();
    });
    await waitFor(() => expect(queries).toHaveLength(1));
    act(() => batch('op-1', [match('early.ts', 1)]));
    expect(result.current.run?.files).toEqual([]);
    await answered(0, 'op-1');
    await waitFor(() =>
      expect(result.current.run?.files.map((file) => file.path)).toEqual(['early.ts']),
    );
  });

  it('cancels the search a new one supersedes, and never shows its late matches', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('first');
      result.current.submit();
    });
    await answered(0, 'op-1');
    act(() => {
      result.current.setQuery('second');
      result.current.submit();
    });
    await waitFor(() => expect(cancels).toEqual(['op-1']));
    await answered(1, 'op-2');
    act(() => batch('op-1', [match('stale.ts', 1)]));
    act(() => batch('op-2', [match('fresh.ts', 1)]));
    await waitFor(() =>
      expect(result.current.run?.files.map((file) => file.path)).toEqual(['fresh.ts']),
    );
    expect(result.current.run?.query).toBe('second');
  });

  it('cancels an operation whose response arrives after it was superseded', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('first');
      result.current.submit();
    });
    await waitFor(() => expect(queries).toHaveLength(1));
    act(() => {
      result.current.setQuery('second');
      result.current.submit();
    });
    await waitFor(() => expect(queries).toHaveLength(2));
    // The first answer lands late: nobody wants it, so it is cancelled.
    await answered(0, 'op-1');
    await waitFor(() => expect(cancels).toEqual(['op-1']));
    await answered(1, 'op-2');
    await waitFor(() => expect(result.current.run?.operationId).toBe('op-2'));
  });

  it('shows a refusal, such as a pattern that does not compile', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('(');
      result.current.submit();
    });
    await waitFor(() => expect(queries).toHaveLength(1));
    await act(async () => {
      queries[0]?.refuse({ code: 'InvalidInput', message: 'regex parse error', retryable: false });
    });
    await waitFor(() => expect(result.current.run?.phase).toBe('failed'));
    expect(result.current.run?.error?.code).toBe('InvalidInput');
    expect(result.current.run?.operationId).toBeNull();
  });
});

describe('stop', () => {
  it('cancels the running operation, and the phase follows the event', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('needle');
      result.current.submit();
    });
    await answered(0, 'op-1');
    act(() => result.current.stop());
    await waitFor(() => expect(cancels).toEqual(['op-1']));
    expect(result.current.run?.stopRequested).toBe(true);
    act(() => cancelledEvent('op-1'));
    await waitFor(() => expect(result.current.run?.phase).toBe('cancelled'));
  });

  it('pressed before the response, cancels as soon as the operation exists', async () => {
    const { result } = renderHook(() => useSearch(WS));
    act(() => {
      result.current.setQuery('needle');
      result.current.submit();
    });
    await waitFor(() => expect(queries).toHaveLength(1));
    act(() => result.current.stop());
    expect(cancels).toEqual([]);
    await answered(0, 'op-1');
    await waitFor(() => expect(cancels).toEqual(['op-1']));
  });
});

describe('the workspace', () => {
  it('changing cancels the run and clears the results, keeping the text', async () => {
    const { result, rerender } = renderHook(({ workspace }) => useSearch(workspace), {
      initialProps: { workspace: WS as WorkspaceId | null },
    });
    act(() => {
      result.current.setQuery('needle');
      result.current.submit();
    });
    await answered(0, 'op-1');
    rerender({ workspace: WS2 });
    await waitFor(() => expect(cancels).toEqual(['op-1']));
    expect(result.current.run).toBeNull();
    expect(result.current.query).toBe('needle');
    act(() => batch('op-1', [match('old.ts', 1)]));
    expect(result.current.run).toBeNull();
  });

  it('closing leaves nothing to search', async () => {
    const { result, rerender } = renderHook(({ workspace }) => useSearch(workspace), {
      initialProps: { workspace: null as WorkspaceId | null },
    });
    act(() => {
      result.current.setQuery('needle');
      result.current.submit();
    });
    expect(queries).toHaveLength(0);
    expect(result.current.run).toBeNull();
    rerender({ workspace: WS });
    expect(result.current.workspaceId).toBe(WS);
  });
});
