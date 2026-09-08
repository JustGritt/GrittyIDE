import type { DiffHunk, FileStatus, RepositoryStatus, WorkspacePath } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
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
