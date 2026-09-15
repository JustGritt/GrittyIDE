import type { SearchMatch, SearchSummary, WorkspacePath } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  headlineOf,
  highlightOf,
  notesOf,
  pageOf,
  plural,
  SEARCH_PAGE,
  skippedOf,
} from './search-model';
import type { FileResult, SearchRun } from './search-reducer';

/**
 * What the panel says and draws about a run (T080-U), without a DOM.
 *
 * Every sentence about completeness is asserted here, because each of them is
 * a way the list could look whole and not be.
 */

function line(path: string, number: number): SearchMatch {
  return {
    path: path as WorkspacePath,
    kind: 'content',
    line: number,
    column: 1,
    snippet: 'needle',
    truncated: false,
    matchStart: 0,
    matchLength: 6,
  };
}

function file(path: string, lines: number, named = false): FileResult {
  return {
    path: path as WorkspacePath,
    nameMatch: named
      ? {
          path: path as WorkspacePath,
          kind: 'filename',
          snippet: path,
          truncated: false,
          matchStart: 0,
          matchLength: 1,
        }
      : null,
    lines: Array.from({ length: lines }, (_, index) => line(path, index + 1)),
  };
}

function summary(overrides: Partial<SearchSummary> = {}): SearchSummary {
  return {
    filesScanned: 10,
    filesMatched: 1,
    directoriesScanned: 1,
    bytesScanned: 1,
    matchesFound: 1,
    snippetsTruncated: 0,
    skipped: {
      binary: 0,
      invalidUtf8: 0,
      tooLarge: 0,
      unreadable: 0,
      outsideRoot: 0,
      ignoredDirectories: 0,
    },
    limitReached: false,
    elapsedMs: 1,
    ...overrides,
  };
}

function run(overrides: Partial<SearchRun> = {}): SearchRun {
  return {
    token: 1,
    query: 'needle',
    options: {},
    phase: 'running',
    operationId: null,
    files: [],
    matchCount: 0,
    coreUndelivered: 0,
    heldUndelivered: 0,
    filesScanned: null,
    summary: null,
    error: null,
    held: [],
    heldDropped: {},
    stopRequested: false,
    ...overrides,
  };
}

describe('highlightOf', () => {
  it('splits the snippet at the match and drops the indentation before it', () => {
    expect(highlightOf({ snippet: '\t  let needle = 1;', matchStart: 7, matchLength: 6 })).toEqual({
      before: 'let ',
      match: 'needle',
      after: ' = 1;',
    });
  });

  it('indexes by UTF-16 code unit, as the wire does', () => {
    // '😀' is two code units, so the match starts at 3, not 2.
    expect(highlightOf({ snippet: 'a😀needle', matchStart: 3, matchLength: 6 })).toEqual({
      before: 'a😀',
      match: 'needle',
      after: '',
    });
  });

  it('clamps a range that runs off the snippet rather than throwing', () => {
    expect(highlightOf({ snippet: 'short', matchStart: 3, matchLength: 99 })).toEqual({
      before: 'sho',
      match: 'rt',
      after: '',
    });
    expect(highlightOf({ snippet: 'short', matchStart: 40, matchLength: 2 })).toEqual({
      before: 'short',
      match: '',
      after: '',
    });
    expect(highlightOf({ snippet: 'short', matchStart: -4, matchLength: -1 }).match).toBe('');
  });

  it('keeps the ellipsis the core put on a windowed line', () => {
    const hit = highlightOf({ snippet: '…abc needle def…', matchStart: 5, matchLength: 6 });
    expect(hit.before).toBe('…abc ');
    expect(hit.after).toBe(' def…');
  });
});

describe('pageOf', () => {
  it('draws everything that fits', () => {
    const page = pageOf([file('a', 2), file('b', 1, true)], SEARCH_PAGE, new Set());
    expect(page.hidden).toBe(0);
    expect(page.files.map((entry) => entry.lines.length)).toEqual([2, 1]);
    expect(page.files[1]?.count).toBe(2);
  });

  it('cuts the page mid-file and counts what it left out', () => {
    const page = pageOf([file('a', 3), file('b', 4), file('c', 2)], 5, new Set());
    expect(page.files.map((entry) => entry.file.path)).toEqual(['a', 'b']);
    expect(page.files[1]?.lines).toHaveLength(2);
    expect(page.hidden).toBe(4);
  });

  it('spends a result on a name match', () => {
    const page = pageOf([file('a', 3, true)], 2, new Set());
    expect(page.files[0]?.lines).toHaveLength(1);
    expect(page.hidden).toBe(2);
  });

  it('lists a collapsed file without spending the page on it', () => {
    const page = pageOf([file('a', 400), file('b', 2)], 2, new Set(['a']));
    expect(page.files.map((entry) => [entry.file.path, entry.lines.length])).toEqual([
      ['a', 0],
      ['b', 2],
    ]);
    expect(page.hidden).toBe(0);
  });
});

