import type { Document, DocumentId, SessionTab, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import type { DocumentTab } from '../documents/documents-reducer';
import {
  buildSessionTabs,
  DEFAULT_LAYOUT,
  DEFAULT_TERMINAL_LAYOUT,
  isViewMode,
  MAX_CUSTOM_SHELLS,
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
    const layout = {
      explorer: { showHidden: true },
      terminal: {
        profilePath: '/bin/bash',
        customShells: ['/opt/homebrew/bin/fish'],
        fontFamily: 'MesloLGS NF',
        fontSize: 15,
      },
    };
    const blob = serializeLayout(layout);
    expect(blob.version).toBe(3);
    expect(parseLayout(blob)).toEqual(layout);
    expect(parseLayout(undefined)).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout({ version: 9, explorer: { showHidden: 'yes' } })).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout({ explorer: null })).toEqual(DEFAULT_LAYOUT);
  });

  it('reads a blob written before the terminal settings existed', () => {
    // A version-1 blob has no terminal section; that is "nothing configured",
    // not a reason to discard the explorer setting stored beside it.
    expect(parseLayout({ version: 1, explorer: { showHidden: true } })).toEqual({
      explorer: { showHidden: true },
      terminal: DEFAULT_TERMINAL_LAYOUT,
    });
  });

  it('ignores terminal settings that are not usable values', () => {
    for (const terminal of [null, {}, { profilePath: '' }, { profilePath: 42 }]) {
      expect(parseLayout({ explorer: { showHidden: false }, terminal }).terminal).toEqual(
        DEFAULT_TERMINAL_LAYOUT,
      );
    }
  });

  it('keeps only absolute, unique, bounded custom shells', () => {
    // The blob is a file on disk: a relative path here would resolve against
    // Gritty's own working directory.
    const { customShells } = parseLayout({
      terminal: {
        customShells: ['/bin/fish', 'bin/relative', '/bin/fish', 42, '/opt/homebrew/bin/nu'],
      },
    }).terminal;
    expect(customShells).toEqual(['/bin/fish', '/opt/homebrew/bin/nu']);

    const many = Array.from({ length: MAX_CUSTOM_SHELLS + 5 }, (_, i) => `/bin/shell-${i}`);
    expect(parseLayout({ terminal: { customShells: many } }).terminal.customShells).toHaveLength(
      MAX_CUSTOM_SHELLS,
    );
  });

  it('rejects a font size outside the range the input allows', () => {
    for (const fontSize of [0, 7, 33, Number.NaN, '15']) {
      expect(parseLayout({ terminal: { fontSize } }).terminal.fontSize).toBeNull();
    }
    expect(parseLayout({ terminal: { fontSize: 15 } }).terminal.fontSize).toBe(15);
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
