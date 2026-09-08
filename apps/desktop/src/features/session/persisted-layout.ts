import { type EditorViewState, viewStateFrom } from '@gritty/editor';
import type { DocumentId, SessionTab, WorkspaceId } from '@gritty/protocol';
import type { DocumentTab } from '../documents/documents-reducer';
import { VIEW_MODES, type ViewMode, type ViewModes } from '../markdown/view-modes';

/**
 * What the UI persists besides tabs and editor preferences: a versioned JSON
 * blob the core stores opaquely. Every field falls back to its default so a
 * blob from another version never breaks startup.
 */
export const LAYOUT_VERSION = 6;

export interface Layout {
  readonly explorer: {
    readonly showHidden: boolean;
  };
  readonly terminal: TerminalLayout;
  /** The sidebar's Agents section (T233-S). */
  readonly agents: AgentsSectionLayout;
  /** Pane sizes in CSS pixels, set by dragging the gutters (T046). */
  readonly panes: {
    readonly sidebarWidth: number;
    readonly terminalHeight: number;
  };
}

export interface AgentsSectionLayout {
  /**
   * Whether the section above the file tree is unfolded.
   *
   * Open by default: this is the surface artboard 12a makes primary, and a
   * section that starts folded would put the first agent that needs an answer
   * one click further away than the design puts it. Folding it still leaves
   * the marks and the waiting count, so the default costs nothing a user
   * cannot take back.
   *
   * **This flag is the only thing about agents that is persisted**, and that is
   * load-bearing rather than incidental (ADR-026). Which console was open, and
   * which was showing, are deliberately not: a session id is rediscovered each
   * launch, so restoring one that no longer exists is worse than restoring
   * nothing — and keeping consoles out of the persisted set is exactly what
   * lets them be tabs without a `SessionTab` wire change and a migration.
   */
  readonly open: boolean;
}

/**
 * Pane bounds, shared by the layout parser and the gutters themselves so a
 * stored size and a dragged one obey the same rules.
 */
export const PANE_LIMITS = {
  // 300px is artboard 12a's sidebar, and the Agents section is why it grew
  // from T048's 244: a row carries an id, a task and a status line at once.
  sidebar: { min: 180, max: 640, default: 300 },
  terminal: { min: 120, max: 900, default: 280 },
} as const;

export interface TerminalLayout {
  /**
   * Absolute path of the shell the user picked for new terminals (T095), or
   * `null` for "whatever the machine's default is". A path is remembered
   * rather than an index or a name: the list can change between runs, and
   * the path is what `terminal.spawn` accepts.
   */
  readonly profilePath: string | null;
  /**
   * Shells the user added by hand. Discovery only reports what `$SHELL` and
   * `/etc/shells` name, so this is how a shell the system does not register
   * reaches the picker.
   */
  readonly customShells: readonly string[];
  /** Overrides the terminal's default font stack; `null` keeps it. */
  readonly fontFamily: string | null;
  /** Overrides the terminal's default font size; `null` keeps it. */
  readonly fontSize: number | null;
}

export const DEFAULT_TERMINAL_LAYOUT: TerminalLayout = {
  profilePath: null,
  customShells: [],
  fontFamily: null,
  fontSize: null,
};

/** Open: it is the sidebar's primary surface, not an aside (artboard 12a). */
export const DEFAULT_AGENTS_SECTION_LAYOUT: AgentsSectionLayout = { open: true };

export const DEFAULT_LAYOUT: Layout = {
  explorer: { showHidden: false },
  terminal: DEFAULT_TERMINAL_LAYOUT,
  agents: DEFAULT_AGENTS_SECTION_LAYOUT,
  panes: {
    sidebarWidth: PANE_LIMITS.sidebar.default,
    terminalHeight: PANE_LIMITS.terminal.default,
  },
};

/** Custom shells kept, matching the core's own bound on the request. */
export const MAX_CUSTOM_SHELLS = 16;
/** The range the terminal font size input allows, and parsing enforces. */
export const TERMINAL_FONT_SIZE_RANGE = { min: 8, max: 32 } as const;

