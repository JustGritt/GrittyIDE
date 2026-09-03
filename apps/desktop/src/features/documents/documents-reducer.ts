import type {
  CoreErrorShape,
  Document,
  DocumentClosedReason,
  DocumentContent,
  DocumentId,
  EventEnvelope,
  ExternalModificationState,
} from '@gritty/protocol';

/**
 * Open documents as the UI knows them. The core owns the buffer; this store
 * holds the last text the core confirmed plus the local draft that is still on
 * its way there (debounced `documents.update`). Pure, so it is unit-tested
 * without React or Tauri.
 */
export interface DocumentTab {
  readonly document: Document;
  /** Text as last confirmed by the core; `null` for binary documents. */
  readonly text: string | null;
  /** Local edits the core has not accepted yet; `null` when in sync. */
  readonly draft: string | null;
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
  readonly orphans: readonly OrphanedBuffer[];
  readonly nextOrphanId: number;
}

export const initialDocumentsState: DocumentsState = {
  tabs: {},
  order: [],
  activeId: null,
  orphans: [],
  nextOrphanId: 1,
};

export type DocumentsAction =
  | { type: 'opened'; content: DocumentContent; activate: boolean }
  | { type: 'edited'; documentId: DocumentId; text: string }
  | { type: 'updateStarted'; documentId: DocumentId }
  | { type: 'updateAccepted'; documentId: DocumentId; document: Document; text: string }
  | { type: 'updateFailed'; documentId: DocumentId; error: CoreErrorShape }
  | { type: 'saveStarted'; documentId: DocumentId }
  | { type: 'saveFinished'; documentId: DocumentId; document: Document }
  | { type: 'saveFailed'; documentId: DocumentId; error: CoreErrorShape }
  | { type: 'reloaded'; content: DocumentContent; notice?: string }
  | { type: 'refreshed'; documents: readonly Document[] }
  | { type: 'closeFailed'; documentId: DocumentId; error: CoreErrorShape }
  | { type: 'closed'; documentId: DocumentId }
  | { type: 'activate'; documentId: DocumentId }
  | { type: 'acknowledgeExternal'; documentId: DocumentId }
  | { type: 'clearError'; documentId: DocumentId }
  | { type: 'dismissOrphan'; id: number }
  | { type: 'event'; event: EventEnvelope };

/** Unsaved from the user's point of view: the core says dirty, or a draft is pending. */
export function isUnsaved(tab: DocumentTab): boolean {
  return tab.document.dirty || tab.draft !== null;
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

export function activeTab(state: DocumentsState): DocumentTab | null {
  return state.activeId === null ? null : (state.tabs[state.activeId] ?? null);
}

function makeTab(content: DocumentContent): DocumentTab {
  return {
    document: content.document,
    text: content.text ?? null,
    draft: null,
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

function open(state: DocumentsState, content: DocumentContent, activate: boolean): DocumentsState {
  const id = content.document.id;
  const existing = state.tabs[id];
  const tab: DocumentTab = existing
    ? { ...withDocument(existing, content.document), text: content.text ?? null }
    : makeTab(content);
  const order = existing ? state.order : [...state.order, id];
  return {
    ...state,
    tabs: { ...state.tabs, [id]: tab },
    order,
    activeId: activate || state.activeId === null ? id : state.activeId,
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
  return { ...state, tabs, order, activeId };
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
      return open(state, action.content, action.activate);
    case 'edited':
      return withTab(state, action.documentId, (tab) => {
        if (tab.text === null || tab.document.readOnly) return tab;
        const draft = action.text === tab.text ? null : action.text;
        return draft === tab.draft ? tab : { ...tab, draft, notice: null };
      });
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
    case 'reloaded':
      return withTab(state, action.content.document.id, (tab) => ({
        ...withDocument(tab, action.content.document),
        text: action.content.text ?? null,
        draft: null,
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
