import type { CoreErrorShape } from '@gritty/protocol';
import type { QuickCommand } from './quick-open-commands';
import type { ParsedQuery } from './quick-open-parser';
import type { OutlineEntry, OutlineKind, QuickOpenMode } from './quick-open-types';

/**
 * What the quick open list draws, in every state it can be in (T130-U, §2).
 *
 * Pure, and the only place a state is decided. The component switches on
 * `QuickOpenList.kind` and draws; it never asks "are there rows, and if not
 * why not". Following `run-console-model.ts`'s `PromptGate`: every state is
 * named and reachable, and none is left to a default nobody chose — including
 * the two empties the symbols mode has, which are different facts about the
 * world ("Gritty has no outline rules for this kind of file" is not "the rules
 * ran and nothing matched") and must not collapse into one blank list.
 *
 * Every path and every symbol name here arrived from the workspace or a buffer,
 * so it is untrusted text (hard rule 13). Nothing in this file parses,
 * resolves, joins or logs one; each is carried to the component and rendered
 * into a text node.
 *
 * **The seam, now that the contract exists.** `workspace.findFiles` landed with
 * T130-C, and `FileMatch` below is still not its response type. It is what a
 * *row* needs, which is less: no `score` (the wire says not to render it — the
 * scale is the scorer's bonuses and means nothing outside one response) and one
 * field the wire does not carry, `name`, because the basename is the row's
 * title and deriving it per render is work the controller does once. Keeping
 * the two apart is hard rule 4: what a provider returns is not domain state.
 * The prediction the provisional version made held — landing the command was a
 * mapping function and one added field.
 */

/**
 * The longest list quick open will draw.
 *
 * A bound, not a preference: the list is keyboard-driven and a thousand rows
 * are neither scannable nor free to render. Exceeding it produces the
 * `truncated` state, never a silently short `results` one (§4 — T080's contract
 * is explicit that a truncated result must not read as a complete one).
 */
export const QUICK_OPEN_MAX_ROWS = 50;

/**
 * One file the search found: what a row needs, mapped from `FoundFile`.
 *
 * Not a wire type, and it must not become one — see the module header.
 */
export interface FileMatch {
  /** Workspace-relative, as the core reported it. Untrusted text. */
  readonly path: string;
  /** The basename, for the row's title. Untrusted text. */
  readonly name: string;
  /**
   * Where the query matched `path`, as `[start, length]` spans in **char**
   * indices, ascending and non-overlapping.
   *
   * Straight from `FoundFile.ranges`, so the bar highlights without re-running
   * the matcher — the reason `workspace.findFiles` exists rather than a
   * client-side rank over `search.query`. Slice with `highlightSegments`, never
   * with `String.prototype.slice`: these index code points and JavaScript
   * strings index UTF-16 code units, and the two disagree past the BMP.
   */
  readonly ranges: readonly HighlightRange[];
}

/** `[start, length]`, in char indices. */
export type HighlightRange = readonly [start: number, length: number];

/**
 * Where the files query has got to.
 *
 * Four states because a search has four: never asked, in flight, answered, and
 * failed. A cancelled query is deliberately *not* one of them — cancellation is
 * not a failure and must not change the rows (the ticket's failure behaviour),
 * so the controller drops a cancelled response and leaves this state alone.
 */
export type FileQueryState =
  /** Nothing has been asked yet, or the field was emptied. */
  | { readonly status: 'idle' }
  | { readonly status: 'loading' }
  | {
      readonly status: 'ready';
      readonly matches: readonly FileMatch[];
      /** The core stopped early; §4 says this is always said out loud. */
      readonly truncated: boolean;
    }
  | { readonly status: 'failed'; readonly error: CoreErrorShape };

/**
 * The active document's outline (T130-S).
 *
 * `unsupported` is a real answer, not an absence: Gritty extracts an outline by
 * pattern and has rules for some kinds of file and not others, and a file it
 * has no rules for must say so rather than look like a file with no symbols in
 * it. Until T130-S lands, the mount supplies `unsupported` and the mode exists
 * and is honest about having nothing.
 */
export type OutlineState =
  | { readonly status: 'unsupported' }
  | {
      readonly status: 'ready';
      readonly entries: readonly OutlineEntry[];
      /**
       * The extractor hit a bound (`OUTLINE_LIMIT`, or the markdown byte cap)
       * and entries after it are missing. Carried through to the row list
       * because §4 is not scoped to the files mode: a truncated list must never
       * read as a complete one, whichever mode produced it.
       */
      readonly truncated: boolean;
    };

/** The document the `:` and `@` modes act on. */
export interface ActiveDocument {
  /** The tab's label. Untrusted text. */
  readonly name: string;
  /** Lines in the buffer, so an out-of-range jump clamps rather than fails. */
  readonly lineCount: number;
  readonly outline: OutlineState;
}

