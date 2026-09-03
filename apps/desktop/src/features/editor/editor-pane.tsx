import { dominantEol, type EolTarget, isEditableEol, normalizeEol } from '@gritty/editor';
import type { DocumentId } from '@gritty/protocol';
import { useCallback, useEffect, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import {
  activeTab,
  type DocumentTab,
  isUnsaved,
  type PendingExternalState,
  pendingExternalState,
} from '../documents/documents-reducer';
import type { DocumentsController } from '../documents/use-documents';
import { CloseDialog } from './close-dialog';
import { useEditorPreferences } from './editor-preferences';
import { workerStatus } from './monaco-setup';
import { OrphansPanel } from './orphans-panel';
import { PreferencesPopover } from './preferences-popover';
import { StatusBar } from './status-bar';
import { TabStrip } from './tab-strip';
import { useEditorBinding } from './use-editor-binding';

const EXTERNAL_MESSAGES: Record<PendingExternalState, string> = {
  MODIFIED_EXTERNALLY: 'This file changed on disk. Your buffer has no unsaved edits.',
  CONFLICTED:
    'This file changed on disk and your buffer has unsaved edits. Reload drops your edits; saving overwrites the disk.',
  DELETED_EXTERNALLY: 'This file was deleted on disk. Saving recreates it.',
};

/**
 * The Monaco editing surface (T040): tabs, one editor bound to the core's
 * buffers, the external-change / conflict banners, placeholders for binary
 * and large documents, the mixed line-ending notice, and the status bar.
 */
export function EditorPane({ documents }: { documents: DocumentsController }) {
  const { state } = documents;
  const { prefs, update } = useEditorPreferences();
  const editor = useEditorBinding(documents, prefs);
  const [closing, setClosing] = useState<DocumentId | null>(null);
  const [closingBusy, setClosingBusy] = useState(false);
  const [workers, setWorkers] = useState(workerStatus.snapshot());
  useEffect(() => workerStatus.subscribe(setWorkers), []);

  const active = activeTab(state);
  const activeEol = active ? (editor.eols[active.document.id] ?? null) : null;
  const closingTab = closing === null ? null : (state.tabs[closing] ?? null);

  const requestClose = useCallback(
    (documentId: DocumentId) => {
      const tab = state.tabs[documentId];
      if (!tab) return;
      if (isUnsaved(tab)) setClosing(documentId);
      else void documents.close(documentId);
    },
    [state.tabs, documents.close],
  );

  // Cmd/Ctrl+S with the focus outside Monaco (tab strip, buttons); inside,
  // the editor's own command handles it.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      if (event.key.toLowerCase() !== 's') return;
      const container = editor.containerRef.current;
      if (container && event.target instanceof Node && container.contains(event.target)) return;
      event.preventDefault();
      if (state.activeId !== null) void documents.save(state.activeId);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [state.activeId, documents.save, editor.containerRef]);

  return (
    <section className="editor" aria-labelledby="editor-title">
      <div className="editor__bar">
        <h2 id="editor-title" className="editor__title">
          Editor
        </h2>
        <TabStrip
          state={state}
          onActivate={documents.activate}
          onClose={requestClose}
          onReorder={documents.reorder}
        />
        <PreferencesPopover prefs={prefs} onChange={update} />
      </div>

      {documents.error && (
        <ErrorEnvelope error={documents.error} onDismiss={documents.clearError} />
      )}
      {workers.failures.length > 0 && (
        <div className="editor__warning" role="status">
          <p>
            Monaco workers failed to start; the editor works with degraded language features (no
            diagnostics, slower find). Details are in the console.
          </p>
          <ul>
            {workers.failures.map((failure) => (
              <li key={`${failure.phase}:${failure.label}:${failure.reason}`}>
                <code>{failure.label}</code> ({failure.phase}): {failure.reason}
              </li>
            ))}
          </ul>
        </div>
      )}

      {active && <Banners tab={active} eol={activeEol} documents={documents} />}

      <section
        ref={editor.containerRef}
        className="editor__monaco"
        hidden={!active || active.text === null}
        aria-label={active ? `Contents of ${active.document.path}` : 'Editor'}
      />
      {!active && (
        <p className="editor__empty">
          {state.order.length === 0
            ? 'No document open. Click a file in the explorer.'
            : 'Select a tab.'}
        </p>
      )}
      {active?.document.binary && (
        <p className="editor__placeholder">
          Binary file ({active.document.size.toLocaleString()} bytes). It is not shown and cannot be
          edited; saving is disabled so it can never be corrupted.
        </p>
      )}

      {active && (
        <StatusBar
          tab={active}
          eol={activeEol}
          subscribeCursor={editor.subscribeCursor}
          documents={documents}
          onClose={requestClose}
        />
      )}

      <OrphansPanel orphans={state.orphans} onDismiss={documents.dismissOrphan} />

      {closingTab && (
        <CloseDialog
          tab={closingTab}
          busy={closingBusy}
          onCancel={() => setClosing(null)}
          onDiscard={() => {
            setClosing(null);
            void documents.close(closingTab.document.id, true);
          }}
          onSave={() => {
            setClosingBusy(true);
            void documents
              .save(closingTab.document.id)
              .then(async (saved) => {
                if (saved) await documents.close(closingTab.document.id);
              })
              .finally(() => {
                setClosingBusy(false);
                setClosing(null);
              });
          }}
        />
      )}
    </section>
  );
}

