import type { Document, DocumentId, SessionTab, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import type { DocumentTab } from '../documents/documents-reducer';
import {
  buildSessionTabs,
  DEFAULT_AGENTS_SECTION_LAYOUT,
  DEFAULT_LAYOUT,
  DEFAULT_TERMINAL_LAYOUT,
  isViewMode,
  MAX_CUSTOM_SHELLS,
  PANE_LIMITS,
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
      agents: { open: false },
      panes: { sidebarWidth: 260, terminalHeight: 340 },
    };
    const blob = serializeLayout(layout);
    expect(blob.version).toBe(6);
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
      agents: DEFAULT_AGENTS_SECTION_LAYOUT,
      panes: DEFAULT_LAYOUT.panes,
    });
  });

  it('ignores the agent rail a version-5 blob still carries', () => {
    // T233-C deleted the rail (ADR-026). Its two fields are not migrated away,
    // they are simply not read — every field falls back independently, so a
    // blob written by the previous build opens without complaint.
    const before = {
      version: 5,
      explorer: { showHidden: true },
      terminal: { profilePath: '/bin/zsh', customShells: [], fontFamily: null, fontSize: null },
      agentRail: { open: true },
      panes: { sidebarWidth: 260, terminalHeight: 340 },
    };
    const layout = parseLayout(before);
    expect(layout).not.toHaveProperty('agentRail');
    expect(layout.panes).not.toHaveProperty('agentRailWidth');
    // Everything written beside it survives.
    expect(layout.panes.sidebarWidth).toBe(260);
    expect(layout.terminal.profilePath).toBe('/bin/zsh');
  });

  it('reads a blob written before the agents section existed', () => {
    // A version-5 blob has no agents section. That is a section nobody has
    // folded, and the default is open — so an existing user gets the new
    // surface rather than an empty strip they have to discover.
    const before = {
      version: 5,
      explorer: { showHidden: true },
      terminal: { profilePath: '/bin/zsh', customShells: [], fontFamily: null, fontSize: null },
      panes: { sidebarWidth: 260, terminalHeight: 340 },
    };
    const layout = parseLayout(before);
    expect(layout.agents).toEqual(DEFAULT_AGENTS_SECTION_LAYOUT);
    expect(layout.agents.open).toBe(true);
    // Everything written beside it survives.
    expect(layout.panes.sidebarWidth).toBe(260);
  });

  it('ignores an agents section that is not a usable value', () => {
    for (const agents of [null, {}, { open: 'yes' }, { open: 1 }, 'open']) {
      expect(parseLayout({ agents }).agents).toEqual(DEFAULT_AGENTS_SECTION_LAYOUT);
    }
    expect(parseLayout({ agents: { open: false } }).agents.open).toBe(false);
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

  it('clamps a stored pane size instead of discarding it', () => {
    // A blob from a larger monitor should still open a usable sidebar.
    const huge = parseLayout({
      panes: { sidebarWidth: 5000, terminalHeight: 5000 },
    }).panes;
    expect(huge).toEqual({
      sidebarWidth: PANE_LIMITS.sidebar.max,
      terminalHeight: PANE_LIMITS.terminal.max,
    });

    const tiny = parseLayout({
      panes: { sidebarWidth: 1, terminalHeight: -10 },
    }).panes;
    expect(tiny).toEqual({
      sidebarWidth: PANE_LIMITS.sidebar.min,
      terminalHeight: PANE_LIMITS.terminal.min,
    });
  });

  it('falls back to the default pane sizes for anything that is not a number', () => {
    for (const panes of [null, {}, { sidebarWidth: '320' }, { sidebarWidth: Number.NaN }]) {
      expect(parseLayout({ panes }).panes.sidebarWidth).toBe(PANE_LIMITS.sidebar.default);
    }
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

describe('an agent console is never persisted (ADR-026)', () => {
  // The claim this whole feature rests on. T231 refused agent entries in the
  // tab strip partly because `SessionTab.path` is a required `WorkspacePath`
  // in a non-nullable column, so a non-document tab would cost a wire change,
  // a ts-rs regeneration and a migration. That cost is avoided by never
  // letting a console into `documents.state` — and this is the guard that
  // fails loudly if it ever gets in.

  it('produces a tab only for a document, because it walks the document order', () => {
    const tabs = { a: tab('a', 'README.md'), b: tab('b', 'src/main.rs') };
    const result = buildSessionTabs({
      workspaceId: ws,
      // A console id parked here — which is what a future refactor might do by
      // accident — resolves to no `DocumentTab` and is dropped, so it can
      // never reach a `TabRow` whose `path` column cannot be null.
      order: ['a', 'session-1', 'b'] as DocumentId[],
      tabs,
      activeId: 'a' as DocumentId,
      viewModes: {},
      readViewState: () => null,
    });
    expect(result.map((entry) => entry.path)).toEqual(['README.md', 'src/main.rs']);
    expect(result.every((entry) => typeof entry.path === 'string' && entry.path !== '')).toBe(true);
  });

  it('keeps the last active document flagged while a console is showing', () => {
    // `documents.state.activeId` is deliberately left pointing at the document
    // even while the console has the pane, so quitting with a console on
    // screen reopens the file you were on. Clearing it would leave every tab
    // `active: false`, and restore would silently fall back to the leftmost.
    const tabs = { a: tab('a', 'README.md'), b: tab('b', 'src/main.rs') };
    const result = buildSessionTabs({
      workspaceId: ws,
      order: ['a', 'b'] as DocumentId[],
      activeId: 'b' as DocumentId,
      tabs,
      viewModes: {},
      readViewState: () => null,
    });
    expect(result.filter((entry) => entry.active).map((entry) => entry.path)).toEqual([
      'src/main.rs',
    ]);
  });

  it('writes nothing about a session into the layout blob', () => {
    // The section's fold is the only agent state that persists, and it is a
    // boolean. A session id in here would be the thing that stops being true.
    const blob = serializeLayout(DEFAULT_LAYOUT);
    expect(Object.keys(blob.agents)).toEqual(['open']);
    expect(typeof blob.agents.open).toBe('boolean');
    expect(JSON.stringify(blob)).not.toContain('session');
  });
});
