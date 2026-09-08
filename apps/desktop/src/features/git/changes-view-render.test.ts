import type {
  CoreErrorShape,
  DiffSummary,
  FileDiff,
  FileStatus,
  OperationId,
  RepositoryStatus,
  WorkspacePath,
} from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { ChangesView, type GitView } from './changes-view';
import type { GitState } from './git-reducer';
import { initialGitState } from './git-reducer';

/**
 * What the Changes tab actually puts on the screen (T100-U).
 *
 * Static markup rather than a DOM, matching `agents-section-render.test.ts`:
 * `apps/desktop`'s vitest project is a node one, and a string is a stronger
 * assertion than a query against a tree. The point of rendering at all is that
 * a view model with no caller is the defect this repository keeps catching
 * late — `git-reducer.test.ts` proves the state machine, and this proves the
 * component that is supposed to be reading it.
 *
 * Every arm of both reads is here, including the three the happy path never
 * reaches: `pending`, `notARepository`, and a refusal from the core.
 */

function tagsOf(markup: string): readonly string[] {
  // Attribute values are blanked first: a repository's own content lands in a
  // `title`, and an escaped `<script` inside one is the safe outcome rather
  // than a finding.
  return markup.replace(/"[^"]*"/g, '""').match(/<[^>]*>/g) ?? [];
}

function entry(path: string, overrides: Partial<FileStatus> = {}): FileStatus {
  return {
    path: path as WorkspacePath,
    index: 'UNMODIFIED',
    worktree: 'MODIFIED',
    kind: 'FILE',
    conflicted: false,
    ...overrides,
  };
}

function repository(overrides: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    branch: 'main',
    detached: false,
    head: 'abcdef1234567890',
    state: 'CLEAN',
    entries: [],
    total: 0,
    truncated: false,
    unrepresentable: 0,
    ...overrides,
  };
}

function file(overrides: Partial<FileDiff> = {}): FileDiff {
  return {
    path: 'src/a.ts' as WorkspacePath,
    change: 'MODIFIED',
    additions: 2,
    deletions: 1,
    binary: false,
    hunksTruncated: false,
    hunks: [
      {
        oldStart: 10,
        oldLines: 2,
        newStart: 10,
        newLines: 3,
        truncated: false,
        lines: [
          { kind: 'CONTEXT', content: 'const a = 1;' },
          { kind: 'REMOVED', content: 'const b = 2;' },
          { kind: 'ADDED', content: 'const b = 3;' },
          { kind: 'ADDED', content: 'const c = 4;' },
        ],
      },
    ],
    ...overrides,
  };
}

function summary(overrides: Partial<DiffSummary> = {}): DiffSummary {
  return {
    target: 'HEAD',
    resolved: 'abcdef1234567890',
    files: [file()],
    total: 1,
    truncated: false,
    unrepresentable: 0,
    ...overrides,
  };
}

const REFUSAL: CoreErrorShape = {
  code: 'Io',
  message: 'object 4f2a is unreadable',
  retryable: false,
};

/** Every callback throws: rendering must be free of side effects. */
const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

function render(
  state: Partial<GitState> = {},
  hasWorkspace = true,
  view: GitView = 'changes',
): string {
  return renderToStaticMarkup(
    createElement(ChangesView, {
      state: { ...initialGitState, ...state },
      hasWorkspace,
      onRefresh: refuse('refresh'),
      onLoadMore: refuse('page'),
      onSelect: refuse('select'),
      onReloadDiff: refuse('re-read the diff'),
      onCancelStatus: refuse('cancel the status'),
      onCancelDiff: refuse('cancel the diff'),
      view,
      onViewChange: refuse('switch view'),
      onLoadMoreHistory: refuse('page the history'),
      onCancelHistory: refuse('cancel the history'),
    }),
  );
}

