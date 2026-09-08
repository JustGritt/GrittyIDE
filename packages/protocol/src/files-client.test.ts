import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { CoreError } from './errors';
import { createFakeTransport } from './testing/fake-transport';
import type { FileOperationResult, HealthReport, WorkspaceId } from './types';
import { PROTOCOL_VERSION } from './types';

const WS_ID = 'ws-1' as WorkspaceId;

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'linux',
  arch: 'x86_64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const result: FileOperationResult = { changed: [''], moved: [], undo: 'Create a.ts' };

function setup() {
  const seen: { command: string; args: unknown }[] = [];
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  for (const command of ['files_create', 'files_rename', 'files_undo', 'files_redo']) {
    transport.handlers.set(command, (args) => {
      seen.push({ command, args });
      return result;
    });
  }
  transport.handlers.set('files_move', (args) => {
    seen.push({ command: 'files_move', args });
    return { status: 'ready', result };
  });
  transport.handlers.set('files_delete', (args) => {
    seen.push({ command: 'files_delete', args });
    const request = args?.request as { force?: boolean };
    if (request.force !== true) {
      throw {
        code: 'Conflict',
        message: '1 document(s) under the selection have unsaved changes',
        retryable: false,
        details: { dirtyDocuments: [{ path: 'src/main.rs' }] },
      };
    }
    return { status: 'pending', operation: { id: 'op-1', kind: 'files.transfer' } };
  });
  return { client: createCoreClient(transport), seen };
}

describe('files client', () => {
  it('sends every command as a single wrapped request', async () => {
    const { client, seen } = setup();
    await client.files.create({ workspaceId: WS_ID, path: 'a/b/c.ts', kind: 'FILE' });
    await client.files.rename({ workspaceId: WS_ID, path: 'a.ts', newName: 'b.ts' });
    await client.files.move({
      workspaceId: WS_ID,
      paths: ['a.ts'],
      targetDir: 'src',
      mode: 'MOVE',
    });

    expect(seen.map((call) => call.command)).toEqual([
      'files_create',
      'files_rename',
      'files_move',
    ]);
    expect(seen[0]?.args).toEqual({
      request: { workspaceId: WS_ID, path: 'a/b/c.ts', kind: 'FILE' },
    });
    expect(seen[2]?.args).toEqual({
      request: { workspaceId: WS_ID, paths: ['a.ts'], targetDir: 'src', mode: 'MOVE' },
    });
  });

  it('builds the undo and redo requests from the workspace id alone', async () => {
    const { client, seen } = setup();
    await client.files.undo(WS_ID);
    await client.files.redo(WS_ID);
    expect(seen.map((call) => call.args)).toEqual([
      { request: { workspaceId: WS_ID } },
      { request: { workspaceId: WS_ID } },
    ]);
  });

  it('returns the result of a create, including what ⌘Z would now reverse', async () => {
    const { client } = setup();
    const created = await client.files.create({
      workspaceId: WS_ID,
      path: 'a.ts',
      kind: 'FILE',
    });
    expect(created.changed).toEqual(['']);
    expect(created.undo).toBe('Create a.ts');
  });

  it('carries both arms of a move: ready inline, pending as an operation', async () => {
    const { client } = setup();
    const ready = await client.files.move({
      workspaceId: WS_ID,
      paths: ['a.ts'],
      targetDir: 'src',
      mode: 'COPY',
    });
    expect(ready.status).toBe('ready');
    if (ready.status === 'ready') expect(ready.result.changed).toEqual(['']);

    const pending = await client.files.delete({
      workspaceId: WS_ID,
      paths: ['a.ts'],
      mode: 'TRASH',
      force: true,
    });
    expect(pending.status).toBe('pending');
    if (pending.status === 'pending') expect(pending.operation.id).toBe('op-1');
  });

  it('surfaces the dirty-document refusal as a CoreError that names what is unsaved', async () => {
    const { client } = setup();
    const refused = await client.files
      .delete({ workspaceId: WS_ID, paths: ['src'], mode: 'TRASH' })
      .catch((error: unknown) => error);

    expect(refused).toBeInstanceOf(CoreError);
    const error = refused as CoreError;
    expect(error.code).toBe('Conflict');
    // The UI needs the list to offer "delete anyway"; it must survive the wire.
    expect((error.details as { dirtyDocuments: unknown[] }).dirtyDocuments).toHaveLength(1);
  });

  it('omits force rather than sending it as undefined', async () => {
    const { client, seen } = setup();
    await client.files
      .delete({ workspaceId: WS_ID, paths: ['a.ts'], mode: 'PERMANENT' })
      .catch(() => undefined);
    expect(seen[0]?.args).toEqual({
      request: { workspaceId: WS_ID, paths: ['a.ts'], mode: 'PERMANENT' },
    });
  });
});
