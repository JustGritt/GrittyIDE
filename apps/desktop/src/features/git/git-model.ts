import type {
  DiffChangeKind,
  DiffHunk,
  DiffLineKind,
  FileState,
  FileStatus,
  RepositoryState,
  RepositoryStatus,
  UpstreamStatus,
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