describe('the status list', () => {
  it('draws each changed file under the band its two sides put it in', () => {
    const markup = render({
      status: {
        kind: 'ready',
        value: repository({
          entries: [
            entry('src/a.ts', { index: 'MODIFIED', worktree: 'UNMODIFIED' }),
            entry('src/b.ts', { index: 'UNMODIFIED', worktree: 'DELETED' }),
            entry('notes.md', { index: 'UNTRACKED', worktree: 'UNTRACKED' }),
          ],
          total: 3,
        }),
      },
    });
    expect(markup).toContain('Staged');
    expect(markup).toContain('Changes');
    expect(markup).toContain('Untracked');
    expect(markup).toContain('a.ts');
    expect(markup).toContain('notes.md');
    // The directory is drawn beside the name rather than folded into it.
    expect(markup).toContain('>src<');
  });

  it('says which branch and what the repository is in the middle of', () => {
    const markup = render({
      status: {
        kind: 'ready',
        value: repository({
          branch: 'feature/x',
          state: 'REBASING',
          upstream: { refName: 'refs/remotes/origin/feature/x', ahead: 2, behind: 1 },
        }),
      },
    });
    expect(markup).toContain('feature/x');
    expect(markup).toContain('rebase in progress');
    expect(markup).toContain('↑2 ↓1');
  });

  it('flags a conflicted file in words as well as colour', () => {
    const markup = render({
      status: {
        kind: 'ready',
        value: repository({
          entries: [entry('src/a.ts', { conflicted: true, index: 'ADDED', worktree: 'ADDED' })],
          total: 1,
        }),
      },
    });
    expect(markup).toContain('Conflicts');
    expect(markup).toContain('conflict');
  });

  it('says how much of a truncated list is shown, and offers the rest', () => {
    const markup = render({
      status: {
        kind: 'ready',
        value: repository({ entries: [entry('a.ts')], total: 4_000, truncated: true, cursor: 'c' }),
      },
    });
    // The numbers go through `toLocaleString`, so the assertion is on the
    // sentence rather than on one machine's thousands separator.
    expect(markup).toContain('Showing 1 of');
    expect(markup).toContain('changed files');
    expect(markup).toContain('Load more');
  });

  it('counts the paths it could not show rather than hiding them', () => {
    const markup = render({
      status: { kind: 'ready', value: repository({ unrepresentable: 2 }) },
    });
    expect(markup).toContain('2 paths could not be shown');
  });
});

describe('the arms that are not the happy path', () => {
  it('reports an empty repository as having no commits rather than as an error', () => {
    const markup = render({
      status: {
        kind: 'ready',
        value: { ...repository(), branch: undefined, head: undefined, entries: [], total: 0 },
      },
    });
    expect(markup).toContain('no commits yet');
    expect(markup).toContain('No commits yet, and nothing to report in the working tree.');
    expect(markup).not.toContain('role="alert"');
  });

  it('reports a clean repository as clean, and says nothing about its state', () => {
    const markup = render({ status: { kind: 'ready', value: repository() } });
    expect(markup).toContain('No changes in the working tree.');
    // `CLEAN` is deliberately not a badge.
    expect(markup).not.toContain('clean');
  });

  it('reports a workspace that is not a repository as a value, not a failure', () => {
    const markup = render({ status: { kind: 'notARepository' } });
    expect(markup).toContain('This workspace is not a Git repository.');
    // No error envelope: `role="alert"` is what one renders with.
    expect(markup).not.toContain('role="alert"');
    // And the reason a folder inside a repository reads this way is stated.
    expect(markup).toContain('never searches above it');
  });

  it('renders a refusal from the core as the whole envelope, with a retry', () => {
    const markup = render({ status: { kind: 'failed', error: REFUSAL } });
    expect(markup).toContain('role="alert"');
    expect(markup).toContain('object 4f2a is unreadable');
    expect(markup).toContain('retryable');
    expect(markup).toContain('Try again');
  });

  it('says a read became an operation, and offers to stop it', () => {
    const markup = render({
      status: { kind: 'pending', operationId: 'op-1' as OperationId },
    });
    expect(markup).toContain('Reading the repository (operation)…');
    expect(markup).toContain('Cancel');
  });

  it('reports a cancelled read as cancelled, not as empty', () => {
    const markup = render({ status: { kind: 'cancelled' } });
    expect(markup).toContain('Read cancelled.');
    expect(markup).toContain('Read again');
  });

  it('says there is no workspace rather than an empty repository', () => {
    expect(render({}, false)).toContain('No workspace open.');
  });
});

