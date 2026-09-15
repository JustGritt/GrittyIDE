import type { DiffHunk, FileStatus, RepositoryStatus, WorkspacePath } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  amendWarning,
  bandVerbs,
  changeKindWord,
  commitBlocker,
  describeEntry,
  discardPlan,
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
    detached: false,
    state: 'CLEAN',
    entries: [],
    total: 0,
    truncated: false,
    unrepresentable: 0,
    ...overrides,
  };
}

describe('grouping a status page', () => {
  it('puts each entry in the band its two sides say it is in', () => {
    const groups = groupEntries([
      entry('a.txt', { index: 'MODIFIED', worktree: 'UNMODIFIED' }),
      entry('b.txt', { index: 'UNMODIFIED', worktree: 'DELETED' }),
      entry('c.txt', { index: 'UNTRACKED', worktree: 'UNTRACKED' }),
    ]);
    expect(groups.map((group) => group.id)).toEqual(['staged', 'unstaged', 'untracked']);
    expect(groups[0]?.entries.map((one) => one.path)).toEqual(['a.txt']);
    expect(groups[1]?.entries.map((one) => one.path)).toEqual(['b.txt']);
    expect(groups[2]?.entries.map((one) => one.path)).toEqual(['c.txt']);
  });

  it('shows a partially staged file in both bands, because it is in both', () => {
    const groups = groupEntries([entry('a.txt', { index: 'ADDED', worktree: 'MODIFIED' })]);
    expect(groups.map((group) => group.id)).toEqual(['staged', 'unstaged']);
    expect(groups[0]?.entries[0]?.path).toBe('a.txt');
    expect(groups[1]?.entries[0]?.path).toBe('a.txt');
  });

  it('puts a conflicted file only in Conflicts, whatever its two sides say', () => {
    const groups = groupEntries([
      entry('a.txt', { index: 'MODIFIED', worktree: 'MODIFIED', conflicted: true }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0]?.id).toBe('conflicted');
    expect(groups[0]?.label).toBe('Conflicts');
  });

  it('drops empty bands rather than drawing a heading with nothing under it', () => {
    expect(groupEntries([])).toEqual([]);
  });

  it('keeps an ignored entry visible when the caller asked for ignored files', () => {
    const groups = groupEntries([entry('target', { index: 'IGNORED', worktree: 'IGNORED' })]);
    expect(groups.map((group) => group.id)).toEqual(['untracked']);
  });
});

describe('an entry as the row draws it', () => {
  it('is a two-column code, index side first', () => {
    expect(statusCode(entry('a', { index: 'MODIFIED', worktree: 'UNMODIFIED' }))).toBe('M·');
    expect(statusCode(entry('a', { index: 'UNMODIFIED', worktree: 'DELETED' }))).toBe('·D');
    expect(statusCode(entry('a', { index: 'UNTRACKED', worktree: 'UNTRACKED' }))).toBe('??');
  });

  it('says the same thing in words, so the code is never the only channel', () => {
    const described = describeEntry(
      entry('a', { index: 'RENAMED', worktree: 'MODIFIED', renamedFrom: 'b' as WorkspacePath }),
    );
    expect(described).toContain('staged: renamed');
    expect(described).toContain('worktree: modified');
    expect(described).toContain('renamed from b');
  });

  it('says a submodule is reported rather than entered', () => {
    expect(describeEntry(entry('vendor/lib', { kind: 'SUBMODULE' }))).toContain(
      'submodule (reported, not entered)',
    );
  });

  it('splits a path into the part that identifies it and the part that locates it', () => {
    expect(fileName('src/features/git/a.ts')).toBe('a.ts');
    expect(fileDirectory('src/features/git/a.ts')).toBe('src/features/git');
    expect(fileName('README.md')).toBe('README.md');
    expect(fileDirectory('README.md')).toBeNull();
  });
});

describe('the repository as the bar draws it', () => {
  it('names the branch when there is one', () => {
    expect(headLine(repository({ branch: 'main' }))).toBe('main');
  });

  it('says detached only when the wire says detached', () => {
    expect(headLine(repository({ detached: true, head: 'abcdef1234567890' }))).toBe(
      'detached at abcdef1',
    );
    // Both fields are on the wire; a build that inferred one from the other
    // would eventually infer it wrong.
    expect(headLine(repository({ detached: false, head: 'abcdef1234567890' }))).toBe('at abcdef1');
  });

  it('says an unborn branch is a repository with no commits, not a failure', () => {
    expect(headLine(repository())).toBe('no commits yet');
  });

  it('names what the repository is in the middle of, and says nothing when it is clean', () => {
    expect(stateLabel('CLEAN')).toBeNull();
    expect(stateLabel('MERGING')).toBe('merge in progress');
    expect(stateLabel('BARE')).toBe('bare repository');
  });

  it('shows a state from a newer core as it arrived rather than folding it into one it knows', () => {
    expect(stateLabel('SOMETHING_NEW' as never)).toBe('unrecognised state (SOMETHING_NEW)');
  });

  it('reports divergence only when there is an upstream to diverge from', () => {
    expect(upstreamLine(undefined)).toBeNull();
    expect(upstreamLine({ refName: 'refs/remotes/origin/main', ahead: 0, behind: 0 })).toBe(
      'up to date',
    );
    expect(upstreamLine({ refName: 'refs/remotes/origin/main', ahead: 2, behind: 1 })).toBe(
      '↑2 ↓1',
    );
    expect(upstreamLine({ refName: 'refs/remotes/origin/main', ahead: 0, behind: 3 })).toBe('↓3');
  });

  it('shortens an object id to the length git log --oneline uses', () => {
    expect(shortId('0123456789abcdef')).toBe('0123456');
  });
});

describe('a hunk as the diff draws it', () => {
  const hunk: DiffHunk = {
    oldStart: 10,
    oldLines: 3,
    newStart: 12,
    newLines: 4,
    truncated: false,
    lines: [
      { kind: 'CONTEXT', content: 'before' },
      { kind: 'REMOVED', content: 'gone' },
      { kind: 'ADDED', content: 'new' },
      { kind: 'ADDED', content: 'newer' },
      { kind: 'CONTEXT', content: 'after' },
    ],
  };

  it('numbers each line on the side it exists on, and only that side', () => {
    expect(numberHunk(hunk)).toEqual([
      { kind: 'CONTEXT', content: 'before', oldNo: 10, newNo: 12 },
      { kind: 'REMOVED', content: 'gone', oldNo: 11, newNo: null },
      { kind: 'ADDED', content: 'new', oldNo: null, newNo: 13 },
      { kind: 'ADDED', content: 'newer', oldNo: null, newNo: 14 },
      { kind: 'CONTEXT', content: 'after', oldNo: 12, newNo: 15 },
    ]);
  });

  it('composes the @@ header out of the four numbers rather than parsing one', () => {
    expect(hunkHeader(hunk)).toBe('@@ -10,3 +12,4 @@');
  });

  it('handles a hunk that adds to an empty file, where the old side starts at 0', () => {
    const added: DiffHunk = {
      oldStart: 0,
      oldLines: 0,
      newStart: 1,
      newLines: 1,
      truncated: false,
      lines: [{ kind: 'ADDED', content: 'first' }],
    };
    expect(numberHunk(added)).toEqual([{ kind: 'ADDED', content: 'first', oldNo: null, newNo: 1 }]);
    expect(hunkHeader(added)).toBe('@@ -0,0 +1,1 @@');
  });

  it('signs a line without ever prefixing the content itself', () => {
    expect(lineSign('ADDED')).toBe('+');
    expect(lineSign('REMOVED')).toBe('−');
    expect(lineSign('CONTEXT')).toBe(' ');
    // The content the core sent is untouched: the sign is a separate column,
    // which is what keeps a line starting with `-` from reading as a removal.
    expect(numberHunk(hunk)[1]?.content).toBe('gone');
  });

  it('names a change kind in words', () => {
    expect(changeKindWord('TYPE_CHANGED')).toBe('type changed');
    expect(changeKindWord('RENAMED')).toBe('renamed');
  });

  it('sizes a binary file the way the tree sizes one', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
  });
});

