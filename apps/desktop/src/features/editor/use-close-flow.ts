import type { DocumentId } from '@gritty/protocol';
import { useCallback, useState } from 'react';
import { isUnsaved } from '../documents/documents-reducer';
import type { DocumentsController } from '../documents/use-documents';

/**
 * Closing a tab with unsaved edits asks first (T030). The flow lives in a hook
 * because two regions need it: the tab strip and the status bar raise the
 * request, and the shell owns the dialog — the status bar sits at the bottom of
 * the window now (T048), outside the editor pane that used to hold both.
 */
export function useCloseFlow(documents: DocumentsController) {
  const { state } = documents;
  const [closing, setClosing] = useState<DocumentId | null>(null);
  const [busy, setBusy] = useState(false);
  const tab = closing === null ? null : (state.tabs[closing] ?? null);

  const request = useCallback(
    (documentId: DocumentId) => {
      const target = state.tabs[documentId];
      if (!target) return;
      if (isUnsaved(target)) setClosing(documentId);
      else void documents.close(documentId);
    },
    [state.tabs, documents.close],
  );

  const cancel = useCallback(() => setClosing(null), []);

  const discard = useCallback(() => {
    if (!tab) return;
    setClosing(null);
    void documents.close(tab.document.id, true);
  }, [tab, documents.close]);

  const save = useCallback(() => {
    if (!tab) return;
    setBusy(true);
    void documents
      .save(tab.document.id)
      .then(async (saved) => {
        if (saved) await documents.close(tab.document.id);
      })
      .finally(() => {
        setBusy(false);
        setClosing(null);
      });
  }, [tab, documents.save, documents.close]);

  return { tab, busy, request, cancel, discard, save };
}
