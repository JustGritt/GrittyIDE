import type { WorkspaceClosedReason, WorkspaceId } from '@gritty/protocol';
import { type FormEvent, type ReactNode, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { FolderIcon } from '../git/sidebar-icons';
import { type SidebarTab, SidebarTabs, sidebarPanelId, sidebarTabId } from '../git/sidebar-tabs';
import { countHidden, selectRows } from './explorer-reducer';
import { ExplorerTree } from './explorer-tree';
import type { ExplorerController } from './use-explorer';

const CLOSED_REASONS: Record<WorkspaceClosedReason, string> = {
  USER: 'closed',
  REPLACED: 'replaced by another workspace',
  ROOT_MISSING: 'closed because its root directory no longer exists',
};

/**
 * Lazy workspace tree: one `workspace.list` per expanded directory, hidden
 * entries behind a toggle, truncated directories continued on demand, and
 * listings that became operations shown with a cancel button. Clicking a file
 * hands it to `onOpenFile` (T030 documents).
 *
 * Since T100-U it is also the frame the sidebar's two sections share: the
 * header is a tablist, and the tree is one of the two panels it switches
 * between. The other one is passed in as `changes` rather than imported here,
 * so the git feature keeps its own directory and this file keeps owning only
 * the frame and the tree.
 */
export function ExplorerPanel({
  explorer,
  onOpenFile,
  tab,
  onTabChange,
  changes,
}: {
  /**
   * Owned by the shell (T049), not created here: the launcher opens folders
   * through the same controller and the top bar reads its workspace, and two
   * instances would mean two subscriptions and two trees.
   */
  explorer: ExplorerController;
  /** `preview` is a single click — open it for a look, not to keep. */
  onOpenFile?: (workspaceId: WorkspaceId, path: string, preview: boolean) => void;
  /** Which section is showing; owned by the shell, like every other layout choice. */
  tab: SidebarTab;
  onTabChange: (tab: SidebarTab) => void;
  /**
   * The Changes panel, rendered only while its tab is selected — so nothing
   * reads the repository while the tree is showing.
   */
  changes?: ReactNode;
}) {
  const { state } = explorer;
  const [pathInput, setPathInput] = useState('');

  function onOpenPath(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const path = pathInput.trim();
    if (path !== '') void explorer.openFolder(path);
  }

  const rows = state.workspace ? selectRows(state) : [];
  const hiddenCount = countHidden(state);
  const workspaceId = state.workspace?.id;
  const openFile =
    onOpenFile && workspaceId
      ? (path: string, preview: boolean) => onOpenFile(workspaceId, path, preview)
      : undefined;

  return (
    <section className="explorer" aria-label="Files and changes">
      {/* A 32px header (T048): the section tabs, the one action you reach for,
          and the rest behind the same disclosure the editor and terminal
          settings use.

          T233-S restyled the label as artboard 9a's section tab, renamed it
          Files, and left it an `<h2>` on purpose: 9a draws a second tab,
          Changes, and nothing was behind it, because no git command reached
          the renderer. T100-U is that second tab, so the heading is now what
          9a drew — a real tablist, keyboard-navigable, with the tree and the
          changes list as its two panels. `sidebar-tabs.tsx` holds the tab
          semantics; the panels are below. */}
      <div className="explorer__toolbar">
        <SidebarTabs tab={tab} onTabChange={onTabChange} />
        <span className="explorer__spacer" />
        {tab === 'files' && hiddenCount > 0 && !state.showHidden && (
          <span className="badge">{hiddenCount} hidden</span>
        )}
        <button
          type="button"
          className="explorer__icon-button"
          aria-label="Open folder…"
          title="Open folder…"
          onClick={() => void explorer.openFolder()}
        >
          <FolderIcon />
        </button>
        <details className="explorer__more">
          <summary aria-label="Workspace options">⋯</summary>
          <div className="explorer__menu">
            <form className="explorer__path" onSubmit={onOpenPath}>
              <input
                value={pathInput}
                onChange={(e) => setPathInput(e.target.value)}
                placeholder="absolute path"
                aria-label="Workspace path"
                spellCheck={false}
              />
              <button type="submit">Open path</button>
            </form>
            <label className="explorer__hidden">
              <input
                type="checkbox"
                checked={state.showHidden}
                onChange={(e) => explorer.setShowHidden(e.target.checked)}
              />
              Show hidden{hiddenCount > 0 && !state.showHidden ? ` (${hiddenCount})` : ''}
            </label>
            {state.workspace && (
              <button type="button" onClick={() => void explorer.closeWorkspace()}>
                Close workspace
              </button>
            )}
          </div>
        </details>
      </div>

      {state.notice && (
        <p className="explorer__notice" role="status">
          Workspace <code>{state.notice.root}</code> was {CLOSED_REASONS[state.notice.reason]}.
          <button type="button" onClick={explorer.dismissNotice}>
            Dismiss
          </button>
        </p>
      )}
      {explorer.error && (
        <>
          <ErrorEnvelope error={explorer.error} onDismiss={explorer.clearError} />
          {/* Only a dirty-document refusal: every other `Conflict` (a taken
              name, an empty undo) is not about unsaved work, and the button
              would replay something with `force` (EXPLORER-DISCARD-NOTICE). */}
          {explorer.canDiscardAndRetry && (
            <p className="explorer__notice" role="status">
              Documents have unsaved changes. Save or close them first, or
              <button type="button" onClick={() => void explorer.discardAndRetry()}>
                discard them and continue
              </button>
            </p>
          )}
        </>
      )}

      {/* Only the selected panel exists in the document. That is what keeps the
          Changes tab from reading the repository while the tree is showing —
          `git.status` is polled on demand (T100 rules watcher integration out),
          so "not mounted" is the same thing as "not polling". */}
      {tab === 'files' ? (
        <div
          className="explorer__panel"
          role="tabpanel"
          id={sidebarPanelId('files')}
          aria-labelledby={sidebarTabId('files')}
        >
          {state.workspace ? (
            <>
              {/* The rows live in `explorer-tree.tsx` since T042: this file
                  owns the frame — the section tabs, the toolbar, the notices —
                  and the tree owns selection, the keyboard and the file
                  operations. Splitting them is what lets the frame and the rows
                  be worked on without the two colliding. */}
              <ExplorerTree explorer={explorer} onOpenFile={openFile} />
              {rows.length === 0 && <p className="explorer__empty">This directory is empty.</p>}
            </>
          ) : (
            <p className="explorer__empty">No workspace open.</p>
          )}
        </div>
      ) : (
        <div
          className="explorer__panel"
          role="tabpanel"
          id={sidebarPanelId('changes')}
          aria-labelledby={sidebarTabId('changes')}
        >
          {changes}
        </div>
      )}
    </section>
  );
}
