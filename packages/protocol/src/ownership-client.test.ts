import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type {
  HealthReport,
  OwnershipAcquisition,
  OwnershipConflict,
  OwnershipLease,
  OwnershipMap,
} from './types';
import { PROTOCOL_VERSION } from './types';

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const lease: OwnershipLease = {
  id: 'l-1' as OwnershipLease['id'],
  workspaceId: 'w-1' as OwnershipLease['workspaceId'],
  workstreamId: 'ws-1' as OwnershipLease['workstreamId'],
  target: { type: 'DIRECTORY', path: 'src/auth' },
  mode: 'WRITE',
  acquiredAt: 1_800_000_000_000,
  state: 'ACTIVE',
  stale: false,
};

const conflict: OwnershipConflict = {
  id: 'c-1' as OwnershipConflict['id'],
  workspaceId: 'w-1' as OwnershipConflict['workspaceId'],
  requestedBy: 'ws-2' as OwnershipConflict['requestedBy'],
  ownedBy: 'ws-1' as OwnershipConflict['ownedBy'],
  holdingLeaseId: lease.id,
  target: { type: 'FILE', path: 'src/auth/session.ts' },
  createdAt: 1_800_000_001_000,
  state: 'WAITING',
};

const map: OwnershipMap = { leases: [lease], conflicts: [conflict] };

function client() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set('ownership_list', () => map);
  transport.handlers.set('ownership_acquire', () => ({ type: 'GRANTED', lease }));
  transport.handlers.set('ownership_release', () => undefined);
  transport.handlers.set('ownership_check', () => ({ target: lease.target, holders: [lease] }));
  transport.handlers.set('ownership_resolve', () => ({ ...conflict, state: 'CANCELLED' }));
  return { transport, core: createCoreClient(transport) };
}

describe('ownership client', () => {
  it('reads the map through the typed command name', async () => {
    const { transport, core } = client();

    await expect(core.ownership.list({ workspaceId: lease.workspaceId })).resolves.toEqual(map);

    // `ownership.list` maps to `ownership_list`, and the request travels as one
    // `request` object.
    const call = transport.calls.find((entry) => entry.command === 'ownership_list');
    expect(call?.args).toEqual({ request: { workspaceId: 'w-1' } });
  });

  it('carries a grant and a refusal on the same discriminated answer', async () => {
    const { transport, core } = client();

    const granted = await core.ownership.acquire({
      workspaceId: lease.workspaceId,
      workstreamId: lease.workstreamId,
      target: { type: 'DIRECTORY', path: 'src/auth' },
    });
    expect(granted).toEqual({ type: 'GRANTED', lease });

    transport.handlers.set(
      'ownership_acquire',
      (): OwnershipAcquisition => ({ type: 'CONFLICTED', conflict }),
    );
    const refused = await core.ownership.acquire({
      workspaceId: lease.workspaceId,
      workstreamId: conflict.requestedBy,
      target: { type: 'FILE', path: 'src/auth/session.ts' },
    });
    // A refusal is a value, not an exception: it names the holder, which is
    // what the queue needs to draw a row.
    expect(refused.type).toBe('CONFLICTED');
    if (refused.type === 'CONFLICTED') {
      expect(refused.conflict.ownedBy).toBe('ws-1');
    }
  });

  it('releases, checks and resolves through their own commands', async () => {
    const { transport, core } = client();

    await expect(
      core.ownership.release({ workspaceId: lease.workspaceId, leaseId: lease.id }),
    ).resolves.toBeUndefined();
    await expect(
      core.ownership.check({ workspaceId: lease.workspaceId, target: lease.target }),
    ).resolves.toEqual({ target: lease.target, holders: [lease] });
    await expect(
      core.ownership.resolve({
        workspaceId: lease.workspaceId,
        conflictId: conflict.id,
        action: 'CANCEL',
      }),
    ).resolves.toMatchObject({ state: 'CANCELLED' });

    expect(transport.calls.map((entry) => entry.command)).toEqual(
      expect.arrayContaining(['ownership_release', 'ownership_check', 'ownership_resolve']),
    );
  });

  it('delivers the map event on the single core channel', async () => {
    const { transport, core } = client();
    const seen: OwnershipMap[] = [];

    core.subscribe('OwnershipChanged', (event) => seen.push(event.payload));
    await Promise.resolve();

    transport.emit({
      name: 'OwnershipChanged',
      payload: map,
      seq: 1,
      actor: { type: 'USER' },
    });

    expect(seen).toEqual([map]);
    // One transport listener for the whole vocabulary, demultiplexed by name.
    expect(transport.listenCalls).toBe(1);
  });
});