describe('headlineOf', () => {
  it('says it is looking, and what it has found so far', () => {
    expect(headlineOf(run({ phase: 'starting' }))).toBe('Searching…');
    expect(headlineOf(run({ matchCount: 1 }))).toBe('Searching… 1 result so far');
    expect(headlineOf(run({ matchCount: 1200, filesScanned: 3 }))).toBe(
      'Searching… 1,200 results so far, 3 files searched',
    );
    expect(headlineOf(run({ matchCount: 2, stopRequested: true }))).toBe(
      'Stopping… 2 results so far',
    );
  });

  it('says how it ended', () => {
    const files = [file('a', 2), file('b', 1)];
    expect(headlineOf(run({ phase: 'completed' }))).toBe('No results');
    expect(headlineOf(run({ phase: 'completed', files, matchCount: 3 }))).toBe(
      '3 results in 2 files',
    );
    expect(headlineOf(run({ phase: 'cancelled', files, matchCount: 3 }))).toBe(
      'Stopped — 3 results in 2 files found before it stopped',
    );
    expect(headlineOf(run({ phase: 'failed', files, matchCount: 3 }))).toBe(
      '3 results in 2 files before it failed',
    );
  });

  it('says nothing beside an error when nothing was found', () => {
    expect(headlineOf(run({ phase: 'failed' }))).toBeNull();
  });
});

describe('notesOf', () => {
  it('is empty for a complete answer', () => {
    expect(notesOf(run({ phase: 'completed', summary: summary() }))).toEqual([]);
  });

  it('says the cap stopped the walk', () => {
    const notes = notesOf(run({ phase: 'completed', summary: summary({ limitReached: true }) }));
    expect(notes).toHaveLength(1);
    expect(notes[0]?.tone).toBe('warn');
    expect(notes[0]?.text).toMatch(/result limit/);
  });

  it('adds both halves of the undelivered count, while running too', () => {
    const notes = notesOf(run({ coreUndelivered: 2, heldUndelivered: 1 }));
    expect(notes).toEqual([
      {
        tone: 'warn',
        text: '3 matches were found but not delivered, so they are missing from this list.',
      },
    ]);
    expect(notesOf(run({ coreUndelivered: 1 }))[0]?.text).toBe(
      '1 match was found but not delivered, so it is missing from this list.',
    );
  });

  it('counts skipped files and ignored directories', () => {
    const skipped = {
      binary: 2,
      invalidUtf8: 0,
      tooLarge: 1,
      unreadable: 0,
      outsideRoot: 0,
      ignoredDirectories: 4,
    };
    const notes = notesOf(run({ phase: 'completed', summary: summary({ skipped }) }));
    expect(notes.map((note) => note.text)).toEqual([
      '3 files not searched: 2 binary, 1 too large.',
      '4 ignored directories not searched.',
    ]);
  });
});

describe('skippedOf', () => {
  it('names every reason', () => {
    expect(
      skippedOf(
        summary({
          skipped: {
            binary: 1,
            invalidUtf8: 1,
            tooLarge: 1,
            unreadable: 1,
            outsideRoot: 1,
            ignoredDirectories: 0,
          },
        }),
      ),
    ).toBe(
      '5 files not searched: 1 binary, 1 not UTF-8, 1 too large, 1 unreadable, 1 outside the workspace.',
    );
    expect(skippedOf(summary())).toBeNull();
  });
});

describe('plural', () => {
  it('formats and inflects', () => {
    expect(plural(1, 'file')).toBe('1 file');
    expect(plural(20000, 'file')).toBe('20,000 files');
    expect(plural(2, 'directory', 'directories')).toBe('2 directories');
  });
});
