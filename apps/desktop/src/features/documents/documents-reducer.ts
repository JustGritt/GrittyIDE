import type {
  CoreErrorShape,
  Document,
  DocumentClosedReason,
  DocumentContent,
  DocumentId,
  EventEnvelope,
  ExternalModificationState,
  WorkspaceId,
} from '@gritty/protocol';

/**
 * Open documents as the UI knows them. The core owns the buffer; this store
 * holds the last text the core confirmed plus the local draft that is still on
 * its way there (debounced `documents.update`). The editor (Monaco, T040)
 * holds the keystrokes between a touch and the next read. Pure, so it is
 * unit-tested without React or Tauri.
 */
export interface DocumentTab {
  readonly document: Document;
  /** Text as last confirmed by the core; `null` for binary documents. */
  readonly text: string | null;
  /** Local edits the core has not accepted yet; `null` when in sync. */
  readonly draft: string | null;
  /** The editor holds edits that have not been read into a draft yet. */
  readonly pending: boolean;
  /**
   * Moves whenever `text` was replaced by the core (reopen, reload, conflict
   * refetch, explicit replacement) and never on an accepted update: the editor
   * pushes `text` into its model only when this changes.
   */
  readonly bufferGeneration: number;
  /** A `documents.update` is in flight. */
  readonly syncing: boolean;
  readonly saving: boolean;
  readonly error: CoreErrorShape | null;
  /** External state the user chose to keep working on ("keep buffer"). */
  readonly acknowledged: ExternalModificationState | null;
  /** Set when the core rejected an edit and the buffer was re-fetched. */
  readonly notice: string | null;
}

/** A dirty buffer whose document the core closed (workspace gone). */
export interface OrphanedBuffer {
  /** Stable per store instance; not a document id. */
  readonly id: number;
  readonly path: string;
  readonly text: string;
  readonly reason: DocumentClosedReason;
}

export interface DocumentsState {
  readonly tabs: Readonly<Record<string, DocumentTab>>;
  readonly order: readonly DocumentId[];
  readonly activeId: DocumentId | null;
  /**
   * The one tab opened for a look rather than to keep: at most one exists, and
   * opening another preview replaces it. `null` when every open tab is kept.
   *
   * Preview is a property of the *tab*, not of the document — the core has one
   * buffer per path either way — so it lives here and never reaches the
   * protocol. It is also not persisted: a preview that survived a restart is a
   * tab the user never asked to keep, and restoring it as an ordinary tab is
   * the honest reading of "I closed the app with this on screen".
   */
  readonly previewId: DocumentId | null;
  readonly orphans: readonly OrphanedBuffer[];
  readonly nextOrphanId: number;
}

export const initialDocumentsState: DocumentsState = {
  tabs: {},
  order: [],
  activeId: null,
  previewId: null,
  orphans: [],
  nextOrphanId: 1,
};

export type DocumentsAction =
  | { type: 'opened'; content: DocumentContent; activate: boolean; preview?: boolean }
  | { type: 'promote'; documentId: DocumentId }
  | { type: 'touched'; documentId: DocumentId }
  | { type: 'edited'; documentId: DocumentId; text: string }
  | { type: 'updateStarted'; documentId: DocumentId }
  | { type: 'updateAccepted'; documentId: DocumentId; document: Document; text: string }
  | { type: 'updateFailed'; documentId: DocumentId; error: CoreErrorShape }
  | { type: 'saveStarted'; documentId: DocumentId }
  | { type: 'saveFinished'; documentId: DocumentId; document: Document }
  | { type: 'saveFailed'; documentId: DocumentId; error: CoreErrorShape }
  | { type: 'replaced'; documentId: DocumentId; document: Document; text: string }
  | { type: 'reloaded'; content: DocumentContent; notice?: string }
  | { type: 'refreshed'; documents: readonly Document[] }
  | { type: 'closeFailed'; documentId: DocumentId; error: CoreErrorShape }
  | { type: 'closed'; documentId: DocumentId }
  | { type: 'activate'; documentId: DocumentId }
  | { type: 'reorder'; documentId: DocumentId; toIndex: number }
  | { type: 'acknowledgeExternal'; documentId: DocumentId }
  | { type: 'clearError'; documentId: DocumentId }
  | { type: 'dismissOrphan'; id: number }
  | { type: 'event'; event: EventEnvelope };

/** Unsaved from the user's point of view: the core says dirty, or edits are on their way. */
export function isUnsaved(tab: DocumentTab): boolean {
  return tab.document.dirty || tab.draft !== null || tab.pending;
}

/** What the editing surface shows. */
export function displayText(tab: DocumentTab): string | null {
  return tab.draft ?? tab.text;
}

/** An external state the user has not decided on yet. */
export type PendingExternalState = Exclude<ExternalModificationState, 'UNCHANGED'>;

