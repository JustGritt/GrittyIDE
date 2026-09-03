import {
  type CoreError,
  type DocumentContent,
  type DocumentId,
  toCoreError,
  type WorkspaceId,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import { type DocumentsState, documentsReducer, initialDocumentsState } from './documents-reducer';

/** Delay between the last keystroke and the `documents.update` that carries it. */
export const UPDATE_DEBOUNCE_MS = 300;

/**
 * Drives the document store against the core: opens files, debounces edits
 * into versioned `documents.update` calls (one in flight per document, the
 * next one waits), saves, reloads, and re-checks the disk when the window
 * regains focus. Text lives in the core; this hook only mirrors it.
 */
export function useDocuments() {
  const [state, dispatch] = useReducer(documentsReducer, initialDocumentsState);
  const [error, setError] = useState<CoreError | null>(null);
  const stateRef = useRef<DocumentsState>(state);
  stateRef.current = state;
  const timers = useRef(new Map<DocumentId, ReturnType<typeof setTimeout>>());

  // Rehydrate after a renderer reload: the core still holds every buffer.
  useEffect(() => {
    const unsubscribe = core.subscribeAll((event) => dispatch({ type: 'event', event }));
    let active = true;
    core.documents
      .list()
      .then(async (documents) => {
        for (const [index, document] of documents.entries()) {
          const content = await core.documents.open({
            workspaceId: document.workspaceId,
            path: document.path,
          });
          if (active) dispatch({ type: 'opened', content, activate: index === 0 });
        }
      })
      .catch((cause: unknown) => {
        if (active) setError(toCoreError(cause));
      });
    return () => {
      active = false;
      unsubscribe();
      for (const timer of timers.current.values()) clearTimeout(timer);
      timers.current.clear();
    };
  }, []);

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

  /** Send the pending draft of one document, then any draft typed meanwhile. */
  const flush = useCallback(async (documentId: DocumentId): Promise<void> => {
    timers.current.delete(documentId);
    const tab = stateRef.current.tabs[documentId];
    if (!tab || tab.draft === null || tab.syncing) return;
    const text = tab.draft;
    dispatch({ type: 'updateStarted', documentId });
    try {
      const document = await core.documents.update({
        documentId,
        baseVersion: tab.document.currentVersion,
        text,
      });
      dispatch({ type: 'updateAccepted', documentId, document, text });
    } catch (cause: unknown) {
      const coreError = toCoreError(cause);
      if (coreError.code === 'Conflict') {
        // The buffer moved under us (a reload raced the edit): show what the
        // core has and say so, never guess which version wins.
        try {
          const content: DocumentContent = await core.documents.open({
            workspaceId: tab.document.workspaceId,
            path: tab.document.path,
          });
          dispatch({
            type: 'reloaded',
            content,
            notice: `Your last edit was rejected (${coreError.message}); showing the core's buffer.`,
          });
        } catch (refetch: unknown) {
          dispatch({ type: 'updateFailed', documentId, error: toCoreError(refetch) });
        }
        return;
      }
      dispatch({ type: 'updateFailed', documentId, error: coreError });
      return;
    }
    if (stateRef.current.tabs[documentId]?.draft !== null) await flush(documentId);
  }, []);

  const edit = useCallback(
    (documentId: DocumentId, text: string) => {
      dispatch({ type: 'edited', documentId, text });
      const existing = timers.current.get(documentId);
      if (existing) clearTimeout(existing);
      timers.current.set(
        documentId,
        setTimeout(() => void flush(documentId), UPDATE_DEBOUNCE_MS),
      );
    },
    [flush],
  );

  const openFile = useCallback(
    async (workspaceId: WorkspaceId, path: string, largeFileEditing = false) => {
      try {
        const request = largeFileEditing
          ? { workspaceId, path, largeFileEditing: true }
          : { workspaceId, path };
        const content = await core.documents.open(request);
        dispatch({ type: 'opened', content, activate: true });
        setError(null);
      } catch (cause: unknown) {
        setError(toCoreError(cause));
      }
    },
    [],
  );

  const save = useCallback(
    async (documentId: DocumentId, force = false) => {
      const pending = timers.current.get(documentId);
      if (pending) {
        clearTimeout(pending);
        await flush(documentId);
      }
      if (!stateRef.current.tabs[documentId]) return;
      dispatch({ type: 'saveStarted', documentId });
      try {
        const document = await core.documents.save(force ? { documentId, force } : { documentId });
        dispatch({ type: 'saveFinished', documentId, document });
      } catch (cause: unknown) {
        dispatch({ type: 'saveFailed', documentId, error: toCoreError(cause) });
      }
    },
    [flush],
  );

  const reload = useCallback(async (documentId: DocumentId) => {
    const pending = timers.current.get(documentId);
    if (pending) {
      clearTimeout(pending);
      timers.current.delete(documentId);
    }
    try {
      const content = await core.documents.reload({ documentId });
      dispatch({ type: 'reloaded', content });
    } catch (cause: unknown) {
      dispatch({ type: 'updateFailed', documentId, error: toCoreError(cause) });
    }
  }, []);

  const close = useCallback(async (documentId: DocumentId, force = false) => {
    const pending = timers.current.get(documentId);
    if (pending) {
      clearTimeout(pending);
      timers.current.delete(documentId);
    }
    try {
      await core.documents.close(force ? { documentId, force } : { documentId });
      dispatch({ type: 'closed', documentId });
    } catch (cause: unknown) {
      dispatch({ type: 'closeFailed', documentId, error: toCoreError(cause) });
    }
  }, []);

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
    edit,
    save,
    reload,
    close,
    enableEditing,
    activate,
    keepBuffer,
    checkStatus,
    clearTabError,
    dismissOrphan,
    clearError,
  };
}

export type DocumentsController = ReturnType<typeof useDocuments>;
