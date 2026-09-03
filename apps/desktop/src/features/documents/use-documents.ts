import {
  type CoreError,
  type DocumentContent,
  type DocumentId,
  toCoreError,
  type WorkspaceId,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import {
  type DocumentsState,
  documentsReducer,
  findTabByPath,
  initialDocumentsState,
} from './documents-reducer';
import { UpdateScheduler } from './update-scheduler';

/** Delay between the last keystroke and the `documents.update` that carries it (ADR-011). */
export const UPDATE_DEBOUNCE_MS = 300;
/** Large documents cross the IPC boundary as one string per update: give them a longer pause. */
export const LARGE_FILE_UPDATE_DEBOUNCE_MS = 1000;

/** Reads the editor's current text for a document; registered by the editor binding. */
export type TextReader = () => string;

/**
 * Drives the document store against the core: opens files, turns editor
 * touches into debounced, versioned `documents.update` calls (one in flight
 * per document), saves, reloads, and re-checks the disk when the window
 * regains focus. Text lives in the core; the editor holds at most one
 * debounce interval of keystrokes; this hook only mirrors and forwards.
 */
export function useDocuments() {
  const [state, dispatch] = useReducer(documentsReducer, initialDocumentsState);
  const [error, setError] = useState<CoreError | null>(null);
  const stateRef = useRef<DocumentsState>(state);
  stateRef.current = state;
  /**
   * What the core last confirmed per document, kept in step synchronously:
   * React state lags a render, and a flush chained right after an accepted
   * update must quote the new version, not the one still on screen.
   */
  const confirmed = useRef(new Map<DocumentId, { version: number; text: string | null }>());
  const readers = useRef(new Map<DocumentId, TextReader>());
  const pending = useRef(new Set<DocumentId>());

  const confirm = useCallback((content: DocumentContent) => {
    confirmed.current.set(content.document.id, {
      version: content.document.currentVersion,
      text: content.text ?? null,
    });
  }, []);

  const forget = useCallback((documentId: DocumentId) => {
    confirmed.current.delete(documentId);
    readers.current.delete(documentId);
    pending.current.delete(documentId);
  }, []);

  const opened = useCallback(
    (content: DocumentContent, activate: boolean) => {
      confirm(content);
      pending.current.delete(content.document.id);
      dispatch({ type: 'opened', content, activate });
    },
    [confirm],
  );

  const reloaded = useCallback(
    (content: DocumentContent, notice?: string) => {
      confirm(content);
      pending.current.delete(content.document.id);
      dispatch(
        notice === undefined
          ? { type: 'reloaded', content }
          : { type: 'reloaded', content, notice },
      );
    },
    [confirm],
  );

  /**
   * Send `text` as the next version. On `Conflict` the buffer moved under us
   * (a reload raced the edit): show what the core has and say so, never guess
   * which version wins. Resolves to whether the core now holds `text`.
   */
  const send = useCallback(
    async (
      documentId: DocumentId,
      text: string,
      accepted: 'updateAccepted' | 'replaced',
    ): Promise<boolean> => {
      const known = confirmed.current.get(documentId);
      if (!known) return false;
      dispatch({ type: 'updateStarted', documentId });
      try {
        const document = await core.documents.update({
          documentId,
          baseVersion: known.version,
          text,
        });
        confirmed.current.set(documentId, { version: document.currentVersion, text });
        dispatch({ type: accepted, documentId, document, text });
        return true;
      } catch (cause: unknown) {
        const coreError = toCoreError(cause);
        const tab = stateRef.current.tabs[documentId];
        if (coreError.code === 'Conflict' && tab) {
          try {
            const content = await core.documents.open({
              workspaceId: tab.document.workspaceId,
              path: tab.document.path,
            });
            reloaded(
              content,
              `Your last edit was rejected (${coreError.message}); showing the core's buffer.`,
            );
          } catch (refetch: unknown) {
            dispatch({ type: 'updateFailed', documentId, error: toCoreError(refetch) });
          }
          return false;
        }
        dispatch({ type: 'updateFailed', documentId, error: coreError });
        return false;
      }
    },
    [reloaded],
  );

  /** Read what the editor holds (or retry a draft the core refused) and send it. */
  const flush = useCallback(
    async (documentId: DocumentId): Promise<boolean> => {
      const known = confirmed.current.get(documentId);
      if (!known) return false;
      const read = readers.current.get(documentId);
      let text: string | null;
      if (pending.current.has(documentId) && read) {
        pending.current.delete(documentId);
        text = read();
      } else {
        text = stateRef.current.tabs[documentId]?.draft ?? null;
      }
      if (text === null) return true;
      dispatch({ type: 'edited', documentId, text });
      if (text === known.text) return true;
      return send(documentId, text, 'updateAccepted');
    },
    [send],
  );

  const [scheduler] = useState(
    () =>
      new UpdateScheduler<boolean>({
        debounceMs: (documentId) =>
          stateRef.current.tabs[documentId]?.document.largeFileMode
            ? LARGE_FILE_UPDATE_DEBOUNCE_MS
            : UPDATE_DEBOUNCE_MS,
        flush,
      }),
  );

  // Events, and rehydration after a renderer reload: the core still holds every buffer.
  useEffect(() => {
    const unsubscribe = core.subscribeAll((event) => {
      if (event.name === 'DocumentClosed') {
        const documentId = event.payload.documentId;
        // Keep what the editor still holds so an orphaned buffer is complete.
        const read = readers.current.get(documentId);
        if (pending.current.has(documentId) && read) {
          dispatch({ type: 'edited', documentId, text: read() });
        }
        scheduler.cancel(documentId);
        forget(documentId);
      }
      dispatch({ type: 'event', event });
    });
    let active = true;
    core.documents
      .list()
      .then(async (documents) => {
        for (const [index, document] of documents.entries()) {
          const content = await core.documents.open({
            workspaceId: document.workspaceId,
            path: document.path,
          });
          if (active) opened(content, index === 0);
        }
      })
      .catch((cause: unknown) => {
        if (active) setError(toCoreError(cause));
      });
    return () => {
      active = false;
      unsubscribe();
      scheduler.cancelAll();
    };
  }, [scheduler, opened, forget]);

  const checkStatus = useCallback(async () => {
    if (stateRef.current.order.length === 0) return;
    try {
      const documents = await core.documents.status();
      dispatch({ type: 'refreshed', documents });
    } catch (cause: unknown) {
      setError(toCoreError(cause));
    }
  }, []);

  // External modification is checked on demand (T070 adds a watcher).
  useEffect(() => {
    const onFocus = () => void checkStatus();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [checkStatus]);

  /** The editor registers how to read a document's text; returns the detach function. */
  const attach = useCallback((documentId: DocumentId, read: TextReader) => {
    readers.current.set(documentId, read);
    return () => {
      if (readers.current.get(documentId) === read) readers.current.delete(documentId);
    };
  }, []);

  /** The editor content changed; the text is read when the debounce fires. */
  const touch = useCallback(
    (documentId: DocumentId) => {
      const tab = stateRef.current.tabs[documentId];
      if (!tab || tab.text === null || tab.document.readOnly) return;
      pending.current.add(documentId);
      dispatch({ type: 'touched', documentId });
      scheduler.touch(documentId);
    },
    [scheduler],
  );

  const openFile = useCallback(
    async (workspaceId: WorkspaceId, path: string, largeFileEditing = false) => {
      if (!largeFileEditing) {
        // Reopening would return the live buffer and replace up to one
        // debounce interval of typing in the editor; activating is enough.
        const existing = findTabByPath(stateRef.current, workspaceId, path);
        if (existing) {
          dispatch({ type: 'activate', documentId: existing.document.id });
          return;
        }
      }
      try {
        const request = largeFileEditing
          ? { workspaceId, path, largeFileEditing: true }
          : { workspaceId, path };
        opened(await core.documents.open(request), true);
        setError(null);
      } catch (cause: unknown) {
        setError(toCoreError(cause));
      }
    },
    [opened],
  );

  /** Replace the whole buffer (explicit line-ending normalization); the editor re-seeds its model. */
  const replaceText = useCallback(
    async (documentId: DocumentId, text: string) => {
      await scheduler.settle(documentId);
      await send(documentId, text, 'replaced');
    },
    [scheduler, send],
  );

  /** Resolves to whether the document was saved. */
  const save = useCallback(
    async (documentId: DocumentId, force = false): Promise<boolean> => {
      const synced = await scheduler.settle(documentId);
      // A refused update left its error on the tab; saving now would write stale text.
      if (!synced || !stateRef.current.tabs[documentId]) return false;
      dispatch({ type: 'saveStarted', documentId });
      try {
        const document = await core.documents.save(force ? { documentId, force } : { documentId });
        dispatch({ type: 'saveFinished', documentId, document });
        return true;
      } catch (cause: unknown) {
        dispatch({ type: 'saveFailed', documentId, error: toCoreError(cause) });
        return false;
      }
    },
    [scheduler],
  );

  const reload = useCallback(
    async (documentId: DocumentId) => {
      scheduler.cancel(documentId);
      pending.current.delete(documentId);
      try {
        reloaded(await core.documents.reload({ documentId }));
      } catch (cause: unknown) {
        dispatch({ type: 'updateFailed', documentId, error: toCoreError(cause) });
      }
    },
    [scheduler, reloaded],
  );

  const close = useCallback(
    async (documentId: DocumentId, force = false) => {
      if (force) {
        scheduler.cancel(documentId);
        pending.current.delete(documentId);
      } else {
        await scheduler.settle(documentId);
      }
      try {
        await core.documents.close(force ? { documentId, force } : { documentId });
        forget(documentId);
        dispatch({ type: 'closed', documentId });
      } catch (cause: unknown) {
        dispatch({ type: 'closeFailed', documentId, error: toCoreError(cause) });
      }
    },
    [scheduler, forget],
  );

  const enableEditing = useCallback(
    (documentId: DocumentId) => {
      const tab = stateRef.current.tabs[documentId];
      if (tab) void openFile(tab.document.workspaceId, tab.document.path, true);
    },
    [openFile],
  );

  const activate = useCallback(
    (documentId: DocumentId) => dispatch({ type: 'activate', documentId }),
    [],
  );
  const reorder = useCallback(
    (documentId: DocumentId, toIndex: number) => dispatch({ type: 'reorder', documentId, toIndex }),
    [],
  );
  const keepBuffer = useCallback(
    (documentId: DocumentId) => dispatch({ type: 'acknowledgeExternal', documentId }),
    [],
  );
  const clearTabError = useCallback(
    (documentId: DocumentId) => dispatch({ type: 'clearError', documentId }),
    [],
  );
  const dismissOrphan = useCallback((id: number) => dispatch({ type: 'dismissOrphan', id }), []);
  const clearError = useCallback(() => setError(null), []);

  return {
    state,
    error,
    openFile,
    attach,
    touch,
    replaceText,
    save,
    reload,
    close,
    enableEditing,
    activate,
    reorder,
    keepBuffer,
    checkStatus,
    clearTabError,
    dismissOrphan,
    clearError,
  };
}

export type DocumentsController = ReturnType<typeof useDocuments>;
