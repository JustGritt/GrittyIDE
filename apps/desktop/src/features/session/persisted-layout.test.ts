import type { Document, DocumentId, SessionTab, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import type { DocumentTab } from '../documents/documents-reducer';
import {
  buildSessionTabs,
  DEFAULT_AGENT_RAIL_LAYOUT,
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
      agentRail: { open: true },
      panes: { sidebarWidth: 260, terminalHeight: 340, agentRailWidth: 300 },
    };
    const blob = serializeLayout(layout);
    expect(blob.version).toBe(5);
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
      agentRail: DEFAULT_AGENT_RAIL_LAYOUT,
      panes: DEFAULT_LAYOUT.panes,
    });
  });

  it('reads a blob written before the agent rail existed', () => {
    // Same case one version later: a version-4 blob has no rail section and no
    // rail width, which is a rail nobody has opened rather than a broken blob.
    const before = {
      version: 4,
      explorer: { showHidden: true },
      terminal: { profilePath: '/bin/zsh', customShells: [], fontFamily: null, fontSize: null },
      panes: { sidebarWidth: 260, terminalHeight: 340 },
    };
    const layout = parseLayout(before);
    expect(layout.agentRail).toEqual(DEFAULT_AGENT_RAIL_LAYOUT);
    expect(layout.panes.agentRailWidth).toBe(PANE_LIMITS.agentRail.default);
    // Everything written beside it survives.
    expect(layout.panes.sidebarWidth).toBe(260);
    expect(layout.terminal.profilePath).toBe('/bin/zsh');
  });

  it('ignores a rail section that is not a usable value', () => {
    for (const agentRail of [null, {}, { open: 'yes' }, { open: 1 }, 'open']) {
      expect(parseLayout({ agentRail }).agentRail).toEqual(DEFAULT_AGENT_RAIL_LAYOUT);
    }
    expect(parseLayout({ agentRail: { open: true } }).agentRail.open).toBe(true);
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
      panes: { sidebarWidth: 5000, terminalHeight: 5000, agentRailWidth: 5000 },
    }).panes;
    expect(huge).toEqual({
      sidebarWidth: PANE_LIMITS.sidebar.max,
      terminalHeight: PANE_LIMITS.terminal.max,
      agentRailWidth: PANE_LIMITS.agentRail.max,
    });

    const tiny = parseLayout({
      panes: { sidebarWidth: 1, terminalHeight: -10, agentRailWidth: 12 },
    }).panes;
    expect(tiny).toEqual({
      sidebarWidth: PANE_LIMITS.sidebar.min,
      terminalHeight: PANE_LIMITS.terminal.min,
      agentRailWidth: PANE_LIMITS.agentRail.min,
    });
  });

  it('falls back to the default pane sizes for anything that is not a number', () => {
    for (const panes of [null, {}, { sidebarWidth: '320' }, { sidebarWidth: Number.NaN }]) {
      expect(parseLayout({ panes }).panes.sidebarWidth).toBe(PANE_LIMITS.sidebar.default);
    }
    for (const agentRailWidth of ['320', Number.NaN, null, undefined]) {
      expect(parseLayout({ panes: { agentRailWidth } }).panes.agentRailWidth).toBe(
        PANE_LIMITS.agentRail.default,
      );
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
