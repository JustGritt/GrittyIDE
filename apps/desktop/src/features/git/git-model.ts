import type {
  CommitSummary,
  CoreErrorShape,
  DiffChangeKind,
  DiffHunk,
  DiffLineKind,
  FileState,
  FileStatus,
  GitWriteAvailability,
  GitWriteResult,
  RepositoryState,
  RepositoryStatus,
  UpstreamStatus,
  WorkspacePath,
} from '@gritty/protocol';

/**
 * Presentation for the Changes tab (T100-U). Pure, so every rule below is
 * unit-tested without React, a DOM or a repository.
 *
 * Nothing here reconstructs a textual diff. `git.diff` returns structured
 * hunks (hard rule 5), and the closest this file comes to the old shape is
 * `hunkHeader`, which *composes* `@@ -a,b +c,d @@` out of four numbers for a
 * reader who knows that notation — the numbers are the source, never a string
 * the core sent.
 */

/** Which band of the list an entry belongs in, in the order they are shown. */
export type ChangeGroupId = 'conflicted' | 'staged' | 'unstaged' | 'untracked';

export interface ChangeGroup {
  readonly id: ChangeGroupId;
  readonly label: string;
  readonly entries: readonly FileStatus[];
}

const GROUP_LABELS: Record<ChangeGroupId, string> = {
  conflicted: 'Conflicts',
  staged: 'Staged',
  unstaged: 'Changes',
  untracked: 'Untracked',
};

/**
 * Split a status page into the bands a reader reads it in.
 *
 * A partially staged file appears in **both** `staged` and `unstaged`, because
 * that is what it is: one half of it is in the index and the other is not.
 * Picking one band would be a claim the repository does not make. A conflicted
 * file is only ever in `Conflicts`, because its index side is not a stage the
 * user chose.
 *
 * Empty bands are dropped rather than rendered as headings with nothing under
 * them.
 */
export function groupEntries(entries: readonly FileStatus[]): readonly ChangeGroup[] {
  const bands: Record<ChangeGroupId, FileStatus[]> = {
    conflicted: [],
    staged: [],
    unstaged: [],
    untracked: [],
  };
  for (const entry of entries) {
    if (entry.conflicted) {
      bands.conflicted.push(entry);
      continue;
    }
    if (entry.worktree === 'UNTRACKED' || entry.index === 'UNTRACKED') {
      bands.untracked.push(entry);
      continue;
    }
    if (entry.index !== 'UNMODIFIED' && entry.index !== 'IGNORED') bands.staged.push(entry);
    if (entry.worktree !== 'UNMODIFIED' && entry.worktree !== 'IGNORED') {
      bands.unstaged.push(entry);
    }
    // An entry that is `IGNORED` on both sides is only here because the caller
    // asked for ignored files; it belongs under Untracked rather than nowhere.
    if (entry.index === 'IGNORED' || entry.worktree === 'IGNORED') bands.untracked.push(entry);
  }
  const order: readonly ChangeGroupId[] = ['conflicted', 'staged', 'unstaged', 'untracked'];
  return order
    .filter((id) => bands[id].length > 0)
    .map((id) => ({ id, label: GROUP_LABELS[id], entries: bands[id] }));
}

const STATE_LETTERS: Record<FileState, string> = {
  UNMODIFIED: '·',
  ADDED: 'A',
  MODIFIED: 'M',
  DELETED: 'D',
  RENAMED: 'R',
  TYPE_CHANGED: 'T',
  UNTRACKED: '?',
  IGNORED: '!',
};

const STATE_WORDS: Record<FileState, string> = {
  UNMODIFIED: 'unmodified',
  ADDED: 'added',
  MODIFIED: 'modified',
  DELETED: 'deleted',
  RENAMED: 'renamed',
  TYPE_CHANGED: 'type changed',
  UNTRACKED: 'untracked',
  IGNORED: 'ignored',
};

/**
 * Widened for the lookups below: the protocol is additive, so a newer core can
 * send a value this build's union does not contain. `run-status.ts` makes the
 * same allowance for the same reason, and every accessor here follows its rule
 * — show what arrived, never fold it into a value this build would be
 * guessing at.
 */
