import { describe, expect, it, vi } from 'vitest';
import { createCoreClient } from './client';
import { CoreError } from './errors';
import { createFakeTransport } from './testing/fake-transport';
import type {
  EventEnvelope,
  HealthReport,
  Operation,
  OperationId,
  OperationProgressPayload,
} from './types';
import { PROTOCOL_VERSION } from './types';

const OP_ID = 'op-1' as OperationId;

// Tracks the real version: a MAJOR bump must not make every unrelated
// client test fail the compatibility probe.
const [MAJOR = 0, MINOR = 0] = PROTOCOL_VERSION.split('.').map(Number);

function health(protocolVersion = PROTOCOL_VERSION): HealthReport {
  return {
    protocolVersion,
    coreVersion: '0.1.0',
    os: 'macos',
    arch: 'aarch64',
    debug: true,
    shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
  };
}

function operation(status: Operation['status'] = 'RUNNING'): Operation {
  return {
    id: OP_ID,
    kind: 'operations.demo',
    actor: { type: 'USER' },
    status,
    createdAt: 1,
    cancellationRequested: false,
    correlationId: 'corr-1' as Operation['correlationId'],
  };
}

function envelope(seq: number, name: EventEnvelope['name'] = 'OperationProgress'): EventEnvelope {
  if (name === 'OperationProgress') {
    const payload: OperationProgressPayload = { operationId: OP_ID, progress: { current: seq } };
    return { seq, actor: { type: 'USER' }, name, payload };
  }
  return {
    seq,
    actor: { type: 'USER' },
    name: 'OperationStarted',
    payload: { operation: operation() },
  };
}

function setup(
  options: {
    protocolVersion?: string;
    deferListen?: boolean;
    failListen?: number;
    listenBudgetMs?: number;
  } = {},
) {
  const transport = createFakeTransport({
    deferListen: options.deferListen,
    failListen: options.failListen,
  });
  transport.handlers.set('core_health', () => health(options.protocolVersion));
  transport.handlers.set('operations_list', () => [operation('COMPLETED')]);
  transport.handlers.set('operations_cancel', (args) => {
    const request = args?.request as { operationId: string };
    if (request.operationId !== OP_ID) {
      throw {
        code: 'NotFound',
        message: `operation ${request.operationId} does not exist`,
        retryable: false,
      };
    }
    return { ...operation(), cancellationRequested: true };
  });
  transport.handlers.set('operations_start_demo', () => operation('CREATED'));
  const diagnostics: Array<{ message: string; context: Record<string, unknown> }> = [];
  const client = createCoreClient(transport, {
    onDiagnostic: (message, context) => diagnostics.push({ message, context }),
    ...(options.listenBudgetMs === undefined ? {} : { listenBudgetMs: options.listenBudgetMs }),
  });
  return { transport, client, diagnostics };
}

/** Enough microtask turns for the health probe and its chained handlers. */
async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

describe('commands', () => {
  it('probes health once and reuses it for later calls', async () => {
    const { client, transport } = setup();
    const first = await client.health();
    const second = await client.health();
    expect(first).toEqual(health());
    expect(second).toBe(first);
    expect(transport.calls.filter((c) => c.command === 'core_health')).toHaveLength(1);
  });

  it('maps command names and wraps arguments in a request object', async () => {
    const { client, transport } = setup();
    await client.operations.startDemo({ steps: 3, stepMs: 10 });
    const cancelled = await client.operations.cancel(OP_ID);
    const listed = await client.operations.list();
    expect(cancelled.cancellationRequested).toBe(true);
    expect(listed[0]?.status).toBe('COMPLETED');
    expect(transport.calls.map((c) => c.command)).toEqual([
      'core_health',
      'operations_start_demo',
      'operations_cancel',
      'operations_list',
    ]);
    expect(transport.calls[1]?.args).toEqual({ request: { steps: 3, stepMs: 10 } });
    expect(transport.calls[2]?.args).toEqual({ request: { operationId: OP_ID } });
    expect(transport.calls[3]?.args).toBeUndefined();
  });

  it('rejects with the core error envelope', async () => {
    const { client } = setup();
    const error = await client.operations.cancel('missing' as OperationId).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(CoreError);
    expect((error as CoreError).code).toBe('NotFound');
    expect((error as CoreError).message).toContain('missing');
  });

  it('turns an unknown command rejection into an Internal CoreError, never a hang', async () => {
    const { client, transport } = setup();
    transport.handlers.delete('operations_list');
    const error = await client.operations.list().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(CoreError);
    expect((error as CoreError).code).toBe('Internal');
    expect((error as CoreError).details).toEqual({ raw: 'Command operations_list not found' });
  });
});

describe('protocol version guard', () => {
  it('refuses a core with a different major and keeps refusing', async () => {
    const { client, transport } = setup({ protocolVersion: `${MAJOR + 1}.0.0` });
    const first = await client.health().catch((e: unknown) => e);
    expect((first as CoreError).code).toBe('ProtocolMismatch');
    const second = await client.operations.list().catch((e: unknown) => e);
    expect((second as CoreError).code).toBe('ProtocolMismatch');
    expect(transport.calls).toHaveLength(1);
  });

  it('accepts a newer minor version', async () => {
    const { client } = setup({ protocolVersion: `${MAJOR}.${MINOR + 5}.0` });
    await expect(client.operations.list()).resolves.toHaveLength(1);
  });

  it('retries the probe after a transport failure', async () => {
    const { client, transport } = setup();
    const healthy = transport.handlers.get('core_health');
    transport.handlers.delete('core_health');
    const error = await client.health().catch((e: unknown) => e);
    expect((error as CoreError).code).toBe('Internal');
    if (healthy) transport.handlers.set('core_health', healthy);
    await expect(client.health()).resolves.toEqual(health());
  });
});