/** The external state the banner should show, if any. */
export function pendingExternalState(tab: DocumentTab): PendingExternalState | null {
  const state = tab.document.externalModificationState;
  if (state === 'UNCHANGED' || tab.acknowledged === state) return null;
  return state;
}

/** Whether this tab is the one open for a look rather than to keep. */
export function isPreview(state: DocumentsState, documentId: DocumentId): boolean {
  return state.previewId === documentId;
}

export function activeTab(state: DocumentsState): DocumentTab | null {
  return state.activeId === null ? null : (state.tabs[state.activeId] ?? null);
}

/** The open tab for a workspace path, if any (documents are one per path). */
export function findTabByPath(
  state: DocumentsState,
  workspaceId: WorkspaceId,
  path: string,
): DocumentTab | null {
  for (const id of state.order) {
    const tab = state.tabs[id];
    if (tab && tab.document.workspaceId === workspaceId && tab.document.path === path) return tab;
  }
  return null;
}

function makeTab(content: DocumentContent): DocumentTab {
  return {
    document: content.document,
    text: content.text ?? null,
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

function withTab(
  state: DocumentsState,
  documentId: DocumentId,
  update: (tab: DocumentTab) => DocumentTab,
): DocumentsState {
  const existing = state.tabs[documentId];
  if (!existing) return state;
  const updated = update(existing);
  if (updated === existing) return state;
  return { ...state, tabs: { ...state.tabs, [documentId]: updated } };
}

/** Replace the document metadata; the acknowledgement is dropped once the state moves. */
function withDocument(tab: DocumentTab, document: Document): DocumentTab {
  const acknowledged =
    tab.acknowledged === document.externalModificationState ? tab.acknowledged : null;
  return { ...tab, document, acknowledged };
}

/**
 * Keep this tab: it stops being the preview.
 *
 * Called for a double click, for a reorder, and — the one that matters — for
 * the first edit. A buffer with unsaved work in it must never be the tab that
 * the next single click silently replaces.
 */
function keep(state: DocumentsState, documentId: DocumentId): DocumentsState {
  return state.previewId === documentId ? { ...state, previewId: null } : state;
}

function open(
  state: DocumentsState,
  content: DocumentContent,
  activate: boolean,
  preview = false,
): DocumentsState {
  const id = content.document.id;
  const existing = state.tabs[id];
  // Reopening (rehydration, `largeFileEditing`) returns the core's buffer: it
  // replaces whatever the editor showed.
  const tab: DocumentTab = existing
    ? {
        ...withDocument(existing, content.document),
        text: content.text ?? null,
        draft: null,
        pending: false,
        bufferGeneration: existing.bufferGeneration + 1,
      }
    : makeTab(content);
  const order = existing ? state.order : [...state.order, id];
  // Only a *new* tab can become the preview, and any deliberate open keeps the
  // tab. Both halves matter for the double click, which fires its single click
  // first: three opens of the same path race, and this makes every order of
  // arrival land on "kept" rather than leaving it to whichever resolves last.
  let previewId = state.previewId;
  if (preview) {
    if (!existing) previewId = id;
  } else if (previewId === id) {
    previewId = null;
  }
  return {
    ...state,
    tabs: { ...state.tabs, [id]: tab },
    order,
    activeId: activate || state.activeId === null ? id : state.activeId,
    previewId,
  };
}

function remove(state: DocumentsState, documentId: DocumentId): DocumentsState {
  const tab = state.tabs[documentId];
  if (!tab) return state;
  const { [documentId]: _removed, ...tabs } = state.tabs;
  const index = state.order.indexOf(documentId);
  const order = state.order.filter((id) => id !== documentId);
  let activeId = state.activeId;
  if (activeId === documentId) {
    activeId = order[Math.min(index, order.length - 1)] ?? null;
  }
  // Whichever path closed it — the tab's ×, a batch, or the core deciding the
  // document is gone — a closed tab is no longer the preview.
  const previewId = state.previewId === documentId ? null : state.previewId;
  return { ...state, tabs, order, activeId, previewId };
}

function onEvent(state: DocumentsState, event: EventEnvelope): DocumentsState {
  switch (event.name) {
    case 'DocumentOpened':
    case 'DocumentChanged':
    case 'DocumentSaved':
    case 'DocumentExternalChangeDetected': {
      const document = event.payload.document;
      return withTab(state, document.id, (tab) => withDocument(tab, document));
    }
    case 'DocumentClosed': {
      const tab = state.tabs[event.payload.documentId];
      if (!tab) return state;
      const removed = remove(state, event.payload.documentId);
      const text = displayText(tab);
      if (!event.payload.dirty && tab.draft === null) return removed;
      if (text === null) return removed;
      return {
        ...removed,
        orphans: [
          ...removed.orphans,
          {
            id: removed.nextOrphanId,
            path: event.payload.path,
            text,
            reason: event.payload.reason,
          },
        ],
        nextOrphanId: removed.nextOrphanId + 1,
      };
    }
    default:
      return state;
  }
}

export function documentsReducer(state: DocumentsState, action: DocumentsAction): DocumentsState {
  switch (action.type) {
    case 'opened':
      return open(state, action.content, action.activate, action.preview);
    case 'promote':
      return keep(state, action.documentId);
    case 'touched': {
      const touched = withTab(state, action.documentId, (tab) =>
        tab.text === null || tab.document.readOnly || tab.pending
          ? tab
          : { ...tab, pending: true, notice: null },
      );
      // Typing into a preview keeps it. Not doing this would let the next
      // single click in the explorer close a buffer with unsaved work in it.
      return touched === state ? state : keep(touched, action.documentId);
    }
    case 'edited': {
      const edited = withTab(state, action.documentId, (tab) => {
        if (tab.text === null || tab.document.readOnly) return tab;
        const draft = action.text === tab.text ? null : action.text;
        return draft === tab.draft && !tab.pending
          ? tab
          : { ...tab, draft, pending: false, notice: null };
      });
      // The same rule as `touched`, for the edit that arrives without one
      // (a flush, or text read off a document the core closed).
      return edited.tabs[action.documentId]?.draft == null
        ? edited
        : keep(edited, action.documentId);
    }
    case 'updateStarted':
      return withTab(state, action.documentId, (tab) => ({ ...tab, syncing: true, error: null }));
    case 'updateAccepted':
      return withTab(state, action.documentId, (tab) => ({
        ...withDocument(tab, action.document),
        text: action.text,
        // Keep the draft only when the user typed past what was sent.
        draft: tab.draft === action.text ? null : tab.draft,
        syncing: false,
      }));
    case 'updateFailed':
      return withTab(state, action.documentId, (tab) => ({
        ...tab,
        syncing: false,
        error: action.error,
      }));
    case 'saveStarted':
      return withTab(state, action.documentId, (tab) => ({ ...tab, saving: true, error: null }));
    case 'saveFinished':
      return withTab(state, action.documentId, (tab) => ({
        ...withDocument(tab, action.document),
        saving: false,
      }));
    case 'saveFailed':
    case 'closeFailed':
      return withTab(state, action.documentId, (tab) => ({
        ...tab,
        saving: false,
        error: action.error,
      }));
    case 'replaced':
      // An explicit whole-buffer rewrite (line-ending normalization) is an
      // edit, so it keeps the tab like any other.
      return keep(
        withTab(state, action.documentId, (tab) => ({
          ...withDocument(tab, action.document),
          text: action.text,
          draft: null,
          pending: false,
          bufferGeneration: tab.bufferGeneration + 1,
          syncing: false,
        })),
        action.documentId,
      );
    case 'reloaded':
      return withTab(state, action.content.document.id, (tab) => ({
        ...withDocument(tab, action.content.document),
        text: action.content.text ?? null,
        draft: null,
        pending: false,
        bufferGeneration: tab.bufferGeneration + 1,
        syncing: false,
        acknowledged: null,
        notice: action.notice ?? null,
      }));
    case 'refreshed': {
      let next = state;
      for (const document of action.documents) {
        next = withTab(next, document.id, (tab) => withDocument(tab, document));
      }
      return next;
    }
    case 'closed':
      return remove(state, action.documentId);
    case 'activate':
      return state.tabs[action.documentId] && state.activeId !== action.documentId
        ? { ...state, activeId: action.documentId }
        : state;
    case 'reorder': {
      const from = state.order.indexOf(action.documentId);
      if (from === -1) return state;
      const toIndex = Math.max(0, Math.min(action.toIndex, state.order.length - 1));
      if (from === toIndex) return state;
      const order = state.order.filter((id) => id !== action.documentId);
      order.splice(toIndex, 0, action.documentId);
      // Arranging a tab is asking to keep it; a preview that slid back into
      // place when the next click replaced it would be its own small betrayal.
      return keep({ ...state, order }, action.documentId);
    }
    case 'acknowledgeExternal':
      return withTab(state, action.documentId, (tab) => ({
        ...tab,
        acknowledged: tab.document.externalModificationState,
      }));
    case 'clearError':
      return withTab(state, action.documentId, (tab) =>
        tab.error === null && tab.notice === null ? tab : { ...tab, error: null, notice: null },
      );
    case 'dismissOrphan':
      return { ...state, orphans: state.orphans.filter((orphan) => orphan.id !== action.id) };
    case 'event':
      return onEvent(state, action.event);
  }
}
