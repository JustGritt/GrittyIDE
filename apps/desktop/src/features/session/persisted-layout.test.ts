import type { Document, DocumentId, SessionTab, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import type { DocumentTab } from '../documents/documents-reducer';
import {
  buildSessionTabs,
  DEFAULT_LAYOUT,
  isViewMode,
  parseLayout,
  serializeLayout,
  viewStateOf,
} from './persisted-layout';

const ws = 'ws-1' as WorkspaceId;
const other = 'ws-2' as WorkspaceId;

function tab(id: string, path: string, workspaceId: WorkspaceId = ws): DocumentTab {
  const document = { id: id as DocumentId, workspaceId, path } as Document;
  return {
    document,
    text: '',
    draft: null,
    pending: false,
    bufferGeneration: 1,
    syncing: false,
    saving: false,
    error: null,
    acknowledged: null,
    notice: null,
  };
}

describe('layout blob', () => {
  it('round-trips and falls back to defaults', () => {
    const layout = { explorer: { showHidden: true } };
    const blob = serializeLayout(layout);
    expect(blob.version).toBe(1);
    expect(parseLayout(blob)).toEqual(layout);
    expect(parseLayout(undefined)).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout({ version: 2, explorer: { showHidden: 'yes' } })).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout({ explorer: null })).toEqual(DEFAULT_LAYOUT);
  });

  it('recognizes view modes', () => {
    expect(isViewMode('split')).toBe(true);
    expect(isViewMode('sideways')).toBe(false);
    expect(isViewMode(undefined)).toBe(false);
  });
});

describe('buildSessionTabs', () => {
  it('keeps strip order, marks the active tab, and carries view mode and view state', () => {
    const tabs = {
      a: tab('a', 'README.md'),
      b: tab('b', 'src/main.rs'),
      c: tab('c', 'elsewhere.txt', other),
    };
    const result = buildSessionTabs({
      workspaceId: ws,
      order: ['b', 'c', 'a'] as DocumentId[],
      tabs,
      activeId: 'a' as DocumentId,
      viewModes: { a: 'preview' },
      readViewState: (id) =>
        id === 'b'
          ? { cursor: { line: 3, column: 2 }, anchor: { line: 1, column: 1 }, scrollLine: 2 }
          : null,
    });
    expect(result).toEqual<SessionTab[]>([
      {
        path: 'src/main.rs',
        active: false,
        cursor: { line: 3, column: 2 },
        anchor: { line: 1, column: 1 },
        scrollLine: 2,
      },
      { path: 'README.md', active: true, viewMode: 'preview' },
    ]);
  });

  it('reads a view state back from a persisted tab', () => {
    expect(viewStateOf({ path: 'a', active: false })).toBeNull();
    expect(viewStateOf({ path: 'a', active: false, cursor: { line: 5, column: 1 } })).toEqual({
      cursor: { line: 5, column: 1 },
      anchor: { line: 5, column: 1 },
      scrollLine: 5,
    });
  });
});
