import type {
  DiffHunk,
  DiffSummary,
  FileDiff,
  FileStatus,
  RepositoryStatus,
  WorkspacePath,
} from '@gritty/protocol';
import { ErrorEnvelope } from '../../components/error-envelope';
import { FileIcon, fileIconKind } from '../../components/file-icon';
import {
  changeKindWord,
  describeEntry,
  fileDirectory,
  fileName,
  formatBytes,
  groupEntries,
  headLine,
  hunkHeader,
  lineSign,
  numberHunk,
  shortId,
  stateLabel,
  statusCode,
  upstreamLine,
} from './git-model';
import type { GitState, Read } from './git-reducer';
import { HistoryView } from './history-view';

/**
 * Everything the Changes tab draws (T100-U), and nothing that reads anything.
 *
 * Split from `changes-panel.tsx` the way `agent-console.tsx` is split from its
 * controller: the container calls `useGit`, this file takes the state it
 * produced. That is what lets the render tests next door mount every arm of
 * both reads — `pending`, `notARepository`, a refusal — without a transport, a
 * repository, or the core client's event channel being constructed at import.
 * A correct view model with no caller is the defect this repository keeps
 * catching late, and those tests are the answer to it.
 *
 * No artboard exists for a git panel — T100 §10 says so, and says to keep it
 * plain rather than invent one — so nothing here is new: the bar is the
 * explorer's 32px section bar, and the diff rows are `permission-prompt.tsx`'s,
 * which is the other place in the window that draws an add beside a remove.
 * Every value is an ADR-016 token, so the light ramp follows without a second
 * palette.
 */
export function ChangesView({
  state,
  hasWorkspace,
  onRefresh,
  onLoadMore,
  onSelect,
  onReloadDiff,
  onCancelStatus,
  onCancelDiff,
  view,
  onViewChange,
  onLoadMoreHistory,
  onCancelHistory,
}: {
  state: GitState;
  hasWorkspace: boolean;
  onRefresh: () => void;
  onLoadMore: () => void;
  onSelect: (path: WorkspacePath | null) => void;
  onReloadDiff: () => void;
  onCancelStatus: () => void;
  onCancelDiff: () => void;
  /**
   * Which half of the panel is showing. A sub-view inside the Changes tab
   * rather than a third *sidebar* tab: artboard 9a drew two sections, and the
   * working tree and its history are one subject — the same question asked of
   * now and of before.
   */
  view: GitView;
  onViewChange: (view: GitView) => void;
  onLoadMoreHistory: () => void;
  onCancelHistory: () => void;
}) {
  if (!hasWorkspace) {
    return <p className="explorer__empty">No workspace open.</p>;
  }

  const repository = state.status.kind === 'ready' ? state.status.value : null;
  const selectedEntry =
    repository && state.selected !== null
      ? (repository.entries.find((entry) => entry.path === state.selected) ?? null)
      : null;

  return (
    <div className="git">
      <div className="git__bar">
        {repository ? (
          <>
            <span
              className="git__head"
              title="Branch or commit the working tree is measured against"
            >
              {headLine(repository)}
            </span>
            {stateLabel(repository.state) !== null && (
              <span className="badge git__state">{stateLabel(repository.state)}</span>
            )}
            {upstreamLine(repository.upstream) !== null && (
              <span className="git__upstream" title={repository.upstream?.refName}>
                {upstreamLine(repository.upstream)}
              </span>
            )}
          </>
        ) : (
          <span className="git__head">Changes</span>
        )}
        <span className="explorer__spacer" />
        <button type="button" onClick={onRefresh}>
          Refresh
        </button>
      </div>

      <GitViewTabs view={view} onViewChange={onViewChange} />

      {view === 'history' ? (
        <div className="git__list">
          <HistoryView
            state={state.history}
            appending={state.appendingHistory}
            onLoadMore={onLoadMoreHistory}
            onRetry={onRefresh}
            onCancel={onCancelHistory}
          />
        </div>
      ) : (
        <ChangesBody
          state={state}
          selectedEntry={selectedEntry}
          onSelect={onSelect}
          onLoadMore={onLoadMore}
          onRefresh={onRefresh}
          onReloadDiff={onReloadDiff}
          onCancelStatus={onCancelStatus}
          onCancelDiff={onCancelDiff}
        />
      )}
    </div>
  );
}

