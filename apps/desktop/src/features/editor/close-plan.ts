import type { DocumentId } from '@gritty/protocol';
import { type DocumentsState, isUnsaved } from '../documents/documents-reducer';

/**
 * Which documents a close action touches, and which of them the user has to be
 * asked about first. Closing more than one tab at a time (the tab menu's
 * "close tabs to the right") must not drop an unsaved buffer to save a click,
 * so the batch is a queue: everything up to the first unsaved document closes,
 * that one is asked about, and the rest waits for the answer. Pure, so the
 * ordering and the skipping are unit-tested without React.
 */
export interface ClosePlan {
  /** Saved documents to close straight away, in queue order. */
  readonly closeNow: readonly DocumentId[];
  /** The first document with unsaved edits: it needs the dialog. */
  readonly ask: DocumentId | null;
  /** Queued behind `ask`, replanned once it is resolved. */
  readonly rest: readonly DocumentId[];
}

export function planClose(state: DocumentsState, queue: readonly DocumentId[]): ClosePlan {
  const closeNow: DocumentId[] = [];
  for (let index = 0; index < queue.length; index += 1) {
    const documentId = queue[index];
    if (documentId === undefined) continue;
    const tab = state.tabs[documentId];
    // Already closed (an event arrived mid-batch): nothing to close or ask.
    if (!tab) continue;
    if (isUnsaved(tab)) return { closeNow, ask: documentId, rest: queue.slice(index + 1) };
    closeNow.push(documentId);
  }
  return { closeNow, ask: null, rest: [] };
}

/** Open tabs after `documentId` in tab order; empty when it is the last one. */
export function tabsToTheRight(
  state: DocumentsState,
  documentId: DocumentId,
): readonly DocumentId[] {
  const index = state.order.indexOf(documentId);
  if (index < 0) return [];
  return state.order.slice(index + 1);
}
