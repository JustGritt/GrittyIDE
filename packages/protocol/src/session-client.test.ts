import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { createFakeTransport } from './testing/fake-transport';
import type { HealthReport, SessionRestoreResponse, WorkspaceId } from './types';
import { PROTOCOL_VERSION } from './types';

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const workspaceId = 'ws-1' as WorkspaceId;

function client() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  return { transport, core: createCoreClient(transport) };
}

describe('session client', () => {
  it('restore takes no arguments and returns the persisted session', async () => {
    const { transport, core } = client();
    const response: SessionRestoreResponse = {
      workspace: { id: workspaceId, root: '/repo', name: 'repo', openedAt: 1 },
      tabs: [
        { path: 'README.md', active: true, viewMode: 'preview', cursor: { line: 2, column: 1 } },
      ],
      preferences: { version: 1, fontSize: 15 },
      persistence: { available: true, healthy: true, databasePath: '/data/gritty.sqlite' },
      notices: [],
    };
    transport.handlers.set('session_restore', () => response);
    await expect(core.session.restore()).resolves.toEqual(response);
    const call = transport.calls.find((c) => c.command === 'session_restore');
    expect(call?.args).toBeUndefined();
  });

  it('wraps every other session command in the single request object', async () => {
    const { transport, core } = client();
    transport.handlers.set('session_candidates', () => [
      { path: 'src/main.rs', state: 'SAME_AS_DISK', updatedAt: 5, size: 12 },
    ]);
    transport.handlers.set('session_recover', () => ({
      document: { id: 'doc-1', path: 'src/main.rs', dirty: true },
      text: 'recovered',
    }));
    transport.handlers.set('session_discard', () => null);
    transport.handlers.set('session_update', () => null);

    await expect(core.session.candidates({ workspaceId })).resolves.toHaveLength(1);
    await expect(core.session.recover({ workspaceId, path: 'src/main.rs' })).resolves.toMatchObject(
      {
        text: 'recovered',
      },
    );
    await expect(core.session.discard({ workspaceId, path: 'src/main.rs' })).resolves.toBeNull();
    await expect(
      core.session.update({ workspaceId, tabs: [{ path: 'a.md', active: true }] }),
    ).resolves.toBeNull();

    const args = (command: string) => transport.calls.find((c) => c.command === command)?.args;
    expect(args('session_candidates')).toEqual({ request: { workspaceId } });
    expect(args('session_recover')).toEqual({ request: { workspaceId, path: 'src/main.rs' } });
    expect(args('session_discard')).toEqual({ request: { workspaceId, path: 'src/main.rs' } });
    expect(args('session_update')).toEqual({
      request: { workspaceId, tabs: [{ path: 'a.md', active: true }] },
    });
  });

  it('offers the last session only when the core has one, and takes no arguments', async () => {
    const { transport, core } = client();
    transport.handlers.set('session_last_session', () => ({
      workspace: {
        root: '/repo',
        displayRoot: '~/repo',
        name: 'repo',
        lastOpenedAt: 42,
        exists: true,
      },
      tabs: [{ path: 'README.md', active: true }],
      unsaved: 1,
    }));
    await expect(core.session.lastSession()).resolves.toMatchObject({
      unsaved: 1,
      workspace: { name: 'repo', exists: true },
    });
    expect(transport.calls.find((c) => c.command === 'session_last_session')?.args).toBeUndefined();

    transport.handlers.set('session_last_session', () => null);
    await expect(core.session.lastSession()).resolves.toBeNull();
  });

  it('surfaces a core rejection as a CoreError', async () => {
    const { transport, core } = client();
    transport.handlers.set('session_recover', () => {
      throw {
        code: 'Conflict',
        message: 'src/main.rs has unsaved changes; the recovery snapshot was not applied',
        details: { documentId: 'doc-1', dirty: true },
        retryable: false,
      };
    });
    await expect(core.session.recover({ workspaceId, path: 'src/main.rs' })).rejects.toMatchObject({
      code: 'Conflict',
      details: { dirty: true },
    });
  });
});