/** Everything the list is computed from. Assembled by the controller. */
export interface QuickOpenState {
  readonly query: ParsedQuery;
  readonly hasWorkspace: boolean;
  readonly files: FileQueryState;
  readonly commands: readonly QuickCommand[];
  /** `null` when no editor is open. */
  readonly document: ActiveDocument | null;
}

/**
 * One drawable row.
 *
 * `id` is positional (`file:0`, `command:2`) rather than the row's natural key,
 * for two reasons. HTML5 forbids whitespace in an `id`, and both a path and a
 * Markdown heading routinely contain one, so `aria-activedescendant` would
 * point at nothing the moment either did. And while results settle — a
 * debounced search answers, then answers again — the keyboard should stay on
 * the row *position* the user moved to rather than chase a path that has
 * scrolled away. The natural key is still on the row, in the field that means
 * it: `path`, `commandId`, `line`.
 */
export type QuickOpenRow =
  | {
      readonly kind: 'file';
      readonly id: string;
      readonly name: string;
      readonly path: string;
      /** Spans over `path`, for `highlightSegments`. Never over `name`. */
      readonly ranges: readonly HighlightRange[];
    }
  | {
      readonly kind: 'command';
      readonly id: string;
      readonly commandId: string;
      readonly title: string;
      readonly group: string;
      /** The reason it cannot run, or `null`. Never the empty string. */
      readonly disabled: string | null;
    }
  | {
      readonly kind: 'line';
      readonly id: string;
      /** Already clamped to the buffer: what accepting this row will do. */
      readonly line: number;
      readonly column: number;
      /** The whole sentence, including the clamp note when there is one. */
      readonly label: string;
      /** The request was out of range and was pulled back into it. */
      readonly clamped: boolean;
    }
  | {
      readonly kind: 'symbol';
      readonly id: string;
      readonly name: string;
      readonly symbol: OutlineKind;
      readonly line: number;
      readonly depth: number;
    };

/**
 * Why a mode has nothing to show.
 *
 * A named reason rather than prose, so the component can style them apart and
 * a test can assert which empty this is without matching a sentence. The two
 * symbol empties are the reason this is an enumeration at all.
 */
export type QuickOpenEmptyReason =
  /** The field is empty; the mode is waiting to be told what to look for. */
  | 'noQuery'
  /** Something was searched and matched nothing. */
  | 'noMatch'
  /** Files and symbols need a workspace; there is not one. */
  | 'noWorkspace'
  /** Line and symbols need an open editor; there is not one. */
  | 'noDocument'
  /** Gritty has no outline rules for this kind of file. */
  | 'noOutlineRules'
  /** It has rules, they ran, and there is nothing (or nothing matching). */
  | 'noOutlineMatch'
  /** The `:` term is not a line number. */
  | 'notALine';

/**
 * The list, total over every state the ticket names: loading, results,
 * truncated, empty (with its reason) and error.
 *
 * `mode` is on every variant because the header says which mode is active even
 * when there is nothing under it — a bar that goes blank without saying what it
 * was doing is the failure §2 is about.
 */
export type QuickOpenList =
  | { readonly kind: 'loading'; readonly mode: QuickOpenMode }
  | {
      readonly kind: 'results';
      readonly mode: QuickOpenMode;
      readonly rows: readonly QuickOpenRow[];
    }
  | {
      readonly kind: 'truncated';
      readonly mode: QuickOpenMode;
      readonly rows: readonly QuickOpenRow[];
      /** The notice the list shows. Decided here, so the render never does. */
      readonly notice: string;
    }
  | {
      readonly kind: 'empty';
      readonly mode: QuickOpenMode;
      readonly reason: QuickOpenEmptyReason;
      /** The sentence the empty row shows. Never blank (§ failure behaviour). */
      readonly message: string;
    }
  | { readonly kind: 'error'; readonly mode: QuickOpenMode; readonly error: CoreErrorShape };

/** A jump target the `:` mode understood, before it has been clamped. */
export interface LineTarget {
  readonly line: number;
  readonly column: number;
}

const LINE_TERM = /^(\d+)(?::(\d+))?$/;

/**
 * Read `120` or `120:8`, or `null` for anything else.
 *
 * Trims: `:120 ` is the same request as `:120`, because unlike a file name a
 * line number never has a space in it. Nothing is clamped here — clamping needs
 * the buffer, and this is the half that only needs the text.
 */
export function parseLineTarget(term: string): LineTarget | null {
  const match = LINE_TERM.exec(term.trim());
  if (match === null) return null;
  const [, rawLine, rawColumn] = match;
  if (rawLine === undefined) return null;
  return { line: Number(rawLine), column: rawColumn === undefined ? 1 : Number(rawColumn) };
}

