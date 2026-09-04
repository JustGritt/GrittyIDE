import type { EditorViewState } from '@gritty/editor';
import type {
  CoreError,
  Document,
  DocumentId,
  SessionRestoreResponse,
  WorkspaceId,
} from '@gritty/protocol';
import type { OpenFileResult } from '../documents/use-documents';
import type { ViewMode } from '../markdown/view-modes';
import { isViewMode, viewStateOf } from './persisted-layout';
import type { SessionAction } from './session-reducer';

/**
 * What the startup restoration needs from the rest of the UI. Kept as plain
 * functions so the flow is unit-tested without React, Monaco or Tauri.
 */
export interface RestoreDeps {
  restore(): Promise<SessionRestoreResponse>;
  /** Open every document the core still holds (renderer reload) into the store. */
  rehydrate(): Promise<readonly Document[]>;
  openFile(workspaceId: WorkspaceId, path: string): Promise<OpenFileResult>;
  applyPreferences(value: unknown): void;
  applyLayout(value: unknown): void;
  applyOrder(ids: readonly DocumentId[], activeId: DocumentId | null): void;
  setViewMode(documentId: DocumentId, mode: ViewMode): void;
  restoreView(documentId: DocumentId, state: EditorViewState): void;
  dispatch(action: SessionAction): void;
  toCoreError(cause: unknown): CoreError;
}

export interface RestoreOutcome {
  readonly response: SessionRestoreResponse | null;
  /** Documents opened from the persisted tabs (not counting the ones the core still had). */
  readonly opened: readonly string[];
  /** Persisted tabs that could not be opened (the file is gone, for instance). */
  readonly skipped: readonly string[];
}

/**
 * The startup sequence (T060): read the persisted session, apply the UI
 * blobs, rehydrate whatever the core still holds (a renderer reload), open
 * the persisted tabs that are not open yet (active first, then the rest),
 * then put order, active tab, view modes and cursors back. Persistence
 * trouble never stops rehydration.
 */
export async function runRestore(deps: RestoreDeps): Promise<RestoreOutcome> {
  deps.dispatch({ type: 'restoreStarted' });
  let response: SessionRestoreResponse | null = null;
  try {
    response = await deps.restore();
    deps.dispatch({ type: 'restoreFinished', response });
  } catch (cause: unknown) {
    deps.dispatch({ type: 'restoreFailed', error: deps.toCoreError(cause) });
  }
  if (response) {
    deps.applyPreferences(response.preferences);
    deps.applyLayout(response.layout);
  }

  let live: readonly Document[] = [];
  try {
    live = await deps.rehydrate();
  } catch {
    // The documents hook reports its own error; the session has nothing to add.
  }

  const opened: string[] = [];
  const skipped: string[] = [];
  const workspace = response?.workspace;
  if (response && workspace) {
    const byPath = new Map<string, DocumentId>();
    for (const document of live) {
      if (document.workspaceId === workspace.id) byPath.set(document.path, document.id);
    }
    const tabs = response.tabs;
    const active = tabs.find((tab) => tab.active) ?? null;
    const openOrder = active ? [active, ...tabs.filter((tab) => tab !== active)] : tabs;
    for (const tab of openOrder) {
      if (byPath.has(tab.path)) continue;
      const result = await deps.openFile(workspace.id, tab.path);
      if (result.ok) {
        byPath.set(tab.path, result.documentId);
        opened.push(tab.path);
      } else {
        skipped.push(tab.path);
      }
    }
    const ordered: DocumentId[] = [];
    for (const tab of tabs) {
      const id = byPath.get(tab.path);
      if (id !== undefined && !ordered.includes(id)) ordered.push(id);
    }
    // Documents the core had that the persisted session does not mention keep their place after.
    for (const document of live) {
      if (document.workspaceId === workspace.id && !ordered.includes(document.id)) {
        ordered.push(document.id);
      }
    }
    const activeId = (active && byPath.get(active.path)) ?? ordered[0] ?? null;
    if (ordered.length > 0) deps.applyOrder(ordered, activeId);
    for (const tab of tabs) {
      const id = byPath.get(tab.path);
      if (id === undefined) continue;
      if (isViewMode(tab.viewMode)) deps.setViewMode(id, tab.viewMode);
      const view = viewStateOf(tab);
      if (view) deps.restoreView(id, view);
    }
  }
  deps.dispatch({ type: 'restoreSettled', skipped });
  return { response, opened, skipped };
}