function Banners({
  tab,
  eol,
  documents,
}: {
  tab: DocumentTab;
  eol: ReturnType<typeof dominantEol> | 'cr' | 'mixed' | 'none' | null;
  documents: DocumentsController;
}) {
  const { document } = tab;
  const id = document.id;
  const external = pendingExternalState(tab);
  const closeRefused = tab.error?.code === 'Conflict' && isUnsaved(tab);
  const mixed = eol !== null && !isEditableEol(eol) && tab.text !== null;
  const suggested: EolTarget = tab.text === null ? 'lf' : dominantEol(tab.text);

  return (
    <>
      {external && (
        <div className="editor__banner" role="status">
          <p>{EXTERNAL_MESSAGES[external]}</p>
          <div className="editor__banner-actions">
            {external !== 'DELETED_EXTERNALLY' && (
              <button type="button" onClick={() => void documents.reload(id)}>
                Reload from disk
              </button>
            )}
            <button type="button" onClick={() => documents.keepBuffer(id)}>
              Keep my buffer
            </button>
            {external !== 'MODIFIED_EXTERNALLY' && (
              <button type="button" onClick={() => void documents.save(id, true)}>
                {external === 'DELETED_EXTERNALLY' ? 'Recreate file' : 'Overwrite disk'}
              </button>
            )}
          </div>
        </div>
      )}

      {mixed && tab.text !== null && (
        <div className="editor__banner editor__banner--notice" role="status">
          <p>
            This file mixes line endings ({eol === 'cr' ? 'CR only' : 'LF, CRLF or CR'}), which the
            editor cannot keep verbatim. It is shown normalized and read-only; normalizing it
            rewrites every line ending and marks the document dirty until you save.
          </p>
          <div className="editor__banner-actions">
            {(['lf', 'crlf'] as const).map((target) => (
              <button
                key={target}
                type="button"
                disabled={document.readOnly || tab.syncing}
                onClick={() => void documents.replaceText(id, normalizeEol(tab.text ?? '', target))}
              >
                Normalize to {target.toUpperCase()}
                {target === suggested ? ' (dominant)' : ''}
              </button>
            ))}
            {document.readOnly && <span className="editor__hint">Enable editing first.</span>}
          </div>
        </div>
      )}

      {tab.notice && (
        <p className="editor__notice" role="status">
          {tab.notice}
          <button type="button" onClick={() => documents.clearTabError(id)}>
            Dismiss
          </button>
        </p>
      )}
      {tab.error && (
        <>
          <ErrorEnvelope error={tab.error} onDismiss={() => documents.clearTabError(id)} />
          {closeRefused && (
            <button
              type="button"
              className="editor__danger"
              onClick={() => void documents.close(id, true)}
            >
              Discard unsaved changes and close
            </button>
          )}
        </>
      )}

      {document.largeFileMode && document.readOnly && !document.binary && (
        <p className="editor__placeholder">
          Large file ({document.size.toLocaleString()} bytes): opened read-only with minimap,
          folding, word wrap and highlighting off.{' '}
          <button type="button" onClick={() => documents.enableEditing(id)}>
            Enable editing
          </button>
        </p>
      )}
    </>
  );
}
