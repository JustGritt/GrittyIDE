/**
 * What the search panel draws, decided without drawing it (T080-U).
 *
 * `search-reducer.ts` says what is known about a run; this file turns that into
 * the pieces the panel renders — the highlighted snippet, the page of results
 * that fits, and the sentences that say how complete the list is. Every
 * sentence the panel shows about a run is written here, so the tests can read
 * them without a DOM.
 */

import type { SearchMatch, SearchSummary } from '@gritty/protocol';
import { type FileResult, type SearchRun, undeliveredOf } from './search-reducer';

/**
 * How many results the panel draws before asking for "Show more".
 *
 * The core stops at 20 000 matches (T080 known limitation 4), and a list that
 * long would be re-rendered on every batch while a search streams. A page is
 * drawn instead, and the count beside it says what is not.
 */
export const SEARCH_PAGE = 500;

/** A snippet cut around its match, every offset a UTF-16 index into the string. */
export interface Highlight {
  readonly before: string;
  readonly match: string;
  readonly after: string;
}

/**
 * The snippet split at the match.
 *
 * Offsets are UTF-16 code units, which is what a JavaScript string indexes by,
 * so no conversion happens here. They are clamped anyway: the snippet is
 * repository content (rule 13) and a range that runs off its end must degrade
 * to a shorter highlight, not throw. Indentation before the match is dropped —
 * a result list is read for the match, and a line of tabs pushes it out of the
 * sidebar.
 */
export function highlightOf(
  match: Pick<SearchMatch, 'snippet' | 'matchStart' | 'matchLength'>,
): Highlight {
  const { snippet } = match;
  const start = clamp(match.matchStart, 0, snippet.length);
  const end = clamp(start + Math.max(0, match.matchLength), start, snippet.length);
  const before = snippet.slice(0, start);
  return {
    before: before.replace(/^\s+/, ''),
    match: snippet.slice(start, end),
    after: snippet.slice(end),
  };
}

/** One file as the panel draws it: possibly fewer lines than it has. */
export interface FilePage {
  readonly file: FileResult;
  readonly lines: readonly SearchMatch[];
  /** Results in this file, name match included. */
  readonly count: number;
}

export interface ResultPage {
  readonly files: readonly FilePage[];
  /** Results found and not drawn, because the page is full. */
  readonly hidden: number;
}

/**
 * The files and lines that fit in `limit` results, in arrival order.
 *
 * A collapsed file costs nothing against the limit — its lines are not drawn —
 * but it is still listed, so collapsing a file that is full of matches is how a
 * user reaches the files after it.
 */
export function pageOf(
  files: readonly FileResult[],
  limit: number,
  collapsed: ReadonlySet<string>,
): ResultPage {
  const page: FilePage[] = [];
  let budget = Math.max(0, limit);
  let hidden = 0;
  for (const file of files) {
    const count = countOf(file);
    if (collapsed.has(file.path)) {
      page.push({ file, lines: [], count });
      continue;
    }
    if (budget === 0) {
      hidden += count;
      continue;
    }
    // The name match is a row, and it spends one result like any other.
    const nameCost = file.nameMatch === null ? 0 : 1;
    const lineBudget = Math.max(0, budget - nameCost);
    const lines = file.lines.slice(0, lineBudget);
    page.push({ file, lines, count });
    const drawn = lines.length + Math.min(nameCost, budget);
    hidden += count - drawn;
    budget -= drawn;
  }
  return { files: page, hidden };
}

export function countOf(file: FileResult): number {
  return file.lines.length + (file.nameMatch === null ? 0 : 1);
}

/** A tone the panel colours by, and the words. */
export interface RunNote {
  readonly tone: 'info' | 'warn';
  readonly text: string;
}

/**
 * The headline: how many results, and whether the search is still looking.
 *
 * `null` for a run that failed before it had anything to count — the error
 * says what happened, and "0 results" beside it would read as an answer.
 */
export function headlineOf(run: SearchRun): string | null {
  const results = plural(run.matchCount, 'result');
  const files = plural(run.files.length, 'file');
  switch (run.phase) {
    case 'starting':
      return 'Searching…';
    case 'running': {
      if (run.stopRequested) return `Stopping… ${results} so far`;
      const scanned =
        run.filesScanned === null ? '' : `, ${plural(run.filesScanned, 'file')} searched`;
      return `Searching… ${results} so far${scanned}`;
    }
    case 'completed':
      return run.matchCount === 0 ? 'No results' : `${results} in ${files}`;
    case 'cancelled':
      return `Stopped — ${results} in ${files} found before it stopped`;
    case 'failed':
      return run.matchCount === 0 ? null : `${results} in ${files} before it failed`;
  }
}

/**
 * Everything that makes the list less than the whole answer, in words.
 *
 * Each of these is a way the list can look complete and not be, which is why
 * none of them is left to a tooltip: the cap, matches found and not delivered,
 * and files the walk declined to read.
 */
export function notesOf(run: SearchRun): readonly RunNote[] {
  const notes: RunNote[] = [];
  if (run.summary?.limitReached === true) {
    notes.push({
      tone: 'warn',
      // No number: the cap counts matches *found*, and some of those may be
      // the undelivered ones below, so the list's own count would misstate it.
      text: 'The search stopped at its result limit, so part of the workspace was not searched. Narrow the query to see everything.',
    });
  }
  const undelivered = undeliveredOf(run);
  if (undelivered > 0) {
    notes.push({
      tone: 'warn',
      text: `${plural(undelivered, 'match', 'matches')} ${undelivered === 1 ? 'was' : 'were'} found but not delivered, so ${undelivered === 1 ? 'it is' : 'they are'} missing from this list.`,
    });
  }
  if (run.summary !== null) {
    const skipped = skippedOf(run.summary);
    if (skipped !== null) notes.push({ tone: 'info', text: skipped });
    const ignored = run.summary.skipped.ignoredDirectories;
    if (ignored > 0) {
      notes.push({
        tone: 'info',
        text: `${plural(ignored, 'ignored directory', 'ignored directories')} not searched.`,
      });
    }
  }
  return notes;
}

/** "3 files not searched: 2 binary, 1 too large." — or `null` when none were. */
export function skippedOf(summary: SearchSummary): string | null {
  const { binary, invalidUtf8, tooLarge, unreadable, outsideRoot } = summary.skipped;
  const reasons: [number, string][] = [
    [binary, 'binary'],
    [invalidUtf8, 'not UTF-8'],
    [tooLarge, 'too large'],
    [unreadable, 'unreadable'],
    [outsideRoot, 'outside the workspace'],
  ];
  const present = reasons.filter(([count]) => count > 0);
  if (present.length === 0) return null;
  const total = present.reduce((sum, [count]) => sum + count, 0);
  const parts = present.map(([count, reason]) => `${formatCount(count)} ${reason}`).join(', ');
  return `${plural(total, 'file')} not searched: ${parts}.`;
}

export function plural(count: number, singular: string, pluralForm = `${singular}s`): string {
  return `${formatCount(count)} ${count === 1 ? singular : pluralForm}`;
}

const COUNT_FORMAT = new Intl.NumberFormat('en-US');

function formatCount(count: number): string {
  return COUNT_FORMAT.format(count);
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}