describe('events', () => {
  it('dispatches by name and shares one listener', async () => {
    const { client, transport } = setup();
    const progress = vi.fn();
    const started = vi.fn();
    const all = vi.fn();
    client.subscribe('OperationProgress', progress);
    client.subscribe('OperationStarted', started);
    client.subscribeAll(all);
    await Promise.resolve();
    transport.emit(envelope(1, 'OperationStarted'));
    transport.emit(envelope(2));
    expect(transport.listenCalls).toBe(1);
    expect(started).toHaveBeenCalledTimes(1);
    expect(progress).toHaveBeenCalledTimes(1);
    expect(progress.mock.calls[0]?.[0]).toMatchObject({ seq: 2, payload: { operationId: OP_ID } });
    expect(all).toHaveBeenCalledTimes(2);
  });

  it('stops delivering after unsubscribe and unlistens on dispose', async () => {
    const { client, transport } = setup();
    const handler = vi.fn();
    const unsubscribe = client.subscribe('OperationProgress', handler);
    await Promise.resolve();
    transport.emit(envelope(1));
    unsubscribe();
    transport.emit(envelope(2));
    expect(handler).toHaveBeenCalledTimes(1);
    client.dispose();
    await Promise.resolve();
    expect(transport.unlistenCalls).toBe(1);
    expect(() => client.subscribe('OperationProgress', handler)).toThrow(/disposed/);
  });

  it('is safe to unsubscribe and dispose before the listener is registered', async () => {
    const { client, transport } = setup({ deferListen: true });
    const handler = vi.fn();
    const unsubscribe = client.subscribe('OperationProgress', handler);
    unsubscribe();
    client.dispose();
    transport.resolveListen();
    await Promise.resolve();
    await Promise.resolve();
    transport.emit(envelope(1));
    expect(handler).not.toHaveBeenCalled();
    expect(transport.unlistenCalls).toBe(1);
  });

  it('reports sequence discontinuities and malformed payloads', async () => {
    const { client, transport, diagnostics } = setup();
    client.subscribeAll(() => undefined);
    await Promise.resolve();
    transport.emit(envelope(5));
    transport.emit(envelope(6));
    transport.emit(envelope(9));
    transport.emit({ not: 'an envelope' });
    expect(diagnostics.map((d) => d.message)).toEqual([
      // Emitting seq 5 first is itself a gap this client cannot have seen.
      'core events preceded this listener',
      'core event sequence discontinuity',
      'dropping malformed core event',
    ]);
    expect(diagnostics[0]?.context).toMatchObject({ preceding: 4, firstSeen: 5 });
    expect(diagnostics[1]?.context).toMatchObject({ expected: 7, received: 9 });
  });

  it('registers the listener before anything subscribes', () => {
    const { transport } = setup();
    // `start()` at construction: the round trip begins while the compatibility
    // probe is in flight, not when some hook happens to mount.
    expect(transport.listenCalls).toBe(1);
  });

  it('does not send a command until the listener is registered', async () => {
    // The budget is enormous on purpose. With a small one this test would pass
    // even if the gate were deleted — the timer would let the command through
    // and the assertion after `resolveListen` would still hold. Ordering is
    // the property under test, so nothing may rescue it but registration.
    const { client, transport } = setup({ deferListen: true, listenBudgetMs: 60_000 });
    const inFlight = client.operations.list();
    await flush();
    // The health probe is not gated — it causes no events, and gating it would
    // deadlock the very round trip readiness overlaps.
    expect(transport.calls.map((call) => call.command)).toEqual(['core_health']);

    transport.resolveListen();
    await expect(inFlight).resolves.toHaveLength(1);
    expect(transport.calls.map((call) => call.command)).toEqual(['core_health', 'operations_list']);
  });

  it('lets commands through when the channel never opens', async () => {
    const { client, transport, diagnostics } = setup({ deferListen: true, listenBudgetMs: 5 });
    // Never resolved: a listener that will not arrive must not become a window
    // that will not respond.
    await expect(client.operations.list()).resolves.toHaveLength(1);
    expect(transport.calls.map((call) => call.command)).toContain('operations_list');
    expect(diagnostics.map((d) => d.message)).toContain(
      'sending commands before the core event channel is listening',
    );
  });

  it('retries a refused registration instead of going deaf', async () => {
    const { client, transport, diagnostics } = setup({ failListen: 1 });
    const handler = vi.fn();
    client.subscribe('OperationProgress', handler);
    await new Promise((resolve) => setTimeout(resolve, 250));

    expect(diagnostics[0]?.message).toBe('failed to listen on the core event channel');
    expect(transport.listenCalls).toBe(2);
    // The subscription made before the failure is live on the retry: without
    // this, one refused registration silenced every hook for the session.
    transport.emit(envelope(1));
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('isolates a throwing handler from the others', async () => {
    const { client, transport, diagnostics } = setup();
    const good = vi.fn();
    client.subscribe('OperationProgress', () => {
      throw new Error('handler bug');
    });
    client.subscribe('OperationProgress', good);
    await Promise.resolve();
    transport.emit(envelope(1));
    expect(good).toHaveBeenCalledTimes(1);
    expect(diagnostics[0]?.message).toBe('core event handler threw');
  });
});