type Lookup = Readonly<Record<string, string | undefined>>;

/** `git status --short`'s letter, so the two-column mark is one a user knows. */
export function stateLetter(state: FileState): string {
  return (STATE_LETTERS as Lookup)[state] ?? '?';
}

export function stateWord(state: FileState): string {
  return (STATE_WORDS as Lookup)[state] ?? state.toLowerCase();
}

/** The two-column code: index side, then worktree side. */
export function statusCode(entry: FileStatus): string {
  return `${stateLetter(entry.index)}${stateLetter(entry.worktree)}`;
}

/**
 * The whole of an entry in words, for the row's tooltip.
 *
 * Colour is never the only channel (T231 §10) and neither is a letter: the
 * code above is two glyphs, and this says the same thing in a sentence.
 */
export function describeEntry(entry: FileStatus): string {
  const parts = [`staged: ${stateWord(entry.index)}`, `worktree: ${stateWord(entry.worktree)}`];
  if (entry.conflicted) parts.push('conflicted');
  if (entry.renamedFrom !== undefined) parts.push(`renamed from ${entry.renamedFrom}`);
  if (entry.kind === 'SUBMODULE') parts.push('submodule (reported, not entered)');
  if (entry.kind === 'SYMLINK') parts.push('symlink');
  return parts.join(' · ');
}

/** The trailing segment, so a long path still shows the part that identifies it. */
export function fileName(path: string): string {
  const cut = path.lastIndexOf('/');
  return cut === -1 ? path : path.slice(cut + 1);
}

/** Everything before the trailing segment, `null` at the repository root. */
export function fileDirectory(path: string): string | null {
  const cut = path.lastIndexOf('/');
  return cut === -1 ? null : path.slice(0, cut);
}

const STATE_LABELS: Record<RepositoryState, string> = {
  CLEAN: 'clean',
  MERGING: 'merge in progress',
  REBASING: 'rebase in progress',
  CHERRY_PICKING: 'cherry-pick in progress',
  REVERTING: 'revert in progress',
  BISECTING: 'bisect in progress',
  APPLYING_MAILBOX: 'applying a mailbox',
  BARE: 'bare repository',
};

/**
 * What the repository is in the middle of, or `null` when it is in the middle
 * of nothing. `CLEAN` is deliberately not a badge: a repository that is not
 * mid-anything should say nothing rather than reassure.
 *
 * A state this build does not recognise is shown as it arrived rather than
 * folded into one it does — the rule `run-status.ts` set for a newer core.
 */
export function stateLabel(state: RepositoryState): string | null {
  if (state === 'CLEAN') return null;
  return (STATE_LABELS as Lookup)[state] ?? `unrecognised state (${state})`;
}

/** Short object ids, the length `git log --oneline` uses. */
export function shortId(id: string): string {
  return id.slice(0, 7);
}

/**
 * Which commit the working tree is measured against, in one line.
 *
 * The three cases are genuinely different and none of them is an error: a
 * branch, a detached HEAD, and a repository with no commits at all.
 */
export function headLine(repository: RepositoryStatus): string {
  if (repository.branch !== undefined) return repository.branch;
  if (repository.head === undefined) return 'no commits yet';
  // `detached` is read rather than inferred from the missing branch: the two
  // are separate fields on the wire and a build that guessed would eventually
  // guess wrong.
  return repository.detached
    ? `detached at ${shortId(repository.head)}`
    : `at ${shortId(repository.head)}`;
}

/**
 * Divergence from the tracking ref, counted against the **local** copy of it.
 * Nothing is fetched to produce this, so it is as stale as the user's last
 * fetch; absent entirely when there is no upstream, which is not `0/0`.
 */
export function upstreamLine(upstream: UpstreamStatus | undefined): string | null {
  if (upstream === undefined) return null;
  if (upstream.ahead === 0 && upstream.behind === 0) return 'up to date';
  const parts: string[] = [];
  if (upstream.ahead > 0) parts.push(`↑${upstream.ahead}`);
  if (upstream.behind > 0) parts.push(`↓${upstream.behind}`);
  return parts.join(' ');
}

