import type {
  Document,
  DocumentContent,
  DocumentId,
  EventEnvelope,
  WorkspaceId,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  activeTab,
  type DocumentsState,
  displayText,
  documentsReducer,
  initialDocumentsState,
  isUnsaved,
  pendingExternalState,
} from './documents-reducer';

const WS = 'ws-1' as WorkspaceId;

function doc(id: string, overrides: Partial<Document> = {}): Document {
  return {
    id: id as DocumentId,
    workspaceId: WS,
    path: `${id}.md`,
    encoding: 'utf-8',
    languageId: 'markdown',
    savedRevision: 1,
    savedHash: 'sha256:a',
    currentVersion: 1,
    dirty: false,
    readOnly: false,
    binary: false,
    largeFileMode: false,
    externalModificationState: 'UNCHANGED',
    size: 5,
    ...overrides,
  };
}

function content(id: string, text: string | null = 'hello', overrides: Partial<Document> = {}) {
  const value: DocumentContent = { document: doc(id, overrides) };
  if (text !== null) value.text = text;
  return value;
}

function event(name: EventEnvelope['name'], payload: unknown): EventEnvelope {
  return { name, seq: 1, actor: { type: 'USER' }, payload } as EventEnvelope;
}

function opened(...ids: string[]): DocumentsState {
  let state = initialDocumentsState;
  for (const id of ids) {
    state = documentsReducer(state, { type: 'opened', content: content(id), activate: true });
  }
  return state;
}

const A = 'a' as DocumentId;
const B = 'b' as DocumentId;