/** Which half of the git panel is showing. */
export type GitView = 'changes' | 'history';

const GIT_VIEWS: readonly { readonly id: GitView; readonly label: string }[] = [
  { id: 'changes', label: 'Changes' },
  { id: 'history', label: 'History' },
];

/**
 * Two buttons, not a second `role="tablist"`.
 *
 * The sidebar header above is already a tablist (`sidebar-tabs.tsx`), and
 * nesting one inside its own panel gives a screen reader two overlapping tab
 * contexts to describe. A group of pressed/unpressed buttons says the same
 * thing without the ambiguity.
 */
function GitViewTabs({
  view,
  onViewChange,
}: {
  view: GitView;
  onViewChange: (view: GitView) => void;
}) {
  return (
    // A `fieldset`, which is what `role="group"` means natively, with the
    // legend carrying the label a screen reader announces. The reset below
    // strips the border a fieldset draws by default.
    <fieldset className="git__views">
      <legend className="git__views-legend">Working tree or history</legend>
      {GIT_VIEWS.map((candidate) => (
        <button
          key={candidate.id}
          type="button"
          className={`git__view${candidate.id === view ? ' git__view--selected' : ''}`}
          aria-pressed={candidate.id === view}
          onClick={() => onViewChange(candidate.id)}
        >
          {candidate.label}
        </button>
      ))}
    </fieldset>
  );
}

function ChangesBody({
  state,
  selectedEntry,
  onSelect,
  onLoadMore,
  onRefresh,
  onReloadDiff,
  onCancelStatus,
  onCancelDiff,
}: {
  state: GitState;
  selectedEntry: FileStatus | null;
  onSelect: (path: WorkspacePath | null) => void;
  onLoadMore: () => void;
  onRefresh: () => void;
  onReloadDiff: () => void;
  onCancelStatus: () => void;
  onCancelDiff: () => void;
}) {
  return (
    <>
      <div className="git__list">
        <StatusRegion
          status={state.status}
          appending={state.appending}
          selected={state.selected}
          onSelect={onSelect}
          onLoadMore={onLoadMore}
          onRetry={onRefresh}
          onCancel={onCancelStatus}
        />
      </div>

      {state.selected !== null && (
        <div className="git__diff">
          <div className="git__diff-bar">
            <span className="git__diff-path" title={state.selected}>
              {state.selected}
            </span>
            <span className="explorer__spacer" />
            <button type="button" onClick={() => onSelect(null)}>
              Close
            </button>
          </div>
          <DiffRegion
            diff={state.diff}
            path={state.selected}
            entry={selectedEntry}
            onRetry={onReloadDiff}
            onCancel={onCancelDiff}
          />
        </div>
      )}
    </>
  );
}

function StatusRegion({
  status,
  appending,
  selected,
  onSelect,
  onLoadMore,
  onRetry,
  onCancel,
}: {
  status: Read<RepositoryStatus>;
  appending: boolean;
  selected: WorkspacePath | null;
  onSelect: (path: WorkspacePath | null) => void;
  onLoadMore: () => void;
  onRetry: () => void;
  onCancel: () => void;
}) {
  switch (status.kind) {
    case 'idle':
    case 'loading':
      return (
        <p className="git__note" role="status">
          Reading the repository…
        </p>
      );
    case 'pending':
      // The read outlived its budget and became an operation. Saying which,
      // and offering to stop it, is the whole reason `pending` is an arm of
      // the response rather than a longer wait.
      return (
        <p className="git__note" role="status">
          Reading the repository (operation)…
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
        </p>
      );
    case 'cancelled':
      return (
        <p className="git__note" role="status">
          Read cancelled.
          <button type="button" onClick={onRetry}>
            Read again
          </button>
        </p>
      );
    case 'notARepository':
      // A value, not an error: a folder that is not a repository is a normal
      // thing to open, and a red envelope in front of one would be the UI
      // inventing a fault.
      return (
        <div className="git__empty">
          <p>This workspace is not a Git repository.</p>
          <p className="git__aside">
            Gritty opens the repository <em>at</em> the workspace root and never searches above it,
            so a folder inside a repository reads this way too.
          </p>
        </div>
      );
    case 'failed':
      return (
        <>
          <ErrorEnvelope error={status.error} />
          <p className="git__note">
            <button type="button" onClick={onRetry}>
              Try again
            </button>
          </p>
        </>
      );
    case 'ready':
      return (
        <StatusList
          repository={status.value}
          appending={appending}
          selected={selected}
          onSelect={onSelect}
          onLoadMore={onLoadMore}
        />
      );
  }
}