const CHANGE_WORDS: Record<DiffChangeKind, string> = {
  ADDED: 'added',
  DELETED: 'deleted',
  MODIFIED: 'modified',
  RENAMED: 'renamed',
  COPIED: 'copied',
  TYPE_CHANGED: 'type changed',
};

export function changeKindWord(kind: DiffChangeKind): string {
  return (CHANGE_WORDS as Lookup)[kind] ?? kind.toLowerCase();
}

export interface NumberedLine {
  readonly kind: DiffLineKind;
  readonly content: string;
  /** Line number on the old side; `null` for an added line. */
  readonly oldNo: number | null;
  /** Line number on the new side; `null` for a removed line. */
  readonly newNo: number | null;
}

/**
 * Walk a hunk's lines and give each the number it has on the side it exists
 * on. The core sends the hunk's two start lines and each line's side; the
 * running count is the only thing this adds, and it is here rather than in the
 * component so it can be asserted directly.
 */
export function numberHunk(hunk: DiffHunk): readonly NumberedLine[] {
  let oldNo = hunk.oldStart;
  let newNo = hunk.newStart;
  return hunk.lines.map((line) => {
    switch (line.kind) {
      case 'ADDED':
        return { kind: line.kind, content: line.content, oldNo: null, newNo: newNo++ };
      case 'REMOVED':
        return { kind: line.kind, content: line.content, oldNo: oldNo++, newNo: null };
      default:
        return { kind: line.kind, content: line.content, oldNo: oldNo++, newNo: newNo++ };
    }
  });
}

/** `@@ -a,b +c,d @@`, composed from the four numbers rather than parsed out of one. */
export function hunkHeader(hunk: DiffHunk): string {
  return `@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@`;
}

/** The sign column. A word is on the row too; this is the second channel. */
export function lineSign(kind: DiffLineKind): string {
  switch (kind) {
    case 'ADDED':
      return '+';
    case 'REMOVED':
      return '−';
    default:
      return ' ';
  }
}

