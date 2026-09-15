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
import { ChangesView, type GitView, type WriteControls } from './changes-view';
import { type DiscardPlan, discardPlan } from './git-model';
import type { GitState, WriteRequest } from './git-reducer';
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

function writeControls(overrides: Partial<WriteControls> = {}): WriteControls {
  return {
    canDiscardAndRetry: false,
    discarding: null,
    onStage: refuse('stage'),
    onUnstage: refuse('unstage'),
    onRequestDiscard: refuse('ask to discard'),
    onConfirmDiscard: refuse('discard'),
    onCancelDiscard: refuse('cancel a discard'),
    onCommit: refuse('commit'),
    onDraftChange: refuse('change the draft'),
    onDiscardAndRetry: refuse('replay a discard'),
    onRetryWrite: refuse('retry a write'),
    onDismissWrite: refuse('dismiss a write'),
    onCancelWrite: refuse('cancel a write'),
    ...overrides,
  };
}

function render(
  state: Partial<GitState> = {},
  hasWorkspace = true,
  view: GitView = 'changes',
  writes: Partial<WriteControls> = {},
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
      writes: writeControls(writes),
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

// T102-U: the verbs. Rendered through the same view, because a reducer that
// tracks a write correctly and a box that never shows it is the defect these
// render tests exist to catch.
describe('the write half', () => {
  const GIT_OK = { available: true, version: 'git version 2.50.1' };
  const STAGED = entry('src/a.ts', { index: 'MODIFIED', worktree: 'UNMODIFIED' });
  const CHANGED = entry('src/b.ts');
  const UNTRACKED = entry('notes.md', { index: 'UNTRACKED', worktree: 'UNTRACKED' });
  const COMMIT: WriteRequest = {
    verb: 'commit',
    message: 'fix: it',
    amend: false,
    noVerify: false,
  };

  function withRepo(overrides: Partial<GitState> = {}, repo: Partial<RepositoryStatus> = {}) {
    return {
      status: {
        kind: 'ready' as const,
        value: repository({ entries: [STAGED, CHANGED, UNTRACKED], total: 3, ...repo }),
      },
      availability: GIT_OK,
      ...overrides,
    };
  }

  /** Every `<button …>` opening tag whose text includes `label`. */
  function button(markup: string, label: string): string | undefined {
    const match = markup.match(new RegExp(`<button[^>]*>(?:(?!</button>).)*${label}`));
    return match?.[0];
  }

  describe('the commit box', () => {
    it('shows the branch the commit lands on, and discloses that hooks run', () => {
      const markup = render(withRepo({}, { branch: 'feature/x' }));
      expect(markup).toContain('Commit to <span class="git__head">feature/x</span>');
      expect(markup).toContain('Commit message');
      expect(markup).toContain('hooks, which are the project');
    });

    it('is disabled with the reason while the message is empty', () => {
      const markup = render(withRepo());
      expect(button(markup, 'Commit<')).toContain('disabled');
      expect(markup).toContain('Write a commit message first.');
    });

    it('is disabled with the reason when nothing is staged', () => {
      const markup = render(
        withRepo(
          { draft: { message: 'fix', amend: false, noVerify: false } },
          { entries: [CHANGED] },
        ),
      );
      expect(button(markup, 'Commit<')).toContain('disabled');
      expect(markup).toContain('Nothing is staged.');
    });

    it('is enabled with a message and something staged', () => {
      const markup = render(withRepo({ draft: { message: 'fix', amend: false, noVerify: false } }));
      expect(button(markup, 'Commit<')).not.toContain('disabled');
    });

    it('offers amend and no-verify unticked, and says what each one costs when ticked', () => {
      const plain = render(withRepo());
      expect(plain).not.toContain('checked');
      const ticked = render(
        withRepo(
          { draft: { message: 'fix', amend: true, noVerify: true } },
          { upstream: { refName: 'refs/remotes/origin/main', ahead: 0, behind: 0 } },
        ),
      );
      expect(ticked).toContain('Amend commit');
      expect(ticked).toContain('origin/main already has');
      expect(ticked).toContain('the project&#x27;s own checks are skipped');
    });

    it('says the hooks are running while a commit is an operation, and offers to stop it', () => {
      const markup = render(
        withRepo({
          write: { kind: 'pending', request: COMMIT, operationId: 'op-9' as OperationId },
        }),
      );
      expect(markup).toContain('the hooks are running');
      expect(button(markup, 'Stop')).toBeDefined();
      expect(markup).toContain('readOnly');
    });
  });

  describe('when git cannot write', () => {
    it('shows every verb disabled with the probe’s reason, never absent', () => {
      const markup = render(
        withRepo({ availability: { available: false, reason: 'git was not found on PATH' } }),
      );
      expect(markup).toContain('Staging and committing are unavailable: git was not found on PATH');
      for (const label of ['Stage all', 'Unstage all', 'Discard all', 'Commit<']) {
        const tag = button(markup, label);
        expect(tag, label).toBeDefined();
        expect(tag, label).toContain('disabled');
      }
      expect(button(markup, 'Stage all')).toContain('title="git was not found on PATH"');
    });

    it('treats a verdict it has not received as unavailable rather than assuming yes', () => {
      const markup = render(withRepo({ availability: null }));
      expect(markup).toContain('not known yet');
      expect(button(markup, 'Stage all')).toContain('disabled');
    });
  });

  describe('the verbs on each band', () => {
    it('stages or discards changes and untracked files, and only unstages the staged band', () => {
      const markup = render(withRepo());
      expect(markup).toContain('aria-label="Unstage src/a.ts"');
      expect(markup).toContain('aria-label="Stage src/b.ts"');
      expect(markup).toContain('aria-label="Discard src/b.ts"');
      expect(markup).toContain('aria-label="Discard notes.md"');
      expect(markup).not.toContain('aria-label="Discard src/a.ts"');
      expect(markup).toContain('aria-label="Unstage all 1 in Staged"');
      expect(markup).toContain('aria-label="Discard all 1 in Untracked"');
    });

    it('offers no verb on a conflicted file', () => {
      const markup = render(
        withRepo(
          {},
          { entries: [entry('src/c.ts', { conflicted: true, index: 'ADDED', worktree: 'ADDED' })] },
        ),
      );
      expect(markup).not.toContain('aria-label="Stage src/c.ts"');
      expect(markup).not.toContain('Stage all');
    });

    it('disables the verbs while a write is in flight', () => {
      const markup = render(
        withRepo({ write: { kind: 'running', request: { verb: 'stage', paths: [CHANGED.path] } } }),
      );
      expect(button(markup, 'Stage all')).toContain('disabled');
      expect(markup).toContain('Staging b.ts…');
    });
  });

  describe('what a write said', () => {
    it('shows a hook’s refusal with its output, and keeps the message', () => {
      const markup = render(
        withRepo({
          draft: { message: 'fix: it', amend: false, noVerify: false },
          write: {
            kind: 'failed',
            request: COMMIT,
            error: {
              code: 'Conflict',
              message: 'git commit refused the write',
              retryable: false,
              details: {
                subcommand: 'commit',
                exitCode: 1,
                stderr: 'lint: 3 problems\n<script>x</script>',
              },
            },
          },
        }),
      );
      expect(markup).toContain('role="alert"');
      expect(markup).toContain('git commit refused (exit 1). Its output:');
      expect(markup).toContain('lint: 3 problems');
      // The hook's output is the repository's text: escaped, never markup.
      expect(tagsOf(markup).some((tag) => tag.startsWith('<script'))).toBe(false);
      expect(markup).toContain('The message is still in the box.');
      expect(markup).toContain('fix: it</textarea>');
      expect(markup).not.toContain('discard them and continue');
    });

    it('offers the dirty-document replay only when the hook says the refusal is one', () => {
      const failed = withRepo({
        write: {
          kind: 'failed',
          request: { verb: 'discard', paths: [CHANGED.path], force: false },
          error: {
            code: 'Conflict',
            message: 'unsaved',
            retryable: false,
            details: { dirtyDocuments: [{ documentId: 'd', path: 'src/b.ts' }] },
          },
        },
      });
      expect(render(failed, true, 'changes', { canDiscardAndRetry: true })).toContain(
        'discard them and continue',
      );
      expect(render(failed)).not.toContain('discard them and continue');
    });

    it('offers to try again only when the core says trying again can help', () => {
      const locked = withRepo({
        write: {
          kind: 'failed',
          request: { verb: 'stage', paths: [CHANGED.path] },
          error: { code: 'Conflict', message: 'index.lock is held', retryable: true },
        },
      });
      expect(button(render(locked), 'Try again')).toBeDefined();
    });

    it('says where a discard put the work, by commit id', () => {
      const markup = render(
        withRepo({
          write: {
            kind: 'done',
            request: { verb: 'discard', paths: [CHANGED.path], force: false },
            result: {
              changed: [CHANGED.path],
              repository: repository(),
              stash: { commit: 'f00ba1234567890', name: 'stash@{0}' },
              trashed: [],
            },
          },
        }),
      );
      expect(markup).toContain('Stashed as stash@{0} (f00ba12)');
      expect(markup).toContain('<code>git stash apply f00ba12</code>');
    });

    it('names the commit it made', () => {
      const markup = render(
        withRepo({
          write: {
            kind: 'done',
            request: COMMIT,
            result: {
              changed: [],
              repository: repository(),
              commit: 'f00ba1234567890',
              trashed: [],
            },
          },
        }),
      );
      expect(markup).toContain('Committed f00ba12 to main.');
    });

    it('says a stopped commit may or may not have been made', () => {
      const markup = render(withRepo({ write: { kind: 'cancelled', request: COMMIT } }));
      expect(markup).toContain('Commit stopped');
      expect(markup).toContain('may or may not');
    });
  });

  describe('the discard confirmation', () => {
    function dialog(plan: DiscardPlan): string {
      return render(withRepo(), true, 'changes', { discarding: plan });
    }

    it('is an alert dialog that says the change is stashed, and how to get it back', () => {
      const markup = dialog(discardPlan([CHANGED.path], [CHANGED]));
      expect(markup).toContain('role="alertdialog"');
      expect(markup).toContain('Discard changes to b.ts?');
      expect(markup).toContain('<code>git stash apply</code>');
      expect(button(markup, 'Discard changes')).toContain('dialog__danger');
    });

    it('says an untracked file goes to the Trash instead', () => {
      const markup = dialog(discardPlan([UNTRACKED.path], [UNTRACKED]));
      expect(markup).toContain('Move notes.md to the Trash?');
      expect(markup).toContain('Git holds no copy');
      expect(button(markup, 'Move to Trash')).toBeDefined();
    });

    it('lists the files of a multi-file discard, and counts past the first few', () => {
      const many = Array.from({ length: 10 }, (_, i) => entry(`f${i}.ts`));
      const markup = dialog(
        discardPlan(
          many.map((e) => e.path),
          many,
        ),
      );
      expect(markup).toContain('<code>f0.ts</code>');
      expect(markup).not.toContain('<code>f9.ts</code>');
      expect(markup).toContain('and 2 more');
    });

    it('is not on screen until a discard asks for it', () => {
      expect(render(withRepo())).not.toContain('alertdialog');
    });
  });
});
