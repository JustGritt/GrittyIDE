import { type Eol, eolLabel } from '@gritty/editor';
import type { CursorPosition } from '@gritty/editor/monaco';
import type { DocumentId, Workspace } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { type DocumentTab, isUnsaved, pendingExternalState } from '../documents/documents-reducer';
import type { DocumentsController } from '../documents/use-documents';
import type { HealthState } from '../shell/app-bar';

/**
 * The last 26px of the window (T048). What is running is on the left, what the
 * contract knows about the active document is on the right, and the cursor
 * lives here as local state so a cursor move re-renders this bar only.
 *
 * The bar is always present — with no document open it still answers "what am I
 * running" — so the document half is what disappears, not the bar.
 */
export function StatusBar({
  health,
  workspace,
  tab,
  eol,
  subscribeCursor,
  documents,
  onClose,
  permissionsWaiting,
}: {
  health: HealthState;
  workspace: Workspace | null;
  tab: DocumentTab | null;
  eol: Eol | null;
  subscribeCursor: (listener: (position: CursorPosition) => void) => () => void;
  documents: DocumentsController;
  onClose: (documentId: DocumentId) => void;
  /**
   * Agents stopped at a permission prompt (T335, artboard 12b).
   *
   * The one thing from 12b's status bar that is earned: `step 4/6` needs runs
   * Gritty owns (T300) and `files held` needs T223's opt-in link, so neither is
   * drawn. This is a *count*, not a name — the bar says something needs you and
   * the queue says what.
   */
  permissionsWaiting: number;
}) {
  const [cursor, setCursor] = useState<CursorPosition | null>(null);
  useEffect(() => subscribeCursor(setCursor), [subscribeCursor]);

  return (
    <div className="statusbar">
      <span className={`statusbar__build statusbar__build--${health.status}`}>
        {buildLabel(health)}
      </span>
      {permissionsWaiting > 0 && (
        <span className="statusbar__permissions">
          {permissionsWaiting === 1
            ? '1 permission waiting'
            : `${permissionsWaiting} permissions waiting`}
        </span>
      )}
      {tab ? (
        <DocumentFacts
          tab={tab}
          eol={eol}
          cursor={cursor}
          documents={documents}
          onClose={onClose}
        />
      ) : (
        <>
          <span className="statusbar__spacer" />
          {/* With nothing open the bar still answers "where am I" (T049). */}
          {workspace === null && <span>no workspace</span>}
        </>
      )}
    </div>
  );
}

function buildLabel(health: HealthState): string {
  if (health.status === 'loading') return 'starting…';
  if (health.status === 'error') return 'core unreachable';
  const { coreVersion, protocolVersion, shell } = health.health;
  return `core ${coreVersion} · ${shell.runtime} ${shell.runtimeVersion} · protocol ${protocolVersion}`;
}

function DocumentFacts({
  tab,
  eol,
  cursor,
  documents,
  onClose,
}: {
  tab: DocumentTab;
  eol: Eol | null;
  cursor: CursorPosition | null;
  documents: DocumentsController;
  onClose: (documentId: DocumentId) => void;
}) {
  const { document } = tab;
  const id = document.id;
  const unsaved = isUnsaved(tab);
  const external = pendingExternalState(tab);

  return (
    <>
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
    </>
  );
}
