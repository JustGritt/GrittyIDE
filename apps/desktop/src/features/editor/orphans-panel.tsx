import type { OrphanedBuffer } from '../documents/documents-reducer';

/**
 * Dirty buffers whose document the core closed (workspace closed or root
 * missing). Nothing was written to disk; the text stays on screen until dismissed.
 */
export function OrphansPanel({
  orphans,
  onDismiss,
}: {
  orphans: readonly OrphanedBuffer[];
  onDismiss: (id: number) => void;
}) {
  if (orphans.length === 0) return null;
  return (
    <div className="orphans">
      <h3>Unsaved buffers from closed documents</h3>
      <p className="orphans__hint">
        These documents were closed by the core with unsaved changes (workspace closed or root
        missing). Copy what you need; nothing was written to disk.
      </p>
      {orphans.map((orphan) => (
        <details key={orphan.id} className="orphans__item">
          <summary>
            <code>{orphan.path}</code> ({orphan.reason.toLowerCase().replace('_', ' ')})
            <button type="button" onClick={() => onDismiss(orphan.id)}>
              Dismiss
            </button>
          </summary>
          <textarea readOnly value={orphan.text} rows={8} spellCheck={false} />
        </details>
      ))}
    </div>
  );
}
