import type {
  DiffHunk,
  DiffSummary,
  FileDiff,
  FileStatus,
  RepositoryStatus,
  WorkspacePath,
} from '@gritty/protocol';
import { ConfirmDialog } from '../../components/confirm-dialog';
import { ErrorEnvelope } from '../../components/error-envelope';
import { FileIcon, fileIconKind } from '../../components/file-icon';
import {
  amendWarning,
  type BandVerb,
  bandVerbs,
  type ChangeGroup,
  changeKindWord,
  commitBlocker,
  type DiscardPlan,
  describeEntry,
  discardReport,
  discardWording,
  fileDirectory,
  fileName,
  formatBytes,
  gitRefusal,
  groupEntries,
  headLine,
  hunkHeader,
  lineSign,
  numberHunk,
  shortId,
  stateLabel,
  statusCode,
  upstreamLine,
  writeUnavailableReason,
} from './git-model';
import {
  type CommitDraft,
  type GitState,
  type Read,
  type Write,
  type WriteRequest,
  writeBusy,
} from './git-reducer';
import { HistoryView } from './history-view';

/**
 * The write half's callbacks (T102-U), in one prop so the read half's
 * signature did not have to grow by eleven.
 */
export interface WriteControls {
  /** The write on screen is a discard refused over dirty documents. */
  readonly canDiscardAndRetry: boolean;
  /** A discard waiting on its confirmation, or `null`. */
  readonly discarding: DiscardPlan | null;
  onStage(paths: readonly WorkspacePath[]): void;
  onUnstage(paths: readonly WorkspacePath[]): void;
  /** Ask first: this opens the confirmation, it does not discard. */
  onRequestDiscard(paths: readonly WorkspacePath[]): void;
  onConfirmDiscard(): void;
  onCancelDiscard(): void;
  onCommit(): void;
  onDraftChange(change: Partial<CommitDraft>): void;
  onDiscardAndRetry(): void;
  onRetryWrite(): void;
  onDismissWrite(): void;
  onCancelWrite(): void;
}

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
  writes,
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
  writes: WriteControls;
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
          writes={writes}
        />
      )}

      {writes.discarding !== null && (
        <DiscardDialog
          plan={writes.discarding}
          onConfirm={writes.onConfirmDiscard}
          onCancel={writes.onCancelDiscard}
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
  writes,
}: {
  state: GitState;
  selectedEntry: FileStatus | null;
  onSelect: (path: WorkspacePath | null) => void;
  onLoadMore: () => void;
  onRefresh: () => void;
  onReloadDiff: () => void;
  onCancelStatus: () => void;
  onCancelDiff: () => void;
  writes: WriteControls;
}) {
  const repository = state.status.kind === 'ready' ? state.status.value : null;
  // Why a verb cannot be used right now. Unavailable comes first because it is
  // the one the user can do something about; a write in flight is momentary.
  const unavailable = writeUnavailableReason(state.availability);
  const disabledReason =
    unavailable ?? (writeBusy(state.write) ? 'Waiting for the write in progress.' : null);
  return (
    <>
      {repository !== null && (
        <div className="git__writes">
          {unavailable !== null && (
            <p className="git__note git__note--warn" role="status">
              Staging and committing are unavailable: {unavailable}
            </p>
          )}
          <CommitBox
            repository={repository}
            draft={state.draft}
            write={state.write}
            unavailable={unavailable}
            writes={writes}
          />
          <WriteFeedback write={state.write} branch={headLine(repository)} writes={writes} />
        </div>
      )}

      <div className="git__list">
        <StatusRegion
          status={state.status}
          appending={state.appending}
          selected={state.selected}
          onSelect={onSelect}
          onLoadMore={onLoadMore}
          onRetry={onRefresh}
          onCancel={onCancelStatus}
          disabledReason={disabledReason}
          writes={writes}
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
  disabledReason,
  writes,
}: {
  status: Read<RepositoryStatus>;
  appending: boolean;
  selected: WorkspacePath | null;
  onSelect: (path: WorkspacePath | null) => void;
  onLoadMore: () => void;
  onRetry: () => void;
  onCancel: () => void;
  disabledReason: string | null;
  writes: WriteControls;
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
          disabledReason={disabledReason}
          writes={writes}
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
  disabledReason,
  writes,
}: {
  repository: RepositoryStatus;
  appending: boolean;
  selected: WorkspacePath | null;
  onSelect: (path: WorkspacePath | null) => void;
  onLoadMore: () => void;
  disabledReason: string | null;
  writes: WriteControls;
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
            <BandActions group={group} disabledReason={disabledReason} writes={writes} />
          </h3>
          <ul className="git__files">
            {group.entries.map((entry) => (
              <ChangeRow
                key={`${group.id}:${entry.path}`}
                entry={entry}
                verbs={bandVerbs(group.id)}
                selected={entry.path === selected}
                onSelect={onSelect}
                disabledReason={disabledReason}
                writes={writes}
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

const VERB_LABELS: Record<BandVerb, string> = {
  stage: 'Stage',
  unstage: 'Unstage',
  discard: 'Discard',
};

/** What a verb does, for its tooltip — the button itself has room for one word. */
const VERB_TITLES: Record<BandVerb, string> = {
  stage: 'Copy the working tree content into the index',
  unstage: 'Take it back out of the index; the working tree is untouched',
  discard: 'Throw the changes away, keeping a copy (asks first)',
};

function runVerb(verb: BandVerb, paths: readonly WorkspacePath[], writes: WriteControls) {
  switch (verb) {
    case 'stage':
      writes.onStage(paths);
      return;
    case 'unstage':
      writes.onUnstage(paths);
      return;
    case 'discard':
      writes.onRequestDiscard(paths);
      return;
  }
}

/**
 * The whole band at once. The paths are the band's own, named one by one:
 * there is no "all" on the wire (T102-C), so "Stage all" means these files and
 * not whatever else changed a moment ago.
 */
function BandActions({
  group,
  disabledReason,
  writes,
}: {
  group: ChangeGroup;
  disabledReason: string | null;
  writes: WriteControls;
}) {
  const verbs = bandVerbs(group.id);
  if (verbs.length === 0) return null;
  const paths = group.entries.map((entry) => entry.path);
  return (
    <span className="git__group-actions">
      {verbs.map((verb) => (
        <button
          key={verb}
          type="button"
          className="git__action"
          disabled={disabledReason !== null}
          title={disabledReason ?? `${VERB_TITLES[verb]} — all ${paths.length} in ${group.label}`}
          aria-label={`${VERB_LABELS[verb]} all ${paths.length} in ${group.label}`}
          onClick={() => runVerb(verb, paths, writes)}
        >
          {VERB_LABELS[verb]} all
        </button>
      ))}
    </span>
  );
}

function ChangeRow({
  entry,
  verbs,
  selected,
  onSelect,
  disabledReason,
  writes,
}: {
  entry: FileStatus;
  verbs: readonly BandVerb[];
  selected: boolean;
  onSelect: (path: WorkspacePath | null) => void;
  disabledReason: string | null;
  writes: WriteControls;
}) {
  const directory = fileDirectory(entry.path);
  return (
    <li className={`git__file${selected ? ' git__file--selected' : ''}`}>
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
      {verbs.length > 0 && (
        // Siblings of the row's button, not children: a button inside a button
        // is invalid, and each verb has to be reachable on its own by Tab.
        <span className="git__row-actions">
          {verbs.map((verb) => (
            <button
              key={verb}
              type="button"
              className="git__action"
              disabled={disabledReason !== null}
              title={disabledReason ?? VERB_TITLES[verb]}
              aria-label={`${VERB_LABELS[verb]} ${entry.path}`}
              onClick={() => runVerb(verb, [entry.path], writes)}
            >
              {VERB_LABELS[verb]}
            </button>
          ))}
        </span>
      )}
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

/**
 * The commit box (T102-U requirements 2, 3 and 6): the message, the two
 * options that are visible choices and never defaults, and the branch the
 * commit lands on — in view, because a commit to the wrong branch is the
 * mistake this box is most likely to be used to make.
 */
function CommitBox({
  repository,
  draft,
  write,
  unavailable,
  writes,
}: {
  repository: RepositoryStatus;
  draft: CommitDraft;
  write: Write;
  unavailable: string | null;
  writes: WriteControls;
}) {
  const committing = writeBusy(write) && isCommit(write);
  const blocker =
    unavailable ??
    (writeBusy(write) ? 'Waiting for the write in progress.' : null) ??
    commitBlocker(repository, draft);
  const submit = () => {
    if (blocker === null) writes.onCommit();
  };
  return (
    <form
      className="git__commit-box"
      aria-label="Commit"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <p className="git__commit-target">
        Commit to <span className="git__head">{headLine(repository)}</span>
      </p>
      <textarea
        className="git__message"
        aria-label="Commit message"
        placeholder="Message (⌘↩ to commit)"
        rows={3}
        value={draft.message}
        // Read-only rather than disabled while a commit runs: the text stays
        // selectable, and what was typed cannot drift from what is being
        // committed and then be cleared by its success.
        readOnly={committing}
        onChange={(event) => writes.onDraftChange({ message: event.target.value })}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            submit();
          }
        }}
      />
      <label className="git__option">
        <input
          type="checkbox"
          checked={draft.amend}
          disabled={committing || repository.head === undefined}
          onChange={(event) => writes.onDraftChange({ amend: event.target.checked })}
        />
        Amend the last commit
      </label>
      {draft.amend && <p className="git__aside git__aside--warn">{amendWarning(repository)}</p>}
      <label className="git__option">
        <input
          type="checkbox"
          checked={draft.noVerify}
          disabled={committing}
          onChange={(event) => writes.onDraftChange({ noVerify: event.target.checked })}
        />
        Skip hooks (<code>--no-verify</code>)
      </label>
      <p className={`git__aside${draft.noVerify ? ' git__aside--warn' : ''}`}>
        {draft.noVerify
          ? "This repository's pre-commit and commit-msg hooks will not run: the project's own checks are skipped."
          : "Commits run this repository's hooks, which are the project's own code."}
      </p>
      <div className="git__commit-actions">
        <button type="submit" disabled={blocker !== null} title={blocker ?? undefined}>
          {draft.amend ? 'Amend commit' : 'Commit'}
        </button>
        {blocker !== null && !committing && <span className="git__aside">{blocker}</span>}
      </div>
      {committing && (
        <p className="git__note" role="status">
          {write.kind === 'pending'
            ? draft.noVerify
              ? 'Committing (operation)…'
              : 'Committing: the hooks are running…'
            : 'Committing…'}
          {write.kind === 'pending' && (
            <button type="button" onClick={writes.onCancelWrite}>
              Stop
            </button>
          )}
        </p>
      )}
    </form>
  );
}

function isCommit(write: Write): boolean {
  return write.kind !== 'idle' && write.request.verb === 'commit';
}

const RUNNING_WORDS: Record<WriteRequest['verb'], string> = {
  stage: 'Staging',
  unstage: 'Unstaging',
  discard: 'Discarding',
  commit: 'Committing',
};

/**
 * What the last write said, until it is dismissed or the next one replaces it.
 *
 * Every refusal is an `ErrorEnvelope` (T102-U failure behaviour), and a refusal
 * from `git` itself also shows `git`'s own output as text — the envelope's
 * `details` is JSON, where a hook's multi-line report is unreadable.
 */
function WriteFeedback({
  write,
  branch,
  writes,
}: {
  write: Write;
  branch: string;
  writes: WriteControls;
}) {
  switch (write.kind) {
    case 'idle':
      return null;
    case 'running':
    case 'pending':
      // The commit box speaks for a commit; the other three are fast, and
      // saying so keeps a slow index from looking like a dead button.
      if (write.request.verb === 'commit') return null;
      return (
        <p className="git__note" role="status">
          {RUNNING_WORDS[write.request.verb]} {countPaths(write.request.paths)}…
        </p>
      );
    case 'failed': {
      const refusal = gitRefusal(write.error);
      return (
        <div className="git__feedback">
          <ErrorEnvelope error={write.error} onDismiss={writes.onDismissWrite} />
          {refusal !== null && (
            <div className="git__refusal">
              <p className="git__aside">
                git {refusal.subcommand} refused
                {refusal.exitCode !== null ? ` (exit ${refusal.exitCode})` : ''}. Its output:
              </p>
              <pre className="git__refusal-output">{refusal.output || '(no output)'}</pre>
            </div>
          )}
          {write.request.verb === 'commit' && (
            <p className="git__aside">The message is still in the box.</p>
          )}
          {writes.canDiscardAndRetry && (
            <p className="git__note" role="status">
              Documents under the selection have unsaved changes. Save or close them first, or
              <button type="button" onClick={writes.onDiscardAndRetry}>
                discard them and continue
              </button>
            </p>
          )}
          {write.error.retryable && (
            <p className="git__note">
              <button type="button" onClick={writes.onRetryWrite}>
                Try again
              </button>
            </p>
          )}
        </div>
      );
    }
    case 'cancelled':
      return (
        <p className="git__note" role="status">
          Commit stopped. The repository is read again, because a stopped commit may or may not have
          been made.
          <button type="button" onClick={writes.onDismissWrite}>
            Dismiss
          </button>
        </p>
      );
    case 'done': {
      if (write.request.verb === 'discard') {
        return (
          <div className="git__note git__note--column" role="status">
            {discardReport(write.result).map((line) => (
              <span key={line}>{withCode(line)}</span>
            ))}
            <button type="button" onClick={writes.onDismissWrite}>
              Dismiss
            </button>
          </div>
        );
      }
      if (write.request.verb === 'commit') {
        return (
          <p className="git__note" role="status">
            {write.result.commit !== undefined
              ? `Committed ${shortId(write.result.commit)} to ${branch}.`
              : `Committed to ${branch}.`}
            <button type="button" onClick={writes.onDismissWrite}>
              Dismiss
            </button>
          </p>
        );
      }
      // A stage or an unstage says what it did by moving the row.
      return null;
    }
  }
}

function countPaths(paths: readonly WorkspacePath[]): string {
  return paths.length === 1 ? fileName(paths[0] ?? '') : `${paths.length.toLocaleString()} files`;
}

/**
 * Backticked spans as `<code>`. The wording lives in `git-model.ts` as plain
 * strings so it can be asserted there; this is the only markup it gets, and
 * every piece stays text — a path in the wording is repository content.
 */
function withCode(line: string) {
  return line.split('`').map((part, index) =>
    // biome-ignore lint/suspicious/noArrayIndexKey: segments of one fixed string
    index % 2 === 1 ? <code key={index}>{part}</code> : <span key={index}>{part}</span>,
  );
}

/** How many paths a discard confirmation lists before it counts the rest. */
const DISCARD_LISTED = 8;

function DiscardDialog({
  plan,
  onConfirm,
  onCancel,
}: {
  plan: DiscardPlan;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const wording = discardWording(plan);
  const listed = plan.paths.slice(0, DISCARD_LISTED);
  return (
    <ConfirmDialog
      title={wording.title}
      confirmLabel={wording.confirm}
      onConfirm={onConfirm}
      onCancel={onCancel}
    >
      {wording.lines.map((line) => (
        <p key={line}>{withCode(line)}</p>
      ))}
      {plan.paths.length > 1 && (
        <ul className="git__discard-paths">
          {listed.map((path) => (
            <li key={path}>
              <code>{path}</code>
            </li>
          ))}
          {plan.paths.length > listed.length && (
            <li>and {(plan.paths.length - listed.length).toLocaleString()} more</li>
          )}
        </ul>
      )}
    </ConfirmDialog>
  );
}
