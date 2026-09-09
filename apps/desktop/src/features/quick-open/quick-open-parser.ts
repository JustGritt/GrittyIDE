import type { QuickOpenMode } from './quick-open-types';

/**
 * What the raw field text means (T130-U, §1).
 *
 * The whole of the prefix rule lives here and nowhere else: the model, the
 * keymap and the component are all handed a `ParsedQuery` and never look at the
 * raw string again. Pure, so every prefix, the escape, and the whitespace rule
 * are a test rather than a click.
 *
 * The user's typing is the only input; nothing here reads a path, a buffer or
 * anything the core sent, so there is nothing to distrust (hard rule 13). The
 * `term` is handed on as data — matched against, never interpolated into
 * markup and never logged.
 */

export interface ParsedQuery {
  readonly mode: QuickOpenMode;
  /** What the mode should search for. Never carries the prefix. */
  readonly term: string;
}

/**
 * The character that selects each mode.
 *
 * A `Record` over `QuickOpenMode` rather than a loose map, so adding a mode to
 * the shared type is a compile error here until it has decided whether it has
 * a prefix. `files` has none, which is exactly why it is the default and why
 * the escape rule below has to exist.
 */
export const QUICK_OPEN_PREFIX: Readonly<Record<QuickOpenMode, string>> = {
  files: '',
  commands: '>',
  line: ':',
  symbols: '@',
};

const MODE_BY_PREFIX: ReadonlyMap<string, QuickOpenMode> = new Map(
  (Object.entries(QUICK_OPEN_PREFIX) as [QuickOpenMode, string][])
    .filter(([, prefix]) => prefix !== '')
    .map(([mode, prefix]) => [prefix, mode]),
);

const LEADING_SPACE = /^\s+/;

/**
 * Split the field text into a mode and a term.
 *
 * Three rules, in this order:
 *
 * 1. **Leading whitespace is insignificant** — before the prefix and after it,
 *    so `  >  save` is the command search `save`. Trailing whitespace is
 *    *kept*: a file name may legally end in a space (this repository ships
 *    `fixtures/weird-files` for exactly that reason), and eating it would make
 *    such a path the one thing quick open cannot reach. The line mode trims its
 *    own term, because there a space is never part of the answer.
 * 2. **The first character, and only the first, can be a prefix.** `a>b` is a
 *    file search for `a>b`; nothing rescans the middle of a query.
 * 3. **A doubled prefix escapes it.** `>>foo` is the *files* term `>foo`, not
 *    the command `>foo` — the doubling collapses to one literal character and
 *    the mode falls back to the default. That is what makes a path beginning
 *    with `>`, `:` or `@` reachable at all, which is the whole point of having
 *    an escape rather than a fourth mode. Only the first doubling is consumed,
 *    so `>>>x` is the files term `>>x`; a longer run is literal from the second
 *    character on, and no amount of typing gets stuck in a mode.
 */
export function parseQuery(raw: string): ParsedQuery {
  const trimmed = raw.replace(LEADING_SPACE, '');
  const first = trimmed[0];
  if (first === undefined) return { mode: 'files', term: '' };

  const prefixed = MODE_BY_PREFIX.get(first);
  if (prefixed === undefined) return { mode: 'files', term: trimmed };

  // The escape. Drop one character, keep the rest verbatim — including any
  // whitespace, which is now part of the literal term rather than padding
  // between a prefix and what follows it.
  if (trimmed[1] === first) return { mode: 'files', term: trimmed.slice(1) };

  return { mode: prefixed, term: trimmed.slice(1).replace(LEADING_SPACE, '') };
}