describe('documents reducer', () => {
  it('opens tabs, activates them and reopening keeps the tab', () => {
    const state = opened('a', 'b');
    expect(state.order).toEqual(['a', 'b']);
    expect(state.activeId).toBe('b');
    const again = documentsReducer(state, {
      type: 'opened',
      content: content('a', 'current buffer', { currentVersion: 3, dirty: true }),
      activate: false,
    });
    expect(again.order).toEqual(['a', 'b']);
    expect(again.activeId).toBe('b');
    expect(again.tabs.a?.text).toBe('current buffer');
    expect(again.tabs.a?.document.currentVersion).toBe(3);
  });

  it('shows a draft as unsaved until the core accepts exactly that text', () => {
    let state = opened('a');
    const tab = () => state.tabs.a as NonNullable<DocumentsState['tabs'][string]>;
    expect(isUnsaved(tab())).toBe(false);
    state = documentsReducer(state, { type: 'edited', documentId: A, text: 'hello world' });
    expect(tab().draft).toBe('hello world');
    expect(isUnsaved(tab())).toBe(true);
    expect(displayText(tab())).toBe('hello world');
    // Typing back the confirmed text clears the draft.
    state = documentsReducer(state, { type: 'edited', documentId: A, text: 'hello' });
    expect(tab().draft).toBeNull();
    expect(isUnsaved(tab())).toBe(false);

    state = documentsReducer(state, { type: 'edited', documentId: A, text: 'hello world' });
    state = documentsReducer(state, { type: 'updateStarted', documentId: A });
    expect(tab().syncing).toBe(true);
    // The user kept typing while the update was in flight.
    state = documentsReducer(state, { type: 'edited', documentId: A, text: 'hello world!' });
    state = documentsReducer(state, {
      type: 'updateAccepted',
      documentId: A,
      document: doc('a', { currentVersion: 2, dirty: true }),
      text: 'hello world',
    });
    expect(tab().syncing).toBe(false);
    expect(tab().text).toBe('hello world');
    expect(tab().draft).toBe('hello world!');
    expect(tab().document.currentVersion).toBe(2);
    state = documentsReducer(state, {
      type: 'updateAccepted',
      documentId: A,
      document: doc('a', { currentVersion: 3, dirty: true }),
      text: 'hello world!',
    });
    expect(tab().draft).toBeNull();
    expect(isUnsaved(tab())).toBe(true);
    state = documentsReducer(state, {
      type: 'saveFinished',
      documentId: A,
      document: doc('a', { currentVersion: 3, savedRevision: 2 }),
    });
    expect(isUnsaved(tab())).toBe(false);
  });

  it('refuses drafts for binary and read-only documents', () => {
    let state = documentsReducer(initialDocumentsState, {
      type: 'opened',
      content: content('bin', null, { binary: true, encoding: 'binary' }),
      activate: true,
    });
    state = documentsReducer(state, {
      type: 'opened',
      content: content('big', 'x', { largeFileMode: true, readOnly: true }),
      activate: true,
    });
    state = documentsReducer(state, { type: 'edited', documentId: 'bin' as DocumentId, text: 'x' });
    state = documentsReducer(state, { type: 'edited', documentId: 'big' as DocumentId, text: 'y' });
    expect(state.tabs.bin?.draft).toBeNull();
    expect(state.tabs.big?.draft).toBeNull();
    expect(displayText(state.tabs.bin as never)).toBeNull();
  });

  it('tracks external state from events and honours keep-buffer until it moves again', () => {
    let state = opened('a');
    const tab = () => state.tabs.a as NonNullable<DocumentsState['tabs'][string]>;
    state = documentsReducer(state, {
      type: 'event',
      event: event('DocumentExternalChangeDetected', {
        document: doc('a', { externalModificationState: 'MODIFIED_EXTERNALLY' }),
        previousState: 'UNCHANGED',
      }),
    });
    expect(pendingExternalState(tab())).toBe('MODIFIED_EXTERNALLY');
    state = documentsReducer(state, { type: 'acknowledgeExternal', documentId: A });
    expect(pendingExternalState(tab())).toBeNull();
    // Same state re-reported: still acknowledged.
    state = documentsReducer(state, {
      type: 'refreshed',
      documents: [doc('a', { externalModificationState: 'MODIFIED_EXTERNALLY' })],
    });
    expect(pendingExternalState(tab())).toBeNull();
    // A different state shows the banner again.
    state = documentsReducer(state, {
      type: 'refreshed',
      documents: [doc('a', { externalModificationState: 'CONFLICTED', dirty: true }), doc('zzz')],
    });
    expect(pendingExternalState(tab())).toBe('CONFLICTED');
    expect(state.tabs.zzz).toBeUndefined();
    state = documentsReducer(state, {
      type: 'reloaded',
      content: content('a', 'from disk'),
      notice: 'reloaded',
    });
    expect(pendingExternalState(tab())).toBeNull();
    expect(tab().text).toBe('from disk');
    expect(tab().draft).toBeNull();
    expect(tab().notice).toBe('reloaded');
  });

  it('closing removes the tab and picks a neighbour; dirty closes keep the text as an orphan', () => {
    let state = opened('a', 'b', 'c');
    state = documentsReducer(state, { type: 'activate', documentId: B });
    state = documentsReducer(state, { type: 'edited', documentId: B, text: 'typed but unsent' });
    state = documentsReducer(state, {
      type: 'event',
      event: event('DocumentClosed', {
        documentId: B,
        workspaceId: WS,
        path: 'b.md',
        reason: 'WORKSPACE_CLOSED',
        dirty: false,
      }),
    });
    expect(state.order).toEqual(['a', 'c']);
    expect(state.activeId).toBe('c');
    expect(state.orphans).toEqual([
      { id: 1, path: 'b.md', text: 'typed but unsent', reason: 'WORKSPACE_CLOSED' },
    ]);
    state = documentsReducer(state, { type: 'closed', documentId: 'c' as DocumentId });
    expect(state.activeId).toBe('a');
    state = documentsReducer(state, { type: 'dismissOrphan', id: 1 });
    expect(state.orphans).toEqual([]);
    state = documentsReducer(state, {
      type: 'event',
      event: event('DocumentClosed', {
        documentId: A,
        workspaceId: WS,
        path: 'a.md',
        reason: 'USER',
        dirty: true,
      }),
    });
    expect(state.orphans).toEqual([{ id: 2, path: 'a.md', text: 'hello', reason: 'USER' }]);
    expect(activeTab(state)).toBeNull();
    expect(state.order).toEqual([]);
  });

  it('records failures on the tab and clears them', () => {
    let state = opened('a');
    const error = { code: 'Io' as const, message: 'disk full', retryable: true };
    state = documentsReducer(state, { type: 'saveStarted', documentId: A });
    expect(state.tabs.a?.saving).toBe(true);
    state = documentsReducer(state, { type: 'saveFailed', documentId: A, error });
    expect(state.tabs.a?.saving).toBe(false);
    expect(state.tabs.a?.error).toEqual(error);
    state = documentsReducer(state, { type: 'clearError', documentId: A });
    expect(state.tabs.a?.error).toBeNull();
    expect(documentsReducer(state, { type: 'clearError', documentId: A })).toBe(state);
    expect(
      documentsReducer(state, { type: 'updateFailed', documentId: 'nope' as DocumentId, error }),
    ).toBe(state);
  });
});
