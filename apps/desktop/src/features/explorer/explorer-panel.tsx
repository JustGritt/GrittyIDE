import { isInstructionFile } from '@gritty/markdown';
import type { DirEntry, WorkspaceClosedReason, WorkspaceId } from '@gritty/protocol';
import { type FormEvent, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import {
  countHidden,
  type ExplorerNode,
  type ExplorerRow,
  isExpandable,
  selectRows,
} from './explorer-reducer';
import { useExplorer } from './use-explorer';

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
 */
export function ExplorerPanel({
  onOpenFile,
}: {
  onOpenFile?: (workspaceId: WorkspaceId, path: string) => void;
}) {
  const explorer = useExplorer();
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
    onOpenFile && workspaceId ? (path: string) => onOpenFile(workspaceId, path) : undefined;

  return (
    <section className="explorer" aria-labelledby="explorer-title">
      <div className="explorer__toolbar">
        <h2 id="explorer-title">Explorer</h2>
        <button type="button" onClick={() => void explorer.openFolder()}>
          Open folder…
        </button>
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
        {state.workspace && (
          <button type="button" onClick={() => void explorer.closeWorkspace()}>
            Close
          </button>
        )}
        <label className="explorer__hidden">
          <input
            type="checkbox"
            checked={state.showHidden}
            onChange={(e) => explorer.setShowHidden(e.target.checked)}
          />
          Show hidden{hiddenCount > 0 && !state.showHidden ? ` (${hiddenCount})` : ''}
        </label>
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
          {explorer.error.code === 'Conflict' && (
            <p className="explorer__notice" role="status">
              Documents have unsaved changes. Save or close them first, or
              <button type="button" onClick={() => void explorer.discardAndRetry()}>
                discard them and continue
              </button>
            </p>
          )}
        </>
      )}

      {state.workspace ? (
        <>
          <p className="explorer__workspace">
            <strong>{state.workspace.name}</strong>
            <code>{state.workspace.root}</code>
          </p>
          <ul className="tree" aria-label="Workspace files">
            {rows.map((row) => (
              <TreeRow
                key={rowKey(row)}
                row={row}
                onToggle={explorer.toggle}
                onReload={explorer.reload}
                onLoadMore={explorer.loadMore}
                onCancel={explorer.cancelListing}
                onOpenFile={openFile}
              />
            ))}
          </ul>
          {rows.length === 0 && <p className="explorer__empty">This directory is empty.</p>}
        </>
      ) : (
        <p className="explorer__empty">No workspace open.</p>
      )}
    </section>
  );
}

function rowKey(row: ExplorerRow): string {
  return `${row.kind}:${row.node.path}`;
}

function TreeRow({
  row,
  onToggle,
  onReload,
  onLoadMore,
  onCancel,
  onOpenFile,
}: {
  row: ExplorerRow;
  onToggle: (node: ExplorerNode) => void;
  onReload: (node: ExplorerNode) => void;
  onLoadMore: (node: ExplorerNode) => void;
  onCancel: (node: ExplorerNode) => Promise<void>;
  onOpenFile?: (path: string) => void;
}) {
  const indent = { paddingLeft: `${0.5 + row.depth * 1.1}rem` };
  switch (row.kind) {
    case 'entry': {
      const { node, entry } = row;
      const expandable = isExpandable(entry);
      const openable = onOpenFile !== undefined && isOpenableFile(entry);
      return (
        <li className={`tree__row tree__row--${entry.kind}`} style={indent}>
          <button
            type="button"
            className="tree__toggle"
            onClick={() => onToggle(node)}
            disabled={!expandable}
            aria-expanded={expandable ? node.expanded : undefined}
            aria-label={expandable ? (node.expanded ? 'Collapse' : 'Expand') : undefined}
          >
            {expandable ? (node.expanded ? '▾' : '▸') : ' '}
          </button>
          <span className="tree__glyph" aria-hidden="true">
            {glyph(entry)}
          </span>
          {openable ? (
            <button
              type="button"
              className={`tree__name tree__name--file${entry.hidden ? ' tree__name--hidden' : ''}`}
              onClick={() => onOpenFile(node.path)}
              title={`Open ${node.path}`}
            >
              {entry.name}
            </button>
          ) : (
            <span className={`tree__name${entry.hidden ? ' tree__name--hidden' : ''}`}>
              {entry.name}
            </span>
          )}
          {describe(entry).map((badge) => (
            <span key={badge} className="tree__badge">
              {badge}
            </span>
          ))}
          {entry.size !== undefined && <span className="tree__size">{formatSize(entry.size)}</span>}
        </li>
      );
    }
    case 'more':
      return (
        <li className="tree__row tree__row--more" style={indent}>
          <button type="button" onClick={() => onLoadMore(row.node)}>
            Load {row.remaining.toLocaleString()} more…
          </button>
          <span className="tree__badge">
            {row.node.children?.length.toLocaleString()} of {row.node.total?.toLocaleString()}
          </span>
        </li>
      );
    case 'status': {
      const { node } = row;
      if (node.error) {
        return (
          <li className="tree__row tree__row--error" style={indent}>
            <span>
              {node.error.code}: {node.error.message}
            </span>
            <button type="button" onClick={() => onReload(node)}>
              Retry
            </button>
          </li>
        );
      }
      return (
        <li className="tree__row tree__row--loading" style={indent}>
          <span>{node.pending ? 'Listing (operation)…' : 'Listing…'}</span>
          {node.pending && (
            <button type="button" onClick={() => void onCancel(node)}>
              Cancel
            </button>
          )}
        </li>
      );
    }
  }
}

/** Regular files, and symlinks that resolve to a file inside the root. */
function isOpenableFile(entry: DirEntry): boolean {
  if (entry.kind === 'file') return true;
  return (
    entry.kind === 'symlink' &&
    entry.symlink?.resolvesInsideRoot === true &&
    entry.symlink.targetKind === 'file'
  );
}

function glyph(entry: DirEntry): string {
  switch (entry.kind) {
    case 'dir':
      return '▪';
    case 'symlink':
      return '↗';
    case 'other':
      return '◌';
    default:
      return '·';
  }
}

function describe(entry: DirEntry): string[] {
  const badges: string[] = [];
  if (entry.ignoredHint) badges.push('ignored');
  if (entry.kind === 'symlink' && entry.symlink) {
    if (entry.symlink.broken) badges.push('broken link');
    else if (!entry.symlink.resolvesInsideRoot) badges.push('link outside root');
    else badges.push(`link → ${entry.symlink.targetKind ?? 'unknown'}`);
  }
  if (entry.kind === 'other') badges.push('special');
  if (entry.kind === 'file' && isInstructionFile(entry.name)) badges.push('instructions');
  return badges;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}
