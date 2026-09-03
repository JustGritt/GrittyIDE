import { type Eol, eolLabel } from '@gritty/editor';
import type { CursorPosition } from '@gritty/editor/monaco';
import type { DocumentId } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { type DocumentTab, isUnsaved, pendingExternalState } from '../documents/documents-reducer';
import type { DocumentsController } from '../documents/use-documents';

/**
 * Everything the contract knows about the active document, plus the cursor.
 * The cursor lives here as local state so a cursor move re-renders this bar only.
 */
export function StatusBar({
  tab,
  eol,
  subscribeCursor,
  documents,
  onClose,
}: {
  tab: DocumentTab;
  eol: Eol | null;
  subscribeCursor: (listener: (position: CursorPosition) => void) => () => void;
  documents: DocumentsController;
  onClose: (documentId: DocumentId) => void;
}) {
  const [cursor, setCursor] = useState<CursorPosition | null>(null);
  useEffect(() => subscribeCursor(setCursor), [subscribeCursor]);
  const { document } = tab;
  const id = document.id;
  const unsaved = isUnsaved(tab);
  const external = pendingExternalState(tab);

  return (
    <div className="statusbar">
      <code className="statusbar__path" title={document.path}>
        {document.path}
      </code>
      <span
        className={`statusbar__state statusbar__state--${document.externalModificationState.toLowerCase()}`}
      >
        {document.externalModificationState}
      </span>
      <span>
        {document.dirty ? 'dirty' : 'clean'}
        {tab.pending || tab.draft !== null ? ' · edits pending' : ''}
        {tab.syncing ? ' · syncing…' : ''}
        {tab.saving ? ' · saving…' : ''}
      </span>
      {document.readOnly && <span>read-only</span>}
      {document.largeFileMode && <span>large file</span>}
      <span className="statusbar__spacer" />
      {cursor && !document.binary && (
        <span>
          Ln {cursor.lineNumber}, Col {cursor.column}
        </span>
      )}
      <span title="line endings">{eol === null ? '–' : eolLabel(eol)}</span>
      <span title="encoding">{document.encoding}</span>
      <span title="language">{document.languageId}</span>
      <span title="version · saved revision">
        v{document.currentVersion} · r{document.savedRevision}
      </span>
      <span title="size on disk">{document.size.toLocaleString()} B</span>
      <div className="statusbar__actions">
        <button
          type="button"
          onClick={() => void documents.save(id)}
          disabled={document.binary || tab.saving || (!unsaved && external === null)}
        >
          Save
        </button>
        <button type="button" onClick={() => void documents.reload(id)} disabled={tab.saving}>
          Reload
        </button>
        <button type="button" onClick={() => void documents.checkStatus()}>
          Check disk
        </button>
        <button type="button" onClick={() => onClose(id)}>
          Close
        </button>
      </div>
    </div>
  );
}
