import type { DocumentId } from '@gritty/protocol';
import { useCallback, useRef, useState } from 'react';
import type { DocumentsController } from '../documents/use-documents';
import { planClose } from './close-plan';

interface Prompt {
  readonly documentId: DocumentId;
  /** Queued behind this one; dropped if the user cancels. */
  readonly rest: readonly DocumentId[];
}

/**
 * Closing a tab with unsaved edits asks first (T030). The flow lives in a hook
 * because two regions need it: the tab strip and the status bar raise the
 * request, and the shell owns the dialog — the status bar sits at the bottom of
 * the window now (T048), outside the editor pane that used to hold both.
 *
 * A request may name several documents (the tab menu's "close tabs to the
 * right"). They close in order, one dialog per unsaved document; cancelling
 * one keeps it and everything still queued behind it, and a close the core
 * refuses stops the batch there rather than closing past a failure.
 */
export function useCloseFlow(documents: DocumentsController) {
  const { state } = documents;
  const [prompt, setPrompt] = useState<Prompt | null>(null);
  const [busy, setBusy] = useState(false);
  const tab = prompt === null ? null : (state.tabs[prompt.documentId] ?? null);
  // The queue outlives several awaits; React state would be a render behind.
  const stateRef = useRef(state);
  stateRef.current = state;

  /** Close what needs no question, then stop at the first document that does. */
  const run = useCallback(
    async (queue: readonly DocumentId[]) => {
      const plan = planClose(stateRef.current, queue);
      for (const documentId of plan.closeNow) {
        if (!(await documents.close(documentId))) {
          // The tab keeps the error; leave the rest open rather than pressing on.
          setPrompt(null);
          return;
        }
      }
      setPrompt(plan.ask === null ? null : { documentId: plan.ask, rest: plan.rest });
    },
    [documents.close],
  );

  const request = useCallback((documentId: DocumentId) => void run([documentId]), [run]);
  const requestMany = useCallback(
    (documentIds: readonly DocumentId[]) => void run(documentIds),
    [run],
  );

  /** Keep this document and everything queued behind it. */
  const cancel = useCallback(() => setPrompt(null), []);

  const discard = useCallback(() => {
    if (!prompt) return;
    const { documentId, rest } = prompt;
    setPrompt(null);
    void documents.close(documentId, true).then((closed) => {
      if (closed) void run(rest);
    });
  }, [prompt, documents.close, run]);

  const save = useCallback(() => {
    if (!prompt) return;
    const { documentId, rest } = prompt;
    setBusy(true);
    void documents
      .save(documentId)
      .then(async (saved) => {
        setPrompt(null);
        // A refused save keeps the document: the batch stops with its error shown.
        if (!saved) return;
        if (await documents.close(documentId)) await run(rest);
      })
      .finally(() => setBusy(false));
  }, [prompt, documents.save, documents.close, run]);

  return {
    tab,
    busy,
    queued: prompt?.rest.length ?? 0,
    request,
    requestMany,
    cancel,
    discard,
    save,
  };
}
