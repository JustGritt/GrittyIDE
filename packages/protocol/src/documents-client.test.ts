import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { CoreError } from './errors';
import { createFakeTransport } from './testing/fake-transport';
import type { Document, DocumentContent, DocumentId, HealthReport, WorkspaceId } from './types';

const WS_ID = 'ws-1' as WorkspaceId;
const DOC_ID = 'doc-1' as DocumentId;

const health: HealthReport = {
  protocolVersion: '1.3.0',
  coreVersion: '0.1.0',
  os: 'linux',
  arch: 'x86_64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const document: Document = {
  id: DOC_ID,
  workspaceId: WS_ID,
  path: 'src/main.rs',
  encoding: 'utf-8',
  languageId: 'rust',
  savedRevision: 1,
  savedHash: 'sha256:00',
  currentVersion: 1,
  dirty: false,
  readOnly: false,
  binary: false,
  largeFileMode: false,
  externalModificationState: 'UNCHANGED',
  size: 12,
};

function setup() {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set(
    'documents_open',
    (): DocumentContent => ({ document, text: 'fn main() {}' }),
  );
  transport.handlers.set('documents_update', (args) => {
    const request = args?.request as { baseVersion: number; text: string };
    if (request.baseVersion !== document.currentVersion) {
      throw {
        code: 'Conflict',
        message: 'stale',
        retryable: false,
        details: { currentVersion: document.currentVersion },
      };
    }
    return { ...document, currentVersion: 2, dirty: true };
  });
  transport.handlers.set('documents_save', () => ({ ...document, savedRevision: 2 }));
  transport.handlers.set(
    'documents_reload',
    (): DocumentContent => ({ document, text: 'reloaded' }),
  );
  transport.handlers.set('documents_status', () => [document]);
  transport.handlers.set('documents_list', () => [document]);
  transport.handlers.set('documents_close', () => document);
  transport.handlers.set('workspace_open', () => null);
  transport.handlers.set('workspace_close', () => ({
    id: WS_ID,
    root: '/ws',
    name: 'ws',
    openedAt: 1,
  }));
  return { transport, client: createCoreClient(transport) };
}

describe('documents commands', () => {
  it('wraps every request as a single request object', async () => {
    const { client, transport } = setup();
    const opened = await client.documents.open({ workspaceId: WS_ID, path: 'src/main.rs' });
    expect(opened.text).toBe('fn main() {}');
    const updated = await client.documents.update({
      documentId: DOC_ID,
      baseVersion: 1,
      text: 'fn main() { }',
    });
    expect(updated.currentVersion).toBe(2);
    await client.documents.save({ documentId: DOC_ID, force: true });
    await client.documents.reload({ documentId: DOC_ID });
    await client.documents.status({ documentId: DOC_ID });
    await client.documents.status();
    await client.documents.list();
    await client.documents.close({ documentId: DOC_ID });
    const calls = transport.calls.filter((c) => c.command.startsWith('documents_'));
    expect(calls.map((c) => c.command)).toEqual([
      'documents_open',
      'documents_update',
      'documents_save',
      'documents_reload',
      'documents_status',
      'documents_status',
      'documents_list',
      'documents_close',
    ]);
    expect(calls[0]?.args).toEqual({ request: { workspaceId: WS_ID, path: 'src/main.rs' } });
    expect(calls[2]?.args).toEqual({ request: { documentId: DOC_ID, force: true } });
    expect(calls[4]?.args).toEqual({ request: { documentId: DOC_ID } });
    expect(calls[5]?.args).toEqual({ request: {} });
    expect(calls[6]?.args).toBeUndefined();
  });

  it('surfaces a stale baseVersion as a Conflict CoreError with details', async () => {
    const { client } = setup();
    const error = await client.documents
      .update({ documentId: DOC_ID, baseVersion: 0, text: 'x' })
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(CoreError);
    expect((error as CoreError).code).toBe('Conflict');
    expect((error as CoreError).details).toEqual({ currentVersion: 1 });
  });

  it('passes force through to workspace open and close', async () => {
    const { client, transport } = setup();
    await client.workspace.open(undefined, { force: true });
    await client.workspace.open('/ws');
    await client.workspace.close(WS_ID, { force: true });
    await client.workspace.close(WS_ID);
    const args = transport.calls
      .filter((c) => c.command.startsWith('workspace_'))
      .map((c) => c.args);
    expect(args).toEqual([
      { request: { force: true } },
      { request: { path: '/ws' } },
      { request: { workspaceId: WS_ID, force: true } },
      { request: { workspaceId: WS_ID } },
    ]);
  });
});