describe('the diff of the selected file', () => {
  const selected = { selected: 'src/a.ts' as WorkspacePath };

  it('draws hunks as numbered lines, never as one blob of text', () => {
    const markup = render({ ...selected, diff: { kind: 'ready', value: summary() } });
    expect(markup).toContain('@@ -10,2 +10,3 @@');
    expect(markup).toContain('const b = 3;');
    expect(markup).toContain('git__line--added');
    expect(markup).toContain('git__line--removed');
    // Hard rule 5, checked on the markup: the content the core sent is drawn
    // in its own column and never prefixed into a `+`/`-` string.
    expect(markup).not.toContain('+const b = 3;');
    expect(markup).not.toContain('-const b = 2;');
  });

  it('states the change kind and both counts in words and numbers', () => {
    const markup = render({ ...selected, diff: { kind: 'ready', value: summary() } });
    expect(markup).toContain('modified');
    expect(markup).toContain('+2');
    expect(markup).toContain('−1');
    expect(markup).toContain('against HEAD');
  });

  it('names where a renamed file came from', () => {
    const markup = render({
      ...selected,
      diff: {
        kind: 'ready',
        value: summary({
          files: [file({ change: 'RENAMED', oldPath: 'src/old.ts' as WorkspacePath })],
        }),
      },
    });
    expect(markup).toContain('renamed');
    expect(markup).toContain('src/old.ts');
  });

  it('says a binary file is binary and shows no content for it', () => {
    const markup = render({
      ...selected,
      diff: {
        kind: 'ready',
        value: summary({ files: [file({ binary: true, hunks: [], size: 2048 })] }),
      },
    });
    expect(markup).toContain('Binary file, 2.0 KB');
    expect(markup).toContain('not shown');
    expect(markup).not.toContain('git__line');
  });

  it('says when a hunk or a file was cut short', () => {
    const markup = render({
      ...selected,
      diff: {
        kind: 'ready',
        value: summary({
          files: [
            file({
              hunksTruncated: true,
              hunks: [
                {
                  oldStart: 1,
                  oldLines: 1,
                  newStart: 1,
                  newLines: 1,
                  truncated: true,
                  lines: [{ kind: 'CONTEXT', content: 'x' }],
                },
              ],
            }),
          ],
        }),
      },
    });
    expect(markup).toContain('This hunk was cut short');
    expect(markup).toContain('diff stops short of the whole change');
  });

  it('explains an untracked file rather than showing an empty pane', () => {
    const markup = render({
      ...selected,
      status: {
        kind: 'ready',
        value: repository({
          entries: [entry('src/a.ts', { index: 'UNTRACKED', worktree: 'UNTRACKED' })],
          total: 1,
        }),
      },
      diff: { kind: 'ready', value: summary({ files: [], total: 0 }) },
    });
    expect(markup).toContain('No difference against');
    expect(markup).toContain('An untracked file is not part of a diff against a revision.');
  });

  it('carries the pending and failed arms of the diff read itself', () => {
    expect(
      render({ ...selected, diff: { kind: 'pending', operationId: 'op-2' as OperationId } }),
    ).toContain('Reading the diff (operation)…');
    const failed = render({ ...selected, diff: { kind: 'failed', error: REFUSAL } });
    expect(failed).toContain('role="alert"');
    expect(failed).toContain('object 4f2a is unreadable');
  });
});

describe('repository content is untrusted input', () => {
  it('escapes a branch name, a path and a diff line rather than rendering them as markup', () => {
    const hostile = '<script>alert(1)</script>';
    const markup = render({
      selected: hostile as WorkspacePath,
      status: {
        kind: 'ready',
        value: repository({
          branch: hostile,
          entries: [entry(hostile)],
          total: 1,
        }),
      },
      diff: {
        kind: 'ready',
        value: summary({
          files: [
            file({
              path: hostile as WorkspacePath,
              hunks: [
                {
                  oldStart: 1,
                  oldLines: 1,
                  newStart: 1,
                  newLines: 1,
                  truncated: false,
                  lines: [{ kind: 'ADDED', content: hostile }],
                },
              ],
            }),
          ],
        }),
      },
    });
    expect(tagsOf(markup).some((tag) => tag.startsWith('<script'))).toBe(false);
    expect(markup).toContain('&lt;script&gt;');
  });
});