describe('the write half (T102-U)', () => {
  const p = (path: string) => path as WorkspacePath;

  function repo(overrides: Partial<RepositoryStatus> = {}): RepositoryStatus {
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

  describe('which verbs a band offers', () => {
    it('stages or discards changes, only unstages what is staged, and leaves conflicts alone', () => {
      expect(bandVerbs('unstaged')).toEqual(['stage', 'discard']);
      expect(bandVerbs('untracked')).toEqual(['stage', 'discard']);
      expect(bandVerbs('staged')).toEqual(['unstage']);
      expect(bandVerbs('conflicted')).toEqual([]);
    });
  });

  describe('the discard confirmation', () => {
    it('says a tracked change is stashed, and how to get it back', () => {
      const plan = discardPlan([p('src/a.ts')], [entry('src/a.ts')]);
      expect(plan.stashed).toEqual(['src/a.ts']);
      expect(plan.trashed).toEqual([]);
      const wording = discardWording(plan);
      expect(wording.title).toBe('Discard changes to a.ts?');
      expect(wording.confirm).toBe('Discard changes');
      const text = wording.lines.join(' ');
      expect(text).toContain('stashed');
      expect(text).toContain('nothing is deleted');
      expect(text).toContain('`git stash apply`');
      expect(text).not.toContain('Trash');
    });

    it('says an untracked file goes to the Trash, because Git holds no copy of it', () => {
      const plan = discardPlan(
        [p('notes.md')],
        [entry('notes.md', { index: 'UNTRACKED', worktree: 'UNTRACKED' })],
      );
      expect(plan.trashed).toEqual(['notes.md']);
      const wording = discardWording(plan);
      expect(wording.title).toBe('Move notes.md to the Trash?');
      expect(wording.confirm).toBe('Move to Trash');
      const text = wording.lines.join(' ');
      expect(text).toContain('untracked');
      expect(text).toContain('Git holds no copy');
      expect(text).toContain('Trash');
      expect(text).not.toContain('stash');
    });

    it('warns that a partly staged file loses its staged half to the stash too', () => {
      const plan = discardPlan(
        [p('src/a.ts'), p('src/b.ts')],
        [entry('src/a.ts', { index: 'MODIFIED' }), entry('src/b.ts')],
      );
      expect(plan.alsoStaged).toEqual(['src/a.ts']);
      const wording = discardWording(plan);
      expect(wording.title).toBe('Discard changes to 2 files?');
      expect(wording.lines.join(' ')).toContain('a.ts also has staged changes');
    });

    it('describes both dispositions when a selection holds both', () => {
      const plan = discardPlan(
        [p('src/a.ts'), p('notes.md')],
        [entry('src/a.ts'), entry('notes.md', { worktree: 'UNTRACKED', index: 'UNTRACKED' })],
      );
      const text = discardWording(plan).lines.join(' ');
      expect(text).toContain('stashed');
      expect(text).toContain('Trash');
    });
  });

  describe('what a finished discard reports', () => {
    it('names the stash by commit, since stash@{0} goes stale as soon as anything else is stashed', () => {
      const lines = discardReport({
        changed: [p('a.ts')],
        repository: repo(),
        stash: { commit: 'f00ba1234567890', name: 'stash@{0}' },
        trashed: [p('dir/notes.md')],
      });
      expect(lines[0]).toContain('stash@{0} (f00ba12)');
      expect(lines[0]).toContain('`git stash apply f00ba12`');
      expect(lines[1]).toBe('Moved to the Trash: notes.md.');
    });

    it('says when there was nothing to save rather than inventing a stash', () => {
      expect(discardReport({ changed: [p('a.ts')], repository: repo(), trashed: [] })).toEqual([
        'Nothing needed saving: there were no changes left to discard.',
      ]);
    });
  });

  describe('whether git can write', () => {
    it('treats an unknown verdict as a reason, never as a yes', () => {
      expect(writeUnavailableReason(null)).toContain('not known yet');
    });

    it('passes the probe’s own reason through, and says nothing when git is there', () => {
      expect(
        writeUnavailableReason({ available: false, reason: 'git was not found on PATH' }),
      ).toBe('git was not found on PATH');
      expect(writeUnavailableReason({ available: true, version: 'git version 2.50.1' })).toBeNull();
    });
  });

  describe('why the commit button is disabled', () => {
    const staged = [entry('a.ts', { index: 'MODIFIED', worktree: 'UNMODIFIED' })];

    it('wants a message that is more than whitespace', () => {
      expect(commitBlocker(repo({ entries: staged }), { message: '  \n', amend: false })).toBe(
        'Write a commit message first.',
      );
      expect(commitBlocker(repo({ entries: staged }), { message: 'fix', amend: false })).toBeNull();
    });

    it('wants something staged, unless it is an amend', () => {
      const nothing = repo({ entries: [entry('a.ts')] });
      expect(commitBlocker(nothing, { message: 'fix', amend: false })).toBe('Nothing is staged.');
      expect(commitBlocker(nothing, { message: 'reword', amend: true })).toBeNull();
    });

    it('does not claim nothing is staged when the list stops short of the whole repository', () => {
      const partial = repo({ entries: [entry('a.ts')], truncated: true, total: 9000, cursor: 'c' });
      expect(commitBlocker(partial, { message: 'fix', amend: false })).toBeNull();
    });

    it('has nothing to amend on a branch with no commits', () => {
      expect(
        commitBlocker(repo({ head: undefined, entries: staged }), { message: 'x', amend: true }),
      ).toBe('There is no commit to amend yet.');
    });
  });

  describe('the danger of an amend, stated', () => {
    it('says when the upstream already has the commit', () => {
      const text = amendWarning(
        repo({ upstream: { refName: 'refs/remotes/origin/main', ahead: 0, behind: 0 } }),
      );
      expect(text).toContain('origin/main already has');
      expect(text).toContain('diverge');
      expect(text).toContain('as of the last fetch');
    });

    it('says when it has not been pushed, and when there is no upstream at all', () => {
      expect(
        amendWarning(
          repo({ upstream: { refName: 'refs/remotes/origin/main', ahead: 2, behind: 0 } }),
        ),
      ).toContain('does not have it yet');
      expect(amendWarning(repo())).toContain('no upstream');
    });
  });

  describe('a refusal from git itself', () => {
    it('carries the hook’s output out of the envelope', () => {
      expect(
        gitRefusal({
          code: 'Conflict',
          message: 'git commit refused the write',
          retryable: false,
          details: { subcommand: 'commit', exitCode: 1, stderr: 'pre-commit: lint failed' },
        }),
      ).toEqual({ subcommand: 'commit', exitCode: 1, output: 'pre-commit: lint failed' });
    });

    it('is not a dirty-document refusal, a held lock, or an envelope of another shape', () => {
      expect(
        gitRefusal({
          code: 'Conflict',
          message: 'unsaved',
          retryable: false,
          details: { dirtyDocuments: [{ documentId: 'd', path: 'a.ts' }] },
        }),
      ).toBeNull();
      expect(
        gitRefusal({ code: 'Conflict', message: 'index.lock is held', retryable: true }),
      ).toBeNull();
      expect(
        gitRefusal({
          code: 'Io',
          message: 'x',
          retryable: false,
          details: { stderr: 'y', subcommand: 'z' },
        }),
      ).toBeNull();
    });
  });
});
