import type { Document, DocumentContent, DocumentId, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  type DocumentsState,
  documentsReducer,
  initialDocumentsState,
} from '../documents/documents-reducer';
import { planClose, tabsToTheRight } from './close-plan';

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

function opened(...ids: string[]): DocumentsState {
  let state = initialDocumentsState;
  for (const id of ids) {
    const content: DocumentContent = { document: doc(id), text: 'hello' };
    state = documentsReducer(state, { type: 'opened', content, activate: true });
  }
  return state;
}

/** The core has accepted an edit but nothing was saved: the tab is unsaved. */
function dirty(state: DocumentsState, id: string): DocumentsState {
  return documentsReducer(state, {
    type: 'updateAccepted',
    documentId: id as DocumentId,
    document: doc(id, { currentVersion: 2, dirty: true }),
    text: 'edited',
  });
}

const ids = (values: string[]) => values as DocumentId[];

describe('tabsToTheRight', () => {
  it('lists the tabs after the target in tab order', () => {
    const state = opened('a', 'b', 'c');
    expect(tabsToTheRight(state, 'a' as DocumentId)).toEqual(['b', 'c']);
    expect(tabsToTheRight(state, 'b' as DocumentId)).toEqual(['c']);
  });

  it('is empty for the last tab and for an unknown one', () => {
    const state = opened('a', 'b');
    expect(tabsToTheRight(state, 'b' as DocumentId)).toEqual([]);
    expect(tabsToTheRight(state, 'zz' as DocumentId)).toEqual([]);
  });

  it('follows the order after a reorder, not the open order', () => {
    const state = documentsReducer(opened('a', 'b', 'c'), {
      type: 'reorder',
      documentId: 'c' as DocumentId,
      toIndex: 0,
    });
    expect(tabsToTheRight(state, 'c' as DocumentId)).toEqual(['a', 'b']);
  });
});

describe('planClose', () => {
  it('closes saved documents without asking', () => {
    const plan = planClose(opened('a', 'b', 'c'), ids(['a', 'b', 'c']));
    expect(plan).toEqual({ closeNow: ['a', 'b', 'c'], ask: null, rest: [] });
  });

  it('stops at the first unsaved document and keeps the rest queued', () => {
    const state = dirty(opened('a', 'b', 'c'), 'b');
    const plan = planClose(state, ids(['a', 'b', 'c']));
    expect(plan).toEqual({ closeNow: ['a'], ask: 'b', rest: ['c'] });
  });

  it('asks again for a second unsaved document once the first is resolved', () => {
    const state = dirty(dirty(opened('a', 'b', 'c'), 'b'), 'c');
    const first = planClose(state, ids(['a', 'b', 'c']));
    expect(first.ask).toBe('b');
    expect(planClose(state, first.rest)).toEqual({ closeNow: [], ask: 'c', rest: [] });
  });

  it('skips documents that are already gone', () => {
    const state = opened('a', 'c');
    expect(planClose(state, ids(['a', 'b', 'c']))).toEqual({
      closeNow: ['a', 'c'],
      ask: null,
      rest: [],
    });
  });

  it('plans nothing for an empty queue', () => {
    expect(planClose(opened('a'), [])).toEqual({ closeNow: [], ask: null, rest: [] });
  });
});
