import { ErrorEnvelope } from '../../components/error-envelope';
import type { useFileDrop } from './use-file-drop';

/**
 * The drop target and what a drop had to ask about.
 *
 * The overlay exists because a drag with no feedback is indistinguishable from
 * a window that ignores drags; it follows `DropHoverChanged`, which the core
 * emits on enter and leave only.
 */
export function DropOverlay({ drop }: { drop: ReturnType<typeof useFileDrop> }) {
  const { hovering, notice } = drop;
  return (
    <>
      {hovering && (
        <div className="drop-overlay" aria-hidden>
          <p className="drop-overlay__label">Drop a file to open it, a folder to work in it</p>
        </div>
      )}
      {notice && (
        <div className="drop-notice" role="status">
          {notice.kind === 'offerFolder' && (
            <>
              <p>
                {describeFiles(notice.files)} outside the open workspace. Gritty opens files through
                a workspace, so its folder has to be the workspace.
              </p>
              <div className="drop-notice__actions">
                <button type="button" onClick={() => drop.acceptFolder(notice.path)}>
                  Open {basename(notice.path)}
                </button>
                <button type="button" onClick={drop.dismiss}>
                  Cancel
                </button>
              </div>
            </>
          )}
          {notice.kind === 'confirmReplace' && (
            <>
              <p>
                Opening {basename(notice.path)} would replace this workspace, and there are unsaved
                changes.
              </p>
              <div className="drop-notice__actions">
                <button type="button" onClick={drop.confirmReplace}>
                  Discard changes and open
                </button>
                <button type="button" onClick={drop.dismiss}>
                  Keep editing
                </button>
              </div>
            </>
          )}
          {notice.kind === 'done' && (
            <>
              <p>{notice.message}</p>
              <div className="drop-notice__actions">
                <button type="button" onClick={drop.dismiss}>
                  Dismiss
                </button>
              </div>
            </>
          )}
          {notice.kind === 'error' && (
            <>
              <ErrorEnvelope error={notice.error} />
              <div className="drop-notice__actions">
                <button type="button" onClick={drop.dismiss}>
                  Dismiss
                </button>
              </div>
            </>
          )}
        </div>
      )}
    </>
  );
}

function describeFiles(files: string[]): string {
  if (files.length === 1) return `${files[0]} is`;
  if (files.length === 2) return `${files[0]} and ${files[1]} are`;
  return `${files[0]} and ${files.length - 1} more are`;
}

function basename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