export function parseLayout(value: unknown): Layout {
  if (typeof value !== 'object' || value === null) return DEFAULT_LAYOUT;
  const raw = value as Record<string, unknown>;
  const explorer =
    typeof raw.explorer === 'object' && raw.explorer !== null
      ? (raw.explorer as Record<string, unknown>)
      : {};
  const terminal =
    typeof raw.terminal === 'object' && raw.terminal !== null
      ? (raw.terminal as Record<string, unknown>)
      : {};
  const panes =
    typeof raw.panes === 'object' && raw.panes !== null
      ? (raw.panes as Record<string, unknown>)
      : {};
  const agents =
    typeof raw.agents === 'object' && raw.agents !== null
      ? (raw.agents as Record<string, unknown>)
      : {};
  return {
    explorer: {
      showHidden:
        typeof explorer.showHidden === 'boolean'
          ? explorer.showHidden
          : DEFAULT_LAYOUT.explorer.showHidden,
    },
    // A blob written before T095 has no terminal section at all, which is
    // exactly the same as "nothing configured".
    terminal: {
      profilePath: nonEmptyString(terminal.profilePath),
      customShells: parseCustomShells(terminal.customShells),
      fontFamily: nonEmptyString(terminal.fontFamily),
      fontSize: parseFontSize(terminal.fontSize),
    },
    // A blob written before T233-S has no agents section, which is the same as
    // a section nobody has folded — and the default is open.
    agents: {
      open: typeof agents.open === 'boolean' ? agents.open : DEFAULT_AGENTS_SECTION_LAYOUT.open,
    },
    panes: {
      // A stored size is clamped, not discarded: a blob from a larger monitor
      // should still open a usable sidebar rather than silently reset it.
      sidebarWidth: parsePane(panes.sidebarWidth, PANE_LIMITS.sidebar),
      terminalHeight: parsePane(panes.terminalHeight, PANE_LIMITS.terminal),
    },
  };
}

function parsePane(
  value: unknown,
  limits: { readonly min: number; readonly max: number; readonly default: number },
): number {
  if (typeof value !== 'number' || Number.isNaN(value)) return limits.default;
  return Math.min(Math.max(Math.round(value), limits.min), limits.max);
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() !== '' ? value : null;
}

/**
 * Absolute paths only, deduplicated and bounded. The blob is written by this
 * application, but it is also a file on disk that anything could have edited,
 * and a relative path here would resolve against Gritty's own directory.
 */
function parseCustomShells(value: unknown): readonly string[] {
  if (!Array.isArray(value)) return DEFAULT_TERMINAL_LAYOUT.customShells;
  const shells: string[] = [];
  for (const entry of value) {
    if (typeof entry !== 'string') continue;
    const path = entry.trim();
    if (!path.startsWith('/') || shells.includes(path)) continue;
    shells.push(path);
    if (shells.length === MAX_CUSTOM_SHELLS) break;
  }
  return shells;
}

function parseFontSize(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  const size = Math.round(value);
  const { min, max } = TERMINAL_FONT_SIZE_RANGE;
  return size >= min && size <= max ? size : null;
}

/** A type alias (not an interface) so it is assignable to the protocol's `JsonValue`. */
export type SerializedLayout = {
  readonly version: number;
  readonly explorer: { readonly showHidden: boolean };
  readonly terminal: {
    readonly profilePath: string | null;
    /** Mutable, because the protocol's `JsonValue` is. */
    readonly customShells: string[];
    readonly fontFamily: string | null;
    readonly fontSize: number | null;
  };
  readonly agents: { readonly open: boolean };
  readonly panes: {
    readonly sidebarWidth: number;
    readonly terminalHeight: number;
  };
};

export function serializeLayout(layout: Layout): SerializedLayout {
  return {
    version: LAYOUT_VERSION,
    explorer: { showHidden: layout.explorer.showHidden },
    terminal: {
      profilePath: layout.terminal.profilePath,
      customShells: [...layout.terminal.customShells],
      fontFamily: layout.terminal.fontFamily,
      fontSize: layout.terminal.fontSize,
    },
    agents: { open: layout.agents.open },
    panes: {
      sidebarWidth: layout.panes.sidebarWidth,
      terminalHeight: layout.panes.terminalHeight,
    },
  };
}

export function isViewMode(value: unknown): value is ViewMode {
  return typeof value === 'string' && (VIEW_MODES as readonly string[]).includes(value);
}

/** The persisted view state of a tab, or `null` when it carries none. */
export function viewStateOf(tab: SessionTab): EditorViewState | null {
  return viewStateFrom({ cursor: tab.cursor, anchor: tab.anchor, scrollLine: tab.scrollLine });
}

export interface TabsSource {
  readonly workspaceId: WorkspaceId;
  readonly order: readonly DocumentId[];
  readonly tabs: Readonly<Record<string, DocumentTab>>;
  readonly activeId: DocumentId | null;
  /** Explicit view modes by document id; absent means the default for the file. */
  readonly viewModes: ViewModes;
  readViewState(documentId: DocumentId): EditorViewState | null;
}

/** The tabs of one workspace as `session.update` wants them, in strip order. */
export function buildSessionTabs(source: TabsSource): SessionTab[] {
  const result: SessionTab[] = [];
  for (const id of source.order) {
    const tab = source.tabs[id];
    if (!tab || tab.document.workspaceId !== source.workspaceId) continue;
    const entry: SessionTab = { path: tab.document.path, active: id === source.activeId };
    const mode = source.viewModes[id];
    if (mode !== undefined) entry.viewMode = mode;
    const view = source.readViewState(id);
    if (view) {
      entry.cursor = view.cursor;
      entry.anchor = view.anchor;
      entry.scrollLine = view.scrollLine;
    }
    result.push(entry);
  }
  return result;
}
