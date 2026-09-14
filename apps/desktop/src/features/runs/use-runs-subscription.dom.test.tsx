/** @vitest-environment jsdom */
import type {
  AgentRun,
  AgentRunEventRecord,
  AgentRunId,
  EventEnvelope,
  HealthReport,
  Workspace,
  WorkspaceId,
} from '@gritty/protocol';
import { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useRuns } from './use-runs';

/**
 * RUNS-EVENT-FILTER: this feature subscribes by name, not to everything.
 *
 * `runsReducer` reads two event names and returns its input state for every
 * other one, so the previous `subscribeAll` here made every domain event in
 * the application — terminal output, watcher batches, agent timelines — a
 * dispatch into a reducer that had already decided to ignore it. The identity
 * bail-out kept that cheap, not free: React still re-enters the component
 * before bailing, once per event, at whatever rate the noisiest producer runs.
 *
 * The first test is the fix. The three after it are the risk the fix carries:
 * narrowing a subscription is how you silently stop receiving something, so
 * both names are exercised end to end through a real client over a fake
 * transport, rather than against a mocked `core`.
 */

const WORKSPACE = 'ws-1' as WorkspaceId;
const RUN = 'run-1' as AgentRunId;

const seam = vi.hoisted(() => {
  const handlers = new Map<string, (args?: Record<string, unknown>) => unknown>();
  const listeners = new Set<(payload: unknown) => void>();
  const byName: string[] = [];
  let all = 0;
  return {
    handlers,
    listeners,
    byName,
    subscribedToAll: () => all,
    countAll: () => {
      all += 1;
    },
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
  const client = createCoreClient(seam.transport);
  // Record what the hook asks for without changing what it gets.
  return {
    core: {
      ...client,
      subscribe: (name: string, handler: (event: EventEnvelope) => void) => {
        seam.byName.push(name);
        return (client.subscribe as (n: string, h: unknown) => () => void)(name, handler);
      },
      subscribeAll: (handler: (event: EventEnvelope) => void) => {
        seam.countAll();
        return client.subscribeAll(handler);
      },
    },
  };
});

const WS: Workspace = { id: WORKSPACE, root: '/ws', name: 'ws', openedAt: 1 };

function run(overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    id: RUN,
    workstreamId: 'wst-1' as AgentRun['workstreamId'],
    workspaceId: WORKSPACE,
    provider: 'FAKE',
    state: 'RUNNING',
    startedAt: 1_800_000_000_000,
    updatedAt: 1_800_000_000_000,
    cancellationRequested: false,
    correlationId: 'corr-1' as AgentRun['correlationId'],
    ...overrides,
  };
}

function record(seq: number): AgentRunEventRecord {
  return {
    seq,
    runId: RUN,
    at: 1_800_000_000_000 + seq,
    event: { type: 'FILE_WRITTEN', path: { display: `file-${seq}.ts` } },
  };
}

/** Never reset: the hub reports a `seq` that goes backwards as a discontinuity. */
let seq = 0;

function emit(event: Omit<EventEnvelope, 'seq' | 'actor'>): void {
  seq += 1;
  const envelope = { seq, actor: { type: 'SYSTEM' }, ...event } as EventEnvelope;
  for (const listener of seam.listeners) listener(envelope);
}

/** A busy event this feature has no interest in. */
function unrelated(directories: string[]): Omit<EventEnvelope, 'seq' | 'actor'> {
  return {
    name: 'WorkspaceEntriesChanged',
    workspaceId: WORKSPACE,
    payload: { workspaceId: WORKSPACE, directories, overflowed: false },
  } as Omit<EventEnvelope, 'seq' | 'actor'>;
}

describe('useRuns subscribes by name', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    seam.byName.length = 0;
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
    seam.handlers.set('runs_list', (): AgentRun[] => []);
  });

  it('asks for exactly the two events its reducer reads, and never for all of them', async () => {
    const { result } = renderHook(() => useRuns(WS));
    await waitFor(() => expect(result.current.loaded).toBe(true));

    expect(seam.byName).toEqual(['AgentRunChanged', 'AgentRunEventAppended']);
    expect(seam.subscribedToAll()).toBe(0);
  });

  it('does not re-render for events belonging to other features', async () => {
    let renders = 0;
    const { result } = renderHook(() => {
      renders += 1;
      return useRuns(WS);
    });
    await waitFor(() => expect(result.current.loaded).toBe(true));

    const before = renders;
    act(() => {
      // A watcher batch storm is the realistic shape of this: rule 8 bounds it
      // to ~40 batches, and under `subscribeAll` each one was a dispatch.
      for (let i = 0; i < 8; i += 1) emit(unrelated([`src/dir-${i}`]));
    });

    expect(renders).toBe(before);
    expect(result.current.runs).toEqual([]);
  });

  it('still receives AgentRunChanged', async () => {
    const { result } = renderHook(() => useRuns(WS));
    await waitFor(() => expect(result.current.loaded).toBe(true));

    act(() => {
      emit({ name: 'AgentRunChanged', payload: { run: run() } } as Omit<
        EventEnvelope,
        'seq' | 'actor'
      >);
    });

    expect(result.current.runs.map((r) => r.id)).toEqual([RUN]);
  });

  it('still receives AgentRunEventAppended', async () => {
    const { result } = renderHook(() => useRuns(WS));
    await waitFor(() => expect(result.current.loaded).toBe(true));

    act(() => {
      emit({ name: 'AgentRunChanged', payload: { run: run() } } as Omit<
        EventEnvelope,
        'seq' | 'actor'
      >);
      emit({
        name: 'AgentRunEventAppended',
        payload: { runId: RUN, events: [record(1)], dropped: 0 },
      } as Omit<EventEnvelope, 'seq' | 'actor'>);
    });

    expect(result.current.logFor(RUN).events.map((e) => e.seq)).toEqual([1]);
  });
});
