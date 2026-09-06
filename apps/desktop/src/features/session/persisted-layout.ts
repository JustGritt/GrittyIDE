import { type EditorViewState, viewStateFrom } from '@gritty/editor';
import type { DocumentId, SessionTab, WorkspaceId } from '@gritty/protocol';
import type { DocumentTab } from '../documents/documents-reducer';
import { VIEW_MODES, type ViewMode, type ViewModes } from '../markdown/view-modes';

/**
 * What the UI persists besides tabs and editor preferences: a versioned JSON
 * blob the core stores opaquely. Every field falls back to its default so a
 * blob from another version never breaks startup.
 */
export const LAYOUT_VERSION = 5;

export interface Layout {
  readonly explorer: {
    readonly showHidden: boolean;
  };
  readonly terminal: TerminalLayout;
  /** The agent rail's own state (T231); its width lives in `panes`. */
  readonly agentRail: AgentRailLayout;
  /** Pane sizes in CSS pixels, set by dragging the gutters (T046). */
  readonly panes: {
    readonly sidebarWidth: number;
    readonly terminalHeight: number;
    readonly agentRailWidth: number;
  };
}

export interface AgentRailLayout {
  /**
   * Whether the rail is expanded. Persisted, unlike the dock's disclosure,
   * because the rail takes width from the editor: reopening into a layout the
   * user did not choose is a worse surprise horizontally than vertically.
   *
   * Which session was selected is deliberately *not* persisted. A session id
   * is ephemeral and rediscovered each launch, so restoring a selection that
   * no longer exists is worse than restoring nothing.
   */
  readonly open: boolean;
}

/**
 * Pane bounds, shared by the layout parser and the gutters themselves so a
 * stored size and a dragged one obey the same rules.
 */
export const PANE_LIMITS = {
  sidebar: { min: 180, max: 640, default: 244 },
  terminal: { min: 120, max: 900, default: 280 },
  // Narrower than this and a session row cannot hold a task line, a directory
  // and a status at once, which is the whole content of the rail.
  agentRail: { min: 240, max: 640, default: 320 },
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

/** Collapsed until asked for: the rail takes its width from the editor. */
export const DEFAULT_AGENT_RAIL_LAYOUT: AgentRailLayout = { open: false };

export const DEFAULT_LAYOUT: Layout = {
  explorer: { showHidden: false },
  terminal: DEFAULT_TERMINAL_LAYOUT,
  agentRail: DEFAULT_AGENT_RAIL_LAYOUT,
  panes: {
    sidebarWidth: PANE_LIMITS.sidebar.default,
    terminalHeight: PANE_LIMITS.terminal.default,
    agentRailWidth: PANE_LIMITS.agentRail.default,
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
  const agentRail =
    typeof raw.agentRail === 'object' && raw.agentRail !== null
      ? (raw.agentRail as Record<string, unknown>)
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
    // A blob written before T231 has no rail section, which is the same as a
    // rail nobody has opened yet.
    agentRail: {
      open: typeof agentRail.open === 'boolean' ? agentRail.open : DEFAULT_AGENT_RAIL_LAYOUT.open,
    },
    panes: {
      // A stored size is clamped, not discarded: a blob from a larger monitor
      // should still open a usable sidebar rather than silently reset it.
      sidebarWidth: parsePane(panes.sidebarWidth, PANE_LIMITS.sidebar),
      terminalHeight: parsePane(panes.terminalHeight, PANE_LIMITS.terminal),
      agentRailWidth: parsePane(panes.agentRailWidth, PANE_LIMITS.agentRail),
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
  readonly agentRail: { readonly open: boolean };
  readonly panes: {
    readonly sidebarWidth: number;
    readonly terminalHeight: number;
    readonly agentRailWidth: number;
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
    agentRail: { open: layout.agentRail.open },
    panes: {
      sidebarWidth: layout.panes.sidebarWidth,
      terminalHeight: layout.panes.terminalHeight,
      agentRailWidth: layout.panes.agentRailWidth,
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
