import type { DocumentId } from '@gritty/protocol';
import { ErrorEnvelope } from '../../components/error-envelope';
import {
  activeTab,
  type DocumentTab,
  displayText,
  isUnsaved,
  type PendingExternalState,
  pendingExternalState,
} from './documents-reducer';
import type { DocumentsController } from './use-documents';

const EXTERNAL_MESSAGES: Record<PendingExternalState, string> = {
  MODIFIED_EXTERNALLY: 'This file changed on disk. Your buffer has no unsaved edits.',
  CONFLICTED:
    'This file changed on disk and your buffer has unsaved edits. Reload drops your edits; saving overwrites the disk.',
  DELETED_EXTERNALLY: 'This file was deleted on disk. Saving recreates it.',
};

/**
 * Minimal editing surface until Monaco lands (T040): tabs, a plain textarea
 * bound to the core's buffer, save / reload / keep decisions for external
 * changes, and placeholders for binary and large documents.
 */
export function DocumentPanel({ documents }: { documents: DocumentsController }) {
  const { state } = documents;
  const active = activeTab(state);

  return (
    <section className="documents" aria-labelledby="documents-title">
      <h2 id="documents-title" className="documents__title">
        Documents
      </h2>
      {documents.error && (
        <ErrorEnvelope error={documents.error} onDismiss={documents.clearError} />
      )}
      {state.order.length > 0 && (
        <div className="documents__tabs" role="tablist" aria-label="Open documents">
          {state.order.map((id) => {
            const tab = state.tabs[id];
            if (!tab) return null;
            const isActive = id === state.activeId;
            return (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={isActive}
                className={`documents__tab${isActive ? ' documents__tab--active' : ''}${
                  isUnsaved(tab) ? ' documents__tab--unsaved' : ''
                }`}
                onClick={() => documents.activate(id)}
                title={tab.document.path}
              >
                {isUnsaved(tab) ? '● ' : ''}
                {fileName(tab.document.path)}
              </button>
            );
          })}
        </div>
      )}
      {active ? (
        <Editor tab={active} documents={documents} />
      ) : (
        <p className="documents__empty">
          {state.order.length === 0
            ? 'No document open. Click a file in the explorer.'
            : 'Select a tab.'}
        </p>
      )}
      {state.orphans.length > 0 && (
        <div className="documents__orphans">
          <h3>Unsaved buffers from closed documents</h3>
          <p className="documents__hint">
            These documents were closed by the core with unsaved changes (workspace closed or root
            missing). Copy what you need; nothing was written to disk.
          </p>
          {state.orphans.map((orphan) => (
            <details key={orphan.id} className="documents__orphan">
              <summary>
                <code>{orphan.path}</code> ({orphan.reason.toLowerCase().replace('_', ' ')})
                <button type="button" onClick={() => documents.dismissOrphan(orphan.id)}>
                  Dismiss
                </button>
              </summary>
              <textarea readOnly value={orphan.text} rows={8} spellCheck={false} />
            </details>
          ))}
        </div>
      )}
    </section>
  );
}

function Editor({ tab, documents }: { tab: DocumentTab; documents: DocumentsController }) {
  const { document } = tab;
  const id: DocumentId = document.id;
  const external = pendingExternalState(tab);
  const text = displayText(tab);
  const unsaved = isUnsaved(tab);
  const closeRefused = tab.error?.code === 'Conflict' && unsaved;

  return (
    <div className="documents__editor">
      <dl className="documents__meta">
        <dt>path</dt>
        <dd>
          <code>{document.path}</code>
        </dd>
        <dt>language</dt>
        <dd>{document.languageId}</dd>
        <dt>encoding</dt>
        <dd>{document.encoding}</dd>
        <dt>version</dt>
        <dd>
          {document.currentVersion} (saved revision {document.savedRevision})
        </dd>
        <dt>size</dt>
        <dd>{document.size.toLocaleString()} B</dd>
        <dt>state</dt>
        <dd>
          <span
            className={`documents__state documents__state--${document.externalModificationState.toLowerCase()}`}
          >
            {document.externalModificationState}
          </span>
          {document.dirty ? ' · dirty' : ' · clean'}
          {tab.draft !== null ? ' · draft pending' : ''}
          {tab.syncing ? ' · syncing…' : ''}
          {tab.saving ? ' · saving…' : ''}
          {document.readOnly ? ' · read-only' : ''}
          {document.largeFileMode ? ' · large file' : ''}
        </dd>
      </dl>

      {external && (
        <div className="documents__banner" role="status">
          <p>{EXTERNAL_MESSAGES[external]}</p>
          <div className="documents__banner-actions">
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

      {tab.notice && (
        <p className="documents__notice" role="status">
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
              className="documents__danger"
              onClick={() => void documents.close(id, true)}
            >
              Discard unsaved changes and close
            </button>
          )}
        </>
      )}

      {document.binary ? (
        <p className="documents__placeholder">
          Binary file ({document.size.toLocaleString()} bytes). It is not shown and cannot be
          edited; saving is disabled so it can never be corrupted.
        </p>
      ) : (
        <>
          {document.largeFileMode && document.readOnly && (
            <p className="documents__placeholder">
              Large file: opened read-only.{' '}
              <button type="button" onClick={() => documents.enableEditing(id)}>
                Enable editing
              </button>
            </p>
          )}
          <textarea
            className="documents__textarea"
            value={text ?? ''}
            readOnly={document.readOnly}
            aria-label={`Contents of ${document.path}`}
            spellCheck={false}
            wrap="off"
            onChange={(e) => documents.edit(id, e.target.value)}
          />
        </>
      )}

      <div className="documents__actions">
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
        <button type="button" onClick={() => void documents.close(id)}>
          Close
        </button>
      </div>
    </div>
  );
}

function fileName(path: string): string {
  const index = path.lastIndexOf('/');
  return index === -1 ? path : path.slice(index + 1);
}
