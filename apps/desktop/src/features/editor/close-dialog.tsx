import { useEffect, useRef } from 'react';
import type { DocumentTab } from '../documents/documents-reducer';
import { fileName } from './file-name';

/** Closing a tab with unsaved changes: the user decides, nothing is dropped silently. */
export function CloseDialog({
  tab,
  busy,
  onSave,
  onDiscard,
  onCancel,
}: {
  tab: DocumentTab;
  busy: boolean;
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  const saveButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    saveButton.current?.focus();
  }, []);
  const name = fileName(tab.document.path);

  return (
    <div className="dialog__backdrop">
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="close-dialog-title"
        aria-describedby="close-dialog-text"
        onKeyDown={(event) => {
          if (event.key === 'Escape') onCancel();
        }}
      >
        <h3 id="close-dialog-title">Close {name}?</h3>
        <p id="close-dialog-text">
          <code>{tab.document.path}</code> has unsaved changes. Save them, discard them, or keep the
          document open.
        </p>
        {tab.error && (
          <p className="dialog__error">
            Last error: {tab.error.code}: {tab.error.message}
          </p>
        )}
        <div className="dialog__actions">
          <button type="button" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button type="button" className="dialog__danger" onClick={onDiscard} disabled={busy}>
            Discard changes
          </button>
          <button type="button" ref={saveButton} onClick={onSave} disabled={busy}>
            {busy ? 'Saving…' : 'Save and close'}
          </button>
        </div>
      </div>
    </div>
  );
}
