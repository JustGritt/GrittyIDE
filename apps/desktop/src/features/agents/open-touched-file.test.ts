import type { CoreErrorShape, DocumentContent, DocumentId, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type DocumentsState,
  documentsReducer,
  findTabByPath,
  initialDocumentsState,
} from '../documents/documents-reducer';
import type { OpenFileOptions, OpenFileResult } from '../documents/use-documents';
import { openTouchedFile, type TouchedFileDocuments } from './open-touched-file';

const WS = 'ws-1' as WorkspaceId;

function content(id: string, path: string): DocumentContent {
  return {
    document: {
      id: id as DocumentId,
      workspaceId: WS,
      path,
      encoding: 'utf-8',
      languageId: 'rust',
      savedRevision: 1,
      savedHash: 'sha256:0',
      currentVersion: 1,
      dirty: false,
      readOnly: false,
      binary: false,
      largeFileMode: false,
      externalModificationState: 'UNCHANGED',
      size: 12,
    },
    text: 'fn main() {}',
  };
}

/**
 * A document store that answers the way the real one does.
 *
 * `openFile` reuses an open tab through the same `findTabByPath` the real
 * controller uses, so the reuse this asserts is the store's own behaviour
 * rather than a second implementation written to make a test pass.
 */
function store() {
  const calls: string[] = [];
  let state: DocumentsState = initialDocumentsState;
  let nextId = 1;
  let statusChecks = 0;
  /** Armed by `failNext`, consumed by the next `openFile`. */
  let refusal: OpenFileResult | null = null;

  const documents: TouchedFileDocuments & { failNext(result: OpenFileResult): void } = {
    async checkStatus() {
      calls.push('status');
      statusChecks += 1;
    },
    async openFile(workspaceId: WorkspaceId, path: string, options?: OpenFileOptions) {
      calls.push(`open ${path}${options?.quiet ? ' (quiet)' : ''}`);
      if (refusal !== null) {
        const result = refusal;
        refusal = null;
        return result;
      }
      // The real controller's own reuse: an open document is one per path, so
      // it activates the tab that exists rather than asking for a second one.
      const existing = findTabByPath(state, workspaceId, path);
      if (existing) return { ok: true, documentId: existing.document.id };
      const opened = content(`doc-${nextId++}`, path);
      state = documentsReducer(state, { type: 'opened', content: opened, activate: true });
      return { ok: true, documentId: opened.document.id };
    },
    failNext(result: OpenFileResult) {
      refusal = result;
    },
  };

  return {
    documents,
    calls,
    tabCount: () => state.order.length,
    statusChecks: () => statusChecks,
  };
}

describe('opening a file the agent touched', () => {
  it('checks the disk before it opens anything', async () => {
    // External modification is checked on demand today (T070 does not exist),
    // so a file the agent rewrote a second ago would otherwise open from a
    // buffer the core last read before the write.
    const { documents, calls } = store();
    await openTouchedFile(documents, WS, 'src/main.rs');
    expect(calls).toEqual(['status', 'open src/main.rs (quiet)']);
  });

  it('checks again on the next open, because nothing is watching the disk', async () => {
    const { documents, statusChecks } = store();
    await openTouchedFile(documents, WS, 'src/main.rs');
    await openTouchedFile(documents, WS, 'src/main.rs');
    expect(statusChecks()).toBe(2);
  });

  it('reuses the tab that is already open for that path', async () => {
    const { documents, tabCount } = store();
    const first = await openTouchedFile(documents, WS, 'src/main.rs');
    const second = await openTouchedFile(documents, WS, 'src/main.rs');
    expect(first).toEqual(second);
    expect(tabCount()).toBe(1);
  });

  it('opens a second file in its own tab', async () => {
    const { documents, tabCount } = store();
    await openTouchedFile(documents, WS, 'src/main.rs');
    await openTouchedFile(documents, WS, 'src/lib.rs');
    expect(tabCount()).toBe(2);
  });

  it('returns the refusal rather than throwing it', async () => {
    // The touched file was deleted since the agent wrote it: the list is
    // history, not a live directory, so this is an ordinary outcome that the
    // rail shows in its own error envelope.
    const { documents } = store();
    const shape: CoreErrorShape = { code: 'NotFound', message: 'no such file', retryable: false };
    const error = shape as Extract<OpenFileResult, { ok: false }>['error'];
    documents.failNext({ ok: false, error });
    const result = await openTouchedFile(documents, WS, 'gone.rs');
    expect(result).toEqual({ ok: false, error });
  });
});
