import { type EditorViewState, viewStateFrom } from '@gritty/editor';
import type { DocumentId, SessionTab, WorkspaceId } from '@gritty/protocol';
import type { DocumentTab } from '../documents/documents-reducer';
import { VIEW_MODES, type ViewMode, type ViewModes } from '../markdown/view-modes';

/**
 * What the UI persists besides tabs and editor preferences: a versioned JSON
 * blob the core stores opaquely. Every field falls back to its default so a
 * blob from another version never breaks startup.
 */
export const LAYOUT_VERSION = 1;

export interface Layout {
  readonly explorer: {
    readonly showHidden: boolean;
  };
}

export const DEFAULT_LAYOUT: Layout = { explorer: { showHidden: false } };

export function parseLayout(value: unknown): Layout {
  if (typeof value !== 'object' || value === null) return DEFAULT_LAYOUT;
  const raw = value as Record<string, unknown>;
  const explorer =
    typeof raw.explorer === 'object' && raw.explorer !== null
      ? (raw.explorer as Record<string, unknown>)
      : {};
  return {
    explorer: {
      showHidden:
        typeof explorer.showHidden === 'boolean'
          ? explorer.showHidden
          : DEFAULT_LAYOUT.explorer.showHidden,
    },
  };
}

/** A type alias (not an interface) so it is assignable to the protocol's `JsonValue`. */
export type SerializedLayout = {
  readonly version: number;
  readonly explorer: { readonly showHidden: boolean };
};

export function serializeLayout(layout: Layout): SerializedLayout {
  return { version: LAYOUT_VERSION, explorer: { showHidden: layout.explorer.showHidden } };
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
