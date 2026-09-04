import {
  dominantEol,
  type EditorPreferences,
  type EolTarget,
  isEditableEol,
  normalizeEol,
} from '@gritty/editor';
import { isInstructionFile } from '@gritty/markdown';
import { type DocumentId, toCoreError } from '@gritty/protocol';
import { type Dispatch, useCallback, useEffect, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { core } from '../../core/client';
import {
  activeTab,
  type DocumentTab,
  isUnsaved,
  type PendingExternalState,
  pendingExternalState,
} from '../documents/documents-reducer';
import type { DocumentsController } from '../documents/use-documents';
import { MarkdownPreview } from '../markdown/markdown-preview';
import { MarkdownToolbar } from '../markdown/markdown-toolbar';
import type { PreviewLinkAction } from '../markdown/preview-links';
import { type ViewModes, type ViewModesAction, viewModeFor } from '../markdown/view-modes';
import { RecoveryPrompt } from '../session/recovery-prompt';
import type { SessionController } from '../session/use-session';
import { CloseDialog } from './close-dialog';
import { workerStatus } from './monaco-setup';
import { OrphansPanel } from './orphans-panel';
import { PreferencesPopover } from './preferences-popover';
import { StatusBar } from './status-bar';
import { TabStrip } from './tab-strip';
import type { EditorBinding } from './use-editor-binding';

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
 * Preferences, view modes and the editor binding are owned by `App` so the
 * session (T060) can restore and persist them.
 */
export function EditorPane({
  documents,
  editor,
  prefs,
  onPrefsChange,
  modes,
  dispatchMode,
  session,
}: {
  documents: DocumentsController;
  editor: EditorBinding;
  prefs: EditorPreferences;
  onPrefsChange: (patch: Partial<EditorPreferences>) => void;
  modes: ViewModes;
  dispatchMode: Dispatch<ViewModesAction>;
  session: SessionController;
}) {
  const { state } = documents;
  const [closing, setClosing] = useState<DocumentId | null>(null);
  const [closingBusy, setClosingBusy] = useState(false);
  const [workers, setWorkers] = useState(workerStatus.snapshot());
  useEffect(() => workerStatus.subscribe(setWorkers), []);

  const active = activeTab(state);
  const activeEol = active ? (editor.eols[active.document.id] ?? null) : null;
  const closingTab = closing === null ? null : (state.tabs[closing] ?? null);

  // Markdown: link outcomes and notices; the view modes come from `App`.
  const [brokenTargets, setBrokenTargets] = useState<ReadonlySet<string>>(() => new Set());
  const [previewNotice, setPreviewNotice] = useState<string | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const isMarkdown =
    active !== null && active.document.languageId === 'markdown' && active.text !== null;
  const mode = active
    ? viewModeFor(modes, active.document.id, active.document.languageId)
    : 'source';
  const showPreview = isMarkdown && mode !== 'source';
  const showEditor = active !== null && active.text !== null && !(isMarkdown && mode === 'preview');

  const followLink = useCallback(
    async (action: NonNullable<PreviewLinkAction>) => {
      const from = activeRef.current;
      switch (action.kind) {
        case 'document': {
          if (!from) return;
          const result = await documents.openFile(from.document.workspaceId, action.path, {
            quiet: true,
          });
          if (!result.ok) {
            if (result.error.code === 'NotFound') {
              setBrokenTargets((previous) => new Set(previous).add(action.path));
            }
            setPreviewNotice(`${action.path}: ${result.error.message}`);
            return;
          }
          // A target shown preview-only has no editor to reveal in.
          if (modes[result.documentId] === 'preview') {
            dispatchMode({ type: 'set', documentId: result.documentId, mode: 'split' });
          }
          if (action.line !== undefined) {
            editor.revealPosition(result.documentId, {
              lineNumber: action.line,
              column: action.column ?? 1,
            });
          }
          return;
        }
        case 'external':
          try {
            await core.shell.openExternal(action.url);
          } catch (cause: unknown) {
            setPreviewNotice(`Could not open ${action.url}: ${toCoreError(cause).message}`);
          }
          return;
        case 'unsupported':
          setPreviewNotice(action.reason);
          return;
        case 'anchor':
          return;
      }
    },
    [documents.openFile, editor.revealPosition, modes, dispatchMode],
  );

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
        <PreferencesPopover prefs={prefs} onChange={onPrefsChange} />
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

      <RecoveryPrompt session={session} />

      {active && <Banners tab={active} eol={activeEol} documents={documents} />}

      {isMarkdown && active && (
        <MarkdownToolbar
          mode={mode}
          onMode={(next) =>
            dispatchMode({ type: 'set', documentId: active.document.id, mode: next })
          }
          instructionFile={isInstructionFile(active.document.path)}
          notice={previewNotice}
          onDismissNotice={() => setPreviewNotice(null)}
        />
      )}

      <div className="editor__surfaces">
        <section
          ref={editor.containerRef}
          className="editor__monaco"
          hidden={!showEditor}
          aria-label={active ? `Contents of ${active.document.path}` : 'Editor'}
        />
        {showPreview && active && (
          <MarkdownPreview
            key={active.document.id}
            documentId={active.document.id}
            path={active.document.path}
            largeFileMode={active.document.largeFileMode}
            fallbackText={active.text}
            editor={editor}
            brokenTargets={brokenTargets}
            onLink={(action) => void followLink(action)}
            onNotice={setPreviewNotice}
          />
        )}
      </div>
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