function StatusList({
  repository,
  appending,
  selected,
  onSelect,
  onLoadMore,
}: {
  repository: RepositoryStatus;
  appending: boolean;
  selected: WorkspacePath | null;
  onSelect: (path: WorkspacePath | null) => void;
  onLoadMore: () => void;
}) {
  const groups = groupEntries(repository.entries);
  return (
    <>
      {groups.length === 0 && (
        <p className="git__empty">
          {repository.head === undefined
            ? 'No commits yet, and nothing to report in the working tree.'
            : 'No changes in the working tree.'}
        </p>
      )}
      {groups.map((group) => (
        <section className="git__group" key={group.id} aria-label={group.label}>
          <h3 className="git__group-title">
            {group.label}
            <span className="badge">{group.entries.length}</span>
          </h3>
          <ul className="git__files">
            {group.entries.map((entry) => (
              <ChangeRow
                key={`${group.id}:${entry.path}`}
                entry={entry}
                selected={entry.path === selected}
                onSelect={onSelect}
              />
            ))}
          </ul>
        </section>
      ))}
      {repository.truncated && (
        <p className="git__note">
          Showing {repository.entries.length.toLocaleString()} of{' '}
          {repository.total.toLocaleString()} changed files.
          <button type="button" onClick={onLoadMore} disabled={appending}>
            {appending ? 'Loading…' : 'Load more'}
          </button>
        </p>
      )}
      {repository.unrepresentable > 0 && (
        // Counted, never hidden: a status that quietly dropped a file would be
        // partial data shown as complete.
        <p className="git__note git__note--warn">
          {repository.unrepresentable.toLocaleString()} path
          {repository.unrepresentable === 1 ? '' : 's'} could not be shown: the name is not
          expressible as a workspace path.
        </p>
      )}
    </>
  );
}

function ChangeRow({
  entry,
  selected,
  onSelect,
}: {
  entry: FileStatus;
  selected: boolean;
  onSelect: (path: WorkspacePath | null) => void;
}) {
  const directory = fileDirectory(entry.path);
  return (
    <li className="git__file">
      <button
        type="button"
        className={`git__file-button${selected ? ' git__file-button--selected' : ''}`}
        aria-pressed={selected}
        title={describeEntry(entry)}
        onClick={() => onSelect(selected ? null : entry.path)}
      >
        <span
          className={`git__code git__code--${entry.conflicted ? 'conflicted' : entry.worktree.toLowerCase()}`}
          aria-hidden="true"
        >
          {statusCode(entry)}
        </span>
        <FileIcon kind={fileIconKind(fileName(entry.path))} />
        <span className="git__name">{fileName(entry.path)}</span>
        {directory !== null && <span className="git__dir">{directory}</span>}
        {entry.conflicted && <span className="badge git__badge--conflict">conflict</span>}
        {entry.kind === 'SUBMODULE' && <span className="badge">submodule</span>}
        {entry.renamedFrom !== undefined && <span className="badge">renamed</span>}
      </button>
    </li>
  );
}

