import type { SearchMatch, SearchSummary, WorkspaceId, WorkspacePath } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { SEARCH_PAGE } from './search-model';
import { SearchPanel } from './search-panel';
import type { FileResult, SearchRun } from './search-reducer';
import { DEFAULT_TOGGLES, type SearchController } from './use-search';

/**
 * What the Search tab puts on the screen (T080-U), as static markup — the
 * pattern `changes-view-render.test.ts` set for the Changes tab.
 *
 * The model tests prove the sentences; this proves the panel is reading them,
 * and every arm is drawn, including the ones a happy path never reaches.
 */

const WS = 'ws-1' as WorkspaceId;

function match(path: string, line: number, overrides: Partial<SearchMatch> = {}): SearchMatch {
  return {
    path: path as WorkspacePath,
    kind: 'content',
    line,
    column: 9,
    snippet: `  const needle = ${line};`,
    truncated: false,
    matchStart: 8,
    matchLength: 6,
    ...overrides,
  };
}

function file(
  path: string,
  lines: SearchMatch[],
  nameMatch: SearchMatch | null = null,
): FileResult {
  return { path: path as WorkspacePath, nameMatch, lines };
}

function summary(overrides: Partial<SearchSummary> = {}): SearchSummary {
  return {
    filesScanned: 10,
    filesMatched: 2,
    directoriesScanned: 1,
    bytesScanned: 1,
    matchesFound: 3,
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
    phase: 'completed',
    operationId: 'op-1' as SearchRun['operationId'],
    files: [],
    matchCount: 0,
    coreUndelivered: 0,
    heldUndelivered: 0,
    filesScanned: null,
    summary: summary(),
    error: null,
    held: [],
    heldDropped: {},
    stopRequested: false,
    ...overrides,
  };
}

function controller(overrides: Partial<SearchController> = {}): SearchController {
  const noop = () => {};
  return {
    workspaceId: WS,
    query: 'needle',
    toggles: DEFAULT_TOGGLES,
    run: null,
    limit: SEARCH_PAGE,
    collapsed: new Set(),
    setQuery: noop,
    setToggle: noop,
    submit: noop,
    stop: noop,
    showMore: noop,
    toggleFile: noop,
    ...overrides,
  };
}

function render(search: SearchController): string {
  return renderToStaticMarkup(createElement(SearchPanel, { search, onOpen: () => {} }));
}

describe('the field', () => {
  it('is disabled with no workspace, and says why', () => {
    const markup = render(controller({ workspaceId: null, query: '' }));
    expect(markup).toContain('Open a folder to search it.');
    expect(markup).toMatch(/<input[^>]*disabled/);
  });

  it('shows the toggles as pressed buttons, and the scope', () => {
    const markup = render(
      controller({ toggles: { ...DEFAULT_TOGGLES, useRegex: true, scope: 'content' } }),
    );
    expect(markup).toContain('aria-label="Use regular expression"');
    expect(markup).toMatch(/aria-pressed="true"[^>]*aria-label="Use regular expression"/);
    expect(markup).toMatch(/aria-pressed="false"[^>]*aria-label="Match case"/);
    expect(markup).toMatch(/<option value="content" selected="">Contents<\/option>/);
  });

  it('draws nothing under the field before a search', () => {
    const markup = render(controller({ query: '' }));
    expect(markup).not.toContain('search__status');
    expect(markup).not.toContain('search__results');
  });

  it('says an invalid pattern beside the field, not as an alert, and drops the results', () => {
    const markup = render(
      controller({
        run: run({
          phase: 'failed',
          operationId: null,
          summary: null,
          error: { code: 'InvalidInput', message: 'regex parse error', retryable: false },
        }),
      }),
    );
    expect(markup).toContain('aria-invalid="true"');
    expect(markup).toContain('aria-describedby="search-invalid"');
    expect(markup).toMatch(/<p class="search__invalid" id="search-invalid">regex parse error<\/p>/);
    expect(markup).not.toContain('role="alert"');
    expect(markup).not.toContain('search__results');
  });
});

describe('a running search', () => {
  it('shows the live count and a Stop, and is not announced on every batch', () => {
    const markup = render(
      controller({
        run: run({
          phase: 'running',
          summary: null,
          matchCount: 1,
          filesScanned: 4,
          files: [file('src/a.ts', [match('src/a.ts', 3)])],
        }),
      }),
    );
    expect(markup).toContain('Searching… 1 result so far, 4 files searched');
    expect(markup).toContain('aria-live="off"');
    expect(markup).toMatch(/<button type="button">Stop<\/button>/);
  });

  it('disables Stop once it was pressed', () => {
    const markup = render(controller({ run: run({ phase: 'running', stopRequested: true }) }));
    expect(markup).toMatch(/<button type="button" disabled="">Stop<\/button>/);
  });
});