/**
 * Whether `text` contains the letters of `term`, in order.
 *
 * Membership only, and case-insensitively: this decides which rows are in the
 * list, never what order they are in — source order is the order. **Ranking is
 * T130-R's**, and when it lands it replaces this for the modes that want it;
 * files never go through here at all, because those are ranked before they
 * arrive. A subsequence rather than a substring so `epn` finds
 * `editor-pane.tsx`, which is the thing a palette is for.
 */
function matchesTerm(term: string, text: string): boolean {
  if (term === '') return true;
  const needle = term.toLowerCase();
  const hay = text.toLowerCase();
  let at = 0;
  for (const character of needle) {
    const found = hay.indexOf(character, at);
    if (found === -1) return false;
    at = found + character.length;
  }
  return true;
}

function empty(mode: QuickOpenMode, reason: QuickOpenEmptyReason, message: string): QuickOpenList {
  return { kind: 'empty', mode, reason, message };
}

/**
 * Cap the rows, and say so when the cap bit.
 *
 * `overflowed` is the *core's* own truncation, folded in here so a list that
 * was cut in two places still reads as cut once. Either cause produces the
 * same state, because the user's question — "is this all of them?" — has the
 * same answer.
 */
function capped(
  mode: QuickOpenMode,
  rows: readonly QuickOpenRow[],
  overflowed: boolean,
): QuickOpenList {
  if (!overflowed && rows.length <= QUICK_OPEN_MAX_ROWS) return { kind: 'results', mode, rows };
  const shown = rows.slice(0, QUICK_OPEN_MAX_ROWS);
  return {
    kind: 'truncated',
    mode,
    rows: shown,
    notice: `Showing the first ${shown.length}. Narrow the search to see the rest.`,
  };
}

function filesList(state: QuickOpenState): QuickOpenList {
  const { term } = state.query;
  if (!state.hasWorkspace) {
    return empty('files', 'noWorkspace', 'Open a folder to search the files in it.');
  }
  // Before the state of the query, because an empty field has not asked
  // anything and a stale answer to a previous term is not this term's answer.
  if (term === '') return empty('files', 'noQuery', 'Type part of a file name.');
  switch (state.files.status) {
    case 'idle':
      // A term with nothing asked for it yet is the debounce window. It is a
      // wait, and drawing it as one is honest; drawing "no match" would be a
      // lie the user is about to see corrected.
      return { kind: 'loading', mode: 'files' };
    case 'loading':
      return { kind: 'loading', mode: 'files' };
    case 'failed':
      return { kind: 'error', mode: 'files', error: state.files.error };
    case 'ready': {
      const matches = state.files.matches;
      if (matches.length === 0) {
        return empty('files', 'noMatch', `No file matches "${term}".`);
      }
      const rows = matches.map(
        (match, index): QuickOpenRow => ({
          kind: 'file',
          id: `file:${index}`,
          name: match.name,
          path: match.path,
          ranges: match.ranges,
        }),
      );
      return capped('files', rows, state.files.truncated);
    }
  }
}

function commandsList(state: QuickOpenState): QuickOpenList {
  const { term } = state.query;
  // No workspace check: commands still work with nothing open, which is the
  // ticket's failure behaviour and the reason Open Folder is one of them.
  const rows = state.commands
    .filter((command) => matchesTerm(term, `${command.group} ${command.title}`))
    .map(
      (command, index): QuickOpenRow => ({
        kind: 'command',
        id: `command:${index}`,
        commandId: command.id,
        title: command.title,
        group: command.group,
        disabled: command.disabled ?? null,
      }),
    );
  if (rows.length === 0) {
    // An empty term matches everything, so reaching here with one means the
    // array itself is empty — a different sentence, because there is nothing
    // to narrow.
    const message = term === '' ? 'There are no commands.' : `No command matches "${term}".`;
    return empty('commands', 'noMatch', message);
  }
  return capped('commands', rows, false);
}

function lineList(state: QuickOpenState): QuickOpenList {
  const { term } = state.query;
  if (state.document === null) {
    return empty('line', 'noDocument', 'Open a file first; there is nothing to jump in.');
  }
  if (term.trim() === '') {
    return empty('line', 'noQuery', 'Type a line number, like 120 or 120:8.');
  }
  const target = parseLineTarget(term);
  if (target === null) {
    return empty('line', 'notALine', `"${term.trim()}" is not a line number.`);
  }
  const last = Math.max(1, state.document.lineCount);
  const line = Math.max(1, Math.min(target.line, last));
  // The column has no upper clamp here: bounding it needs the length of that
  // one line, which this model does not have. The editor's own reveal clamps
  // it, and inventing a bound from the line count would be a worse guess than
  // no bound at all.
  const column = Math.max(1, target.column);
  const clamped = line !== target.line;
  const place = target.column === 1 ? `line ${line}` : `line ${line}, column ${column}`;
  return {
    kind: 'results',
    mode: 'line',
    rows: [
      {
        kind: 'line',
        id: 'line:0',
        line,
        column,
        label: clamped ? `Go to ${place} — the file ends there.` : `Go to ${place}.`,
        clamped,
      },
    ],
  };
}

