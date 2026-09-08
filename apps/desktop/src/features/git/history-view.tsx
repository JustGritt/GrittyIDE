import type { CommitPage, CommitSummary } from '@gritty/protocol';
import { ErrorEnvelope } from '../../components/error-envelope';
import { commitByline, commitTimestamp, isMerge } from './git-model';
import type { Read } from './git-reducer';

/**
 * The **History** half of the git panel (T101-U): `git.log` as a list of
 * commits, newest first.
 *
 * Split from `changes-view.tsx` for the reason that file's own header gives —
 * a view with no caller is the defect this repository keeps catching late — so
 * the render tests next door mount every arm of the read without a transport or
 * a repository.
 *
 * ## Why there is no count, and no graph
 *
 * `CommitPage` carries no `total`, deliberately: counting every commit means
 * walking a whole history to render a number beside twenty rows, and the core
 * refuses to report a count it did not compute. So the footer says "Load more"
 * or nothing, and never "20 of 4,812".
 *
 * There is no branch graph either. A graph is a layout algorithm over the
 * parent edges, and drawing one in a ~300px column is where that effort would
 * be worst spent. `parents` is on every row, so a graph is additive later.
 */
export function HistoryView({
  state,
  appending,
  onLoadMore,
  onRetry,
  onCancel,
}: {
  state: Read<CommitPage>;
  appending: boolean;
  onLoadMore: () => void;
  onRetry: () => void;
  onCancel: () => void;
}) {
  switch (state.kind) {
    case 'idle':
    case 'loading':
      return (
        <p className="git__note" role="status">
          Reading the history…
        </p>
      );

    case 'pending':
      // The read outlived its budget and became an operation. Saying so, and
      // offering to stop it, is the whole reason the arm exists.
      return (
        <p className="git__note" role="status">
          Reading the history (operation)…{' '}
          <button type="button" onClick={onCancel}>
            Stop
          </button>
        </p>
      );

    case 'notARepository':
      return (
        <div className="git__empty">
          <p>This folder is not a Git repository.</p>
        </div>
      );

    case 'cancelled':
      return (
        <p className="git__note git__note--warn" role="status">
          The read was cancelled.{' '}
          <button type="button" onClick={onRetry}>
            Read again
          </button>
        </p>
      );

    case 'failed':
      return (
        <div className="git__note git__note--warn">
          <ErrorEnvelope error={state.error} />
          <button type="button" onClick={onRetry}>
            Try again
          </button>
        </div>
      );

    case 'ready': {
      const page = state.value;
      if (page.commits.length === 0) {
        return (
          <div className="git__empty">
            {/* An unborn branch is not an error: the repository exists and has
                no commits yet, which is a different sentence from a failure. */}
            <p>No commits yet.</p>
          </div>
        );
      }
      return (
        <>
          <ol className="git__commits">
            {page.commits.map((commit) => (
              <CommitRow key={commit.id} commit={commit} />
            ))}
          </ol>
          {page.truncated && (
            <p className="git__note">
              <button type="button" onClick={onLoadMore} disabled={appending}>
                {appending ? 'Loading…' : 'Load more'}
              </button>
            </p>
          )}
        </>
      );
    }
  }
}

function CommitRow({ commit }: { commit: CommitSummary }) {
  return (
    <li className="git__commit">
      <span className="git__commit-subject" title={commit.subject}>
        {commit.subject}
      </span>
      <span className="git__commit-meta">
        {/* The abbreviation the *repository* considers unambiguous, computed by
            the core — not a fixed prefix length, which grows ambiguous as a
            repository does. */}
        <code className="git__commit-id" title={commit.id}>
          {commit.shortId}
        </code>
        {isMerge(commit) && (
          <span className="badge git__commit-merge" title={`${commit.parents.length} parents`}>
            merge
          </span>
        )}
        <span className="git__commit-byline" title={commitTimestamp(commit.author.time)}>
          {commitByline(commit)}
        </span>
      </span>
    </li>
  );
}