export function formatBytes(bytes: number): string {
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

/**
 * When a commit was made, in the reader's own timezone.
 *
 * The core keeps the two numbers Git stores — epoch seconds, and the offset the
 * author's machine was at — precisely so this decision is made here. Two rules,
 * both about not lying:
 *
 * - Recent commits get a relative label ("2 hours ago"), which is what a reader
 *   scanning a history actually wants.
 * - Anything older than a week gets the date, because "37 days ago" is
 *   arithmetic the reader has to undo.
 *
 * `now` is a parameter so this is testable without freezing the clock.
 */
export function commitAge(seconds: number, now: number = Date.now()): string {
  const elapsed = Math.floor(now / 1000) - seconds;
  if (elapsed < 60) return 'just now';
  const minutes = Math.floor(elapsed / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? '' : 's'} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? '' : 's'} ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days} day${days === 1 ? '' : 's'} ago`;
  return new Date(seconds * 1000).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

/**
 * The exact timestamp, for the row's `title`.
 *
 * The relative label above is the readable one; this is the one that answers
 * "when exactly", and a reader should never have to leave the panel for it.
 */
export function commitTimestamp(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString();
}

/**
 * How many parents make a commit a merge.
 *
 * Derived from `parents.length` rather than carried as a flag, so there is no
 * second field that could disagree with the list.
 */
export function isMerge(commit: CommitSummary): boolean {
  return commit.parents.length > 1;
}

/**
 * The line under a commit's subject: who, and when.
 *
 * The *author* rather than the committer, which is what `git log` shows and
 * what a reader means by "who wrote this". The two differ after a rebase or a
 * cherry-pick, and showing the committer there would credit the wrong person.
 */
export function commitByline(commit: CommitSummary, now: number = Date.now()): string {
  return `${commit.author.name} · ${commitAge(commit.author.time, now)}`;
}

// ---------------------------------------------------------------------------
// The write half (T102-U): which verbs a band offers, what a discard will do in
// words, and why a button is disabled when it is.
// ---------------------------------------------------------------------------

/** A verb a row or a band header can offer. */
export type BandVerb = 'stage' | 'unstage' | 'discard';

/**
 * The verbs each band offers, in the order they are drawn.
 *
 * - **Staged** unstages and does nothing else. Discarding from here would stash
 *   the worktree half too, which is not what a reader of the Staged band is
 *   looking at.
 * - **Changes** and **Untracked** stage or discard.
 * - **Conflicts** offer nothing. Staging a conflicted file is how Git records
 *   it as resolved, and doing that from a button — with the markers possibly
 *   still in the file — is a decision this ticket does not make on the user's
 *   behalf.
 */
export function bandVerbs(band: ChangeGroupId): readonly BandVerb[] {
  switch (band) {
    case 'staged':
      return ['unstage'];
    case 'unstaged':
    case 'untracked':
      return ['stage', 'discard'];
    case 'conflicted':
      return [];
  }
}

/**
 * A discard, split the way the core will split it (`git/write.rs`
 * `split_by_tracking`): untracked paths go to the Trash, everything else is
 * stashed.
 */
export interface DiscardPlan {
  readonly paths: readonly WorkspacePath[];
  readonly stashed: readonly WorkspacePath[];
  readonly trashed: readonly WorkspacePath[];
  /**
   * Stashed paths that also have staged changes. `git stash push -- <path>`
   * saves and reverts the index *and* the worktree for a path, so the staged
   * half goes into the stash too — a thing to say before, not discover after.
   */
  readonly alsoStaged: readonly WorkspacePath[];
}

export function discardPlan(
  paths: readonly WorkspacePath[],
  entries: readonly FileStatus[],
): DiscardPlan {
  const stashed: WorkspacePath[] = [];
  const trashed: WorkspacePath[] = [];
  const alsoStaged: WorkspacePath[] = [];
  for (const path of paths) {
    const entry = entries.find((candidate) => candidate.path === path);
    if (entry !== undefined && (entry.worktree === 'UNTRACKED' || entry.index === 'UNTRACKED')) {
      trashed.push(path);
      continue;
    }
    stashed.push(path);
    if (entry !== undefined && entry.index !== 'UNMODIFIED' && entry.index !== 'IGNORED') {
      alsoStaged.push(path);
    }
  }
  return { paths, stashed, trashed, alsoStaged };
}

/** What the discard confirmation says. */
export interface DiscardWording {
  readonly title: string;
  readonly lines: readonly string[];
  readonly confirm: string;
}

function countedFiles(paths: readonly WorkspacePath[]): string {
  if (paths.length === 1) return fileName(paths[0] ?? '');
  return `${paths.length.toLocaleString()} files`;
}

/**
 * The confirmation's wording. The two dispositions are worded differently
 * because they are different (T102-U requirement 4): a stash is Git's, named
 * and restorable with Git; the Trash is the operating system's, and restoring
 * from it happens outside Gritty.
 */
export function discardWording(plan: DiscardPlan): DiscardWording {
  const lines: string[] = [];
  if (plan.stashed.length > 0) {
    lines.push(
      `The changes to ${countedFiles(plan.stashed)} are stashed before the ` +
        `${plan.stashed.length === 1 ? 'file is' : 'files are'} restored, so nothing is deleted: ` +
        'the stash is listed by `git stash list`, and `git stash apply` brings the changes back.',
    );
  }
  if (plan.alsoStaged.length > 0) {
    lines.push(
      `${countedFiles(plan.alsoStaged)} ${plan.alsoStaged.length === 1 ? 'also has' : 'also have'} ` +
        'staged changes. Those are stashed and unstaged too.',
    );
  }
  if (plan.trashed.length > 0) {
    lines.push(
      `${countedFiles(plan.trashed)} ${plan.trashed.length === 1 ? 'is' : 'are'} untracked, ` +
        'so Git holds no copy. ' +
        `${plan.trashed.length === 1 ? 'It goes' : 'They go'} to the Trash, and can be restored ` +
        'from there.',
    );
  }
  const onlyTrash = plan.stashed.length === 0;
  const what = countedFiles(plan.paths);
  return onlyTrash
    ? { title: `Move ${what} to the Trash?`, lines, confirm: 'Move to Trash' }
    : { title: `Discard changes to ${what}?`, lines, confirm: 'Discard changes' };
}

/**
 * What a finished discard did, for the notice after it: where the work went
 * and how to get it back, named by commit id because `stash@{0}` stops meaning
 * this stash as soon as anything else is stashed.
 */
export function discardReport(result: GitWriteResult): readonly string[] {
  const lines: string[] = [];
  if (result.stash !== undefined) {
    lines.push(
      `Stashed as ${result.stash.name} (${shortId(result.stash.commit)}). ` +
        `\`git stash apply ${shortId(result.stash.commit)}\` restores it.`,
    );
  }
  if (result.trashed.length > 0) {
    lines.push(`Moved to the Trash: ${result.trashed.map(fileName).join(', ')}.`);
  }
  if (lines.length === 0) {
    // The core stashes only when the stash ref moved; `git stash push` with
    // nothing to save exits 0 and creates nothing.
    lines.push('Nothing needed saving: there were no changes left to discard.');
  }
  return lines;
}