function DiffRegion({
  diff,
  path,
  entry,
  onRetry,
  onCancel,
}: {
  diff: Read<DiffSummary>;
  path: WorkspacePath;
  entry: FileStatus | null;
  onRetry: () => void;
  onCancel: () => void;
}) {
  switch (diff.kind) {
    case 'idle':
    case 'loading':
      return (
        <p className="git__note" role="status">
          Reading the diff…
        </p>
      );
    case 'pending':
      return (
        <p className="git__note" role="status">
          Reading the diff (operation)…
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
        </p>
      );
    case 'cancelled':
      return (
        <p className="git__note" role="status">
          Diff cancelled.
          <button type="button" onClick={onRetry}>
            Read again
          </button>
        </p>
      );
    case 'notARepository':
      return <p className="git__note">This workspace is not a Git repository.</p>;
    case 'failed':
      return (
        <>
          <ErrorEnvelope error={diff.error} />
          <p className="git__note">
            <button type="button" onClick={onRetry}>
              Try again
            </button>
          </p>
        </>
      );
    case 'ready': {
      const file = diff.value.files.find((candidate) => candidate.path === path);
      if (file === undefined) {
        return (
          <div className="git__empty">
            <p>
              No difference against <code>{diff.value.target}</code>.
            </p>
            {entry?.worktree === 'UNTRACKED' && (
              // The honest reason, rather than an empty pane: `git.diff`
              // matches `git diff HEAD`, which does not describe a file the
              // repository has never seen.
              <p className="git__aside">
                An untracked file is not part of a diff against a revision.
              </p>
            )}
          </div>
        );
      }
      return <FileDiffView file={file} resolved={diff.value.resolved} target={diff.value.target} />;
    }
  }
}

function FileDiffView({
  file,
  resolved,
  target,
}: {
  file: FileDiff;
  resolved: string | undefined;
  target: string;
}) {
  return (
    <>
      <p className="git__diff-facts">
        <span className="git__badge-kind">{changeKindWord(file.change)}</span>
        {file.oldPath !== undefined && <span className="git__aside">from {file.oldPath}</span>}
        <span className="git__adds">+{file.additions.toLocaleString()}</span>
        <span className="git__dels">−{file.deletions.toLocaleString()}</span>
        <span className="git__aside">
          against {target}
          {resolved !== undefined ? ` (${shortId(resolved)})` : ''}
        </span>
      </p>
      {file.binary ? (
        // Binary content never crosses the wire, so there is nothing to draw
        // and the panel says so instead of showing an empty file.
        <p className="git__note">
          Binary file{file.size !== undefined ? `, ${formatBytes(file.size)}` : ''}. Its contents
          are not shown.
        </p>
      ) : (
        <div className="git__hunks">
          {file.hunks.map((hunk) => (
            <HunkView key={`${hunk.oldStart}:${hunk.newStart}:${hunk.lines.length}`} hunk={hunk} />
          ))}
          {file.hunks.length === 0 && <p className="git__note">No hunks in this response.</p>}
        </div>
      )}
      {file.hunksTruncated && (
        <p className="git__note git__note--warn">
          More hunks follow; this file's diff stops short of the whole change.
        </p>
      )}
    </>
  );
}

function HunkView({ hunk }: { hunk: DiffHunk }) {
  const lines = numberHunk(hunk);
  return (
    <div className="git__hunk">
      <div className="git__hunk-head">{hunkHeader(hunk)}</div>
      {lines.map((line, index) => (
        <div
          // Diff lines have no id and repeat freely; the index is their
          // identity within a hunk, and the hunk is replaced whole.
          // biome-ignore lint/suspicious/noArrayIndexKey: no stable id exists
          key={index}
          className={`git__line git__line--${line.kind.toLowerCase()}`}
        >
          <span className="git__gutter" aria-hidden="true">
            {line.oldNo ?? ''}
          </span>
          <span className="git__gutter" aria-hidden="true">
            {line.newNo ?? ''}
          </span>
          <span className="git__sign" aria-hidden="true">
            {lineSign(line.kind)}
          </span>
          <span className="git__text">{line.content}</span>
        </div>
      ))}
      {hunk.truncated && (
        <div className="git__note git__note--warn">
          This hunk was cut short; more lines follow in the file.
        </div>
      )}
    </div>
  );
}