function symbolsList(state: QuickOpenState): QuickOpenList {
  const { term } = state.query;
  // Workspace before document, deliberately: with no folder open there is no
  // editor either, and "open a folder" is the actionable half of that.
  if (!state.hasWorkspace) {
    return empty('symbols', 'noWorkspace', 'Open a folder to outline the files in it.');
  }
  if (state.document === null) {
    return empty('symbols', 'noDocument', 'Open a file to see its outline.');
  }
  const outline = state.document.outline;
  if (outline.status === 'unsupported') {
    // The first of the two empties: the rules never ran, because there are
    // none for this kind of file. Saying "no symbols" here would claim the
    // file has none, which is a different and unearned statement.
    return empty('symbols', 'noOutlineRules', 'Gritty has no outline rules for this kind of file.');
  }
  const rows = outline.entries
    .filter((entry) => matchesTerm(term, entry.name))
    .map(
      (entry, index): QuickOpenRow => ({
        kind: 'symbol',
        id: `symbol:${index}`,
        name: entry.name,
        symbol: entry.kind,
        line: entry.line,
        depth: entry.depth,
      }),
    );
  if (rows.length === 0) {
    // The second empty: the rules ran. Whether the file has nothing in it or
    // the term excluded everything is the same fact about the outline, said
    // with the sentence that fits.
    const message =
      term === '' ? 'This file has nothing to outline.' : `No outline entry matches "${term}".`;
    return empty('symbols', 'noOutlineMatch', message);
  }
  // The outline's own bound, not just the row cap: an outline cut short at
  // OUTLINE_LIMIT is incomplete even when it fits in the list.
  return capped('symbols', rows, outline.truncated);
}

/** The list for the current state. Total over `QuickOpenMode`. */
export function quickOpenList(state: QuickOpenState): QuickOpenList {
  switch (state.query.mode) {
    case 'files':
      return filesList(state);
    case 'commands':
      return commandsList(state);
    case 'line':
      return lineList(state);
    case 'symbols':
      return symbolsList(state);
  }
}

/** The rows a list holds — none, for the three states that have no rows. */
export function rowsOf(list: QuickOpenList): readonly QuickOpenRow[] {
  switch (list.kind) {
    case 'results':
    case 'truncated':
      return list.rows;
    case 'loading':
    case 'empty':
    case 'error':
      return [];
  }
}

/**
 * The row ids, in order — the `readonly string[]` the explorer's `moveFocus`
 * and `focusAt` are generic over, so quick open reuses that list algebra
 * rather than growing a second one (§9).
 */
export function rowIds(list: QuickOpenList): readonly string[] {
  return rowsOf(list).map((row) => row.id);
}

/** A run of `text`, either matched by the query or not. */
export interface HighlightSegment {
  readonly text: string;
  readonly matched: boolean;
}

/**
 * Split `text` into matched and unmatched runs, for the bar to draw.
 *
 * **Slices by code point, not by `String.prototype.slice`.** `FoundFile.ranges`
 * are char indices — Unicode scalar values, which is what the Rust scorer walks
 * — and JavaScript string indices are UTF-16 code units. The two agree exactly
 * up to U+FFFF and disagree after it, so a path containing an emoji or a rarer
 * CJK character would highlight the wrong span, or half a surrogate pair, with
 * the obvious slice. `Array.from` iterates code points, which is the same unit.
 *
 * Total by construction: the segments always concatenate back to `text`, so a
 * malformed range cannot lose characters from the row. Ranges are assumed
 * ascending and non-overlapping, which the contract guarantees; anything that
 * would run backwards or past the end is clamped rather than trusted, because
 * this is workspace-derived input (rule 13) and a render must not throw.
 */
export function highlightSegments(
  text: string,
  ranges: readonly HighlightRange[],
): readonly HighlightSegment[] {
  const points = Array.from(text);
  if (ranges.length === 0) {
    return text === '' ? [] : [{ text, matched: false }];
  }
  const segments: HighlightSegment[] = [];
  let at = 0;
  const push = (from: number, to: number, matched: boolean) => {
    if (to <= from) return;
    segments.push({ text: points.slice(from, to).join(''), matched });
  };
  for (const [start, length] of ranges) {
    const from = Math.max(at, Math.min(start, points.length));
    const to = Math.max(from, Math.min(start + length, points.length));
    push(at, from, false);
    push(from, to, true);
    at = to;
  }
  push(at, points.length, false);
  return segments;
}