/**
 * Why git cannot write here, or `null` when it can. Unknown is a reason too:
 * the verdict rides on `git.status`, and its absence is never read as "yes".
 */
export function writeUnavailableReason(availability: GitWriteAvailability | null): string | null {
  if (availability === null) {
    return 'Whether git can write here is not known yet. Refresh to check.';
  }
  if (availability.available) return null;
  return availability.reason ?? 'git is not available.';
}

/** How many entries are in the Staged band of the page shown. */
export function stagedCount(entries: readonly FileStatus[]): number {
  return groupEntries(entries).find((group) => group.id === 'staged')?.entries.length ?? 0;
}

/**
 * Why the commit button is disabled, or `null` when it is not.
 *
 * "Nothing staged" is only claimed when the page is the whole list: a
 * truncated status may have staged files past the page, and a button disabled
 * on a guess is the enabled-and-inert state's mirror image. Git itself refuses
 * an empty commit, and that refusal is shown.
 */
export function commitBlocker(
  repository: RepositoryStatus,
  draft: { readonly message: string; readonly amend: boolean },
): string | null {
  if (draft.message.trim() === '') return 'Write a commit message first.';
  if (draft.amend && repository.head === undefined) return 'There is no commit to amend yet.';
  if (!draft.amend && !repository.truncated && stagedCount(repository.entries) === 0) {
    return 'Nothing is staged.';
  }
  return null;
}

/**
 * The danger of an amend, stated rather than prevented (T102-U non-goals).
 *
 * The upstream counts are against the local copy of the tracking ref, so this
 * is only as fresh as the last fetch, and it says so.
 */
export function amendWarning(repository: RepositoryStatus): string {
  const upstream = repository.upstream;
  if (upstream === undefined) {
    return 'Replaces the last commit. This branch has no upstream, so nothing pushed is known to contain it.';
  }
  const remote = upstream.refName.replace(/^refs\/remotes\//, '');
  if (upstream.ahead === 0) {
    return `Replaces the last commit, which ${remote} already has (as of the last fetch). Amending it makes the branch diverge from ${remote}.`;
  }
  return `Replaces the last commit. ${remote} does not have it yet (as of the last fetch).`;
}

/** A refusal from `git` itself — a hook, or git's own "nothing to commit" — with its output. */
export interface GitRefusal {
  readonly subcommand: string;
  readonly exitCode: number | null;
  readonly output: string;
}

/**
 * The output of a refused write, when the error carries one.
 *
 * Read out of `details` defensively: the envelope is JSON, and a newer core
 * could shape it differently. The output is untrusted repository-authored text
 * and is rendered as text, never as markup.
 */
export function gitRefusal(error: CoreErrorShape): GitRefusal | null {
  if (error.code !== 'Conflict') return null;
  const details = error.details;
  if (typeof details !== 'object' || details === null || Array.isArray(details)) return null;
  const { subcommand, exitCode, stderr } = details as Record<string, unknown>;
  if (typeof stderr !== 'string' || typeof subcommand !== 'string') return null;
  return {
    subcommand,
    exitCode: typeof exitCode === 'number' ? exitCode : null,
    output: stderr,
  };
}