describe('results', () => {
  const files = [
    file('src/a.ts', [match('src/a.ts', 3), match('src/a.ts', 12)]),
    file('lib/needle.ts', [], {
      path: 'lib/needle.ts' as WorkspacePath,
      kind: 'filename',
      snippet: 'lib/needle.ts',
      truncated: false,
      matchStart: 4,
      matchLength: 6,
    }),
  ];

  it('groups by file with the name, directory and count, and highlights the match', () => {
    const markup = render(controller({ run: run({ files, matchCount: 3 }) }));
    expect(markup).toContain('3 results in 2 files');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain('<span class="search__name">a.ts</span>');
    expect(markup).toContain('<span class="search__dir">src</span>');
    expect(markup).toContain('title="2 results">2</span>');
    expect(markup).toContain(
      '<span class="search__line">3</span><span class="search__snippet">const <mark class="search__hit">needle</mark> = 3;</span>',
    );
    expect(markup).toContain('title="src/a.ts:12:9"');
    // A name match is a row of its own, pointing at the file and nothing else.
    expect(markup).toContain(
      '<span class="search__line">name</span><span class="search__snippet">lib/<mark class="search__hit">needle</mark>.ts</span>',
    );
    expect(markup).toContain('title="lib/needle.ts"');
    expect(markup).not.toContain('No results');
  });

  it('marks a line the core shortened', () => {
    const long = file('big.txt', [
      match('big.txt', 1, { truncated: true, snippet: '…x needle y…', matchStart: 3 }),
    ]);
    const markup = render(controller({ run: run({ files: [long], matchCount: 1 }) }));
    expect(markup).toContain('title="big.txt:1:9 — the line is longer than shown"');
    expect(markup).toContain('…x <mark class="search__hit">needle</mark> y…');
  });

  it('folds a collapsed file to its header', () => {
    const markup = render(
      controller({ run: run({ files, matchCount: 3 }), collapsed: new Set(['src/a.ts']) }),
    );
    expect(markup).toMatch(/aria-expanded="false"[^>]*title="src\/a.ts"/);
    expect(markup).not.toContain('title="src/a.ts:3:9"');
  });

  it('draws a page and offers the rest', () => {
    const markup = render(controller({ run: run({ files, matchCount: 3 }), limit: 1 }));
    expect(markup).toContain('title="src/a.ts:3:9"');
    expect(markup).not.toContain('title="src/a.ts:12:9"');
    expect(markup).toContain('1 more result in this file not shown');
    expect(markup).toContain('2 more results not shown.');
    expect(markup).toContain('>Show more</button>');
  });

  it('says No results when a finished search found nothing', () => {
    expect(render(controller({ run: run() }))).toContain('No results');
  });

  it('writes every completeness note out', () => {
    const markup = render(
      controller({
        run: run({
          files,
          matchCount: 3,
          coreUndelivered: 2,
          summary: summary({
            limitReached: true,
            skipped: { ...summary().skipped, binary: 1, ignoredDirectories: 2 },
          }),
        }),
      }),
    );
    expect(markup).toContain('search__note--warn');
    expect(markup).toContain('result limit');
    expect(markup).toContain('2 matches were found but not delivered');
    expect(markup).toContain('1 file not searched: 1 binary.');
    expect(markup).toContain('2 ignored directories not searched.');
  });

  it('keeps the results beside an error envelope when the operation fails', () => {
    const markup = render(
      controller({
        run: run({
          phase: 'failed',
          summary: null,
          files,
          matchCount: 3,
          error: { code: 'Io', message: 'the root went away', retryable: false },
        }),
      }),
    );
    expect(markup).toContain('role="alert"');
    expect(markup).toContain('the root went away');
    expect(markup).toContain('3 results in 2 files before it failed');
    expect(markup).toContain('title="src/a.ts:3:9"');
    expect(markup).not.toContain('aria-invalid="true"');
  });

  it('keeps what a stopped search found', () => {
    const markup = render(controller({ run: run({ phase: 'cancelled', files, matchCount: 3 }) }));
    expect(markup).toContain('Stopped — 3 results in 2 files found before it stopped');
    expect(markup).not.toMatch(/>Stop<\/button>/);
  });

  it('escapes repository content rather than rendering it', () => {
    const hostile = file('x.html', [
      match('x.html', 1, { snippet: '<img src=x onerror=alert(1)> needle', matchStart: 29 }),
    ]);
    const markup = render(controller({ run: run({ files: [hostile], matchCount: 1 }) }));
    expect(markup).not.toContain('<img');
    expect(markup).toContain(
      '&lt;img src=x onerror=alert(1)&gt; <mark class="search__hit">needle</mark>',
    );
  });
});
