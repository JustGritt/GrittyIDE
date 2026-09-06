import type { EditorViewState } from '@gritty/editor';
import type {
  CoreError,
  Document,
  DocumentId,
  SessionRestoreResponse,
  SessionTab,
  Workspace,
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
  /** Hand the workspace the core reports open to the shell that renders it. */
  adoptWorkspace(workspace: Workspace): void;
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
    // Before anything slow: the restore's own answer says which workspace is
    // open, and waiting for `WorkspaceOpened` to repeat it loses the window.
    // The core opens the workspace while the WebView is still registering its
    // event listener, so that event can be emitted to nobody — and then the
    // shell sits on the launcher with a workspace open underneath it, where
    // even dropping that same folder does nothing (reopening the open root
    // emits nothing either).
    if (response.workspace) deps.adoptWorkspace(response.workspace);
    deps.applyPreferences(response.preferences);
    deps.applyLayout(response.layout);
  }

  let live: readonly Document[] = [];
  try {
    live = await deps.rehydrate();
  } catch {
    // The documents hook reports its own error; the session has nothing to add.
  }

  const workspace = response?.workspace;
  const { opened, skipped } =
    response && workspace
      ? await applyTabs(deps, workspace.id, response.tabs, live)
      : { opened: [] as string[], skipped: [] as string[] };
  deps.dispatch({ type: 'restoreSettled', skipped });
  return { response, opened, skipped };
}

/**
 * Open a set of persisted tabs into a workspace and put order, active tab,
 * view modes and cursors back.
 *
 * Extracted from [`runRestore`] because the launcher's resume rail (T051) does
 * the same thing on demand: the startup restore runs once per WebView, so a
 * workspace reopened later cannot go through it. One implementation means the
 * two paths cannot restore differently.
 *
 * `live` is what the core already holds — after a renderer reload those
 * documents are open and must not be opened twice.
 */
export async function applyTabs(
  deps: Pick<RestoreDeps, 'openFile' | 'applyOrder' | 'setViewMode' | 'restoreView'>,
  workspaceId: WorkspaceId,
  tabs: readonly SessionTab[],
  live: readonly Document[] = [],
): Promise<{ opened: string[]; skipped: string[] }> {
  const opened: string[] = [];
  const skipped: string[] = [];
  const byPath = new Map<string, DocumentId>();
  for (const document of live) {
    if (document.workspaceId === workspaceId) byPath.set(document.path, document.id);
  }
  // Active first: it is the one the user sees, so it should not wait behind
  // the others' reads.
  const active = tabs.find((tab) => tab.active) ?? null;
  const openOrder = active ? [active, ...tabs.filter((tab) => tab !== active)] : tabs;
  for (const tab of openOrder) {
    if (byPath.has(tab.path)) continue;
    const result = await deps.openFile(workspaceId, tab.path);
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
    if (document.workspaceId === workspaceId && !ordered.includes(document.id)) {
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
  return { opened, skipped };
}
