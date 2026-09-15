import { type ReactNode, useEffect, useId, useRef } from 'react';

/**
 * A confirmation before something the user should not do by accident.
 *
 * Extracted for T102-U's discard rather than written a third time: the
 * explorer's `DeleteDialog` and the editor's `CloseDialog` are two hand-written
 * copies of this same chrome, and `CONFIRM-DIALOG` moves them onto it.
 *
 * The policy it carries, so a caller cannot forget it:
 *
 * - **Focus starts on Cancel.** Enter on a dialog that just appeared keeps what
 *   the user has; it never confirms the destructive choice.
 * - **Escape cancels**, from anywhere inside the dialog.
 * - **The title and the body are announced**: `alertdialog`, labelled by the
 *   title and described by the body.
 *
 * It says nothing about *when* to ask. That is the caller's policy, and the
 * explorer's is the one to keep: only an action that cannot be recovered asks.
 */
export function ConfirmDialog({
  title,
  children,
  confirmLabel,
  danger = true,
  onConfirm,
  onCancel,
}: {
  title: string;
  /** What will happen, in words. Rendered inside the dialog's description. */
  children: ReactNode;
  confirmLabel: string;
  /** Draw the confirm button as destructive. */
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const id = useId();
  const cancelButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    cancelButton.current?.focus();
  }, []);
  return (
    <div className="dialog__backdrop">
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={`${id}-title`}
        aria-describedby={`${id}-text`}
        onKeyDown={(event) => {
          if (event.key === 'Escape') onCancel();
        }}
      >
        <h3 id={`${id}-title`}>{title}</h3>
        <div id={`${id}-text`}>{children}</div>
        <div className="dialog__actions">
          <button type="button" ref={cancelButton} onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className={danger ? 'dialog__danger' : undefined}
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
