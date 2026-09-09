import { createMarkdownInstance, inlineText } from '@gritty/markdown';
import type { OutlineEntry, OutlineKind } from './quick-open-types';

/**
 * The current file's **outline, extracted by pattern** from the open buffer
 * (T130-S) — Markdown headings, and the declaration keywords of the three
 * languages Gritty has rules for.
 *
 * It is deliberately not called *symbols*. Gritty has no language server (the
 * project plan defers LSP until the editing substrate is stable) and Monaco's
 * `DocumentSymbolProvider` registry is empty, so nothing here knows what a name
 * *means*: `impl` inside a string literal is an entry, a method built by a
 * macro is not, and a declaration split across lines is missed. Everything the
 * user sees says "outline" for that reason. The whole thing is one pure module
 * so it can be replaced by real symbols later, or deleted, without touching the
 * palette.
 *
 * Three properties the row model can rely on:
 *
 * - **Supported is not the same as empty.** `outlineOf` returns
 *   `{ supported, truncated, entries }` rather than a bare array, because
 *   "Gritty has no outline rules for Python" and "this Markdown file has no
 *   headings" are different sentences and the field must say the right one.
 *   `hasOutlineRules(languageId)` answers the first question alone, for a row
 *   model that has a language but no buffer yet. `result.supported` and
 *   `hasOutlineRules(languageId)` always agree.
 * - **Bounded.** At most {@link OUTLINE_LIMIT} entries, and Markdown longer
 *   than {@link MAX_MARKDOWN_BYTES} is parsed only up to a line boundary;
 *   either way `truncated` is true and the field is expected to say so.
 * - **Linear on hostile input** (AGENTS.md rule 13). Repository content is
 *   untrusted: every pattern here is anchored, applies to at most
 *   {@link MAX_LINE_SCAN} characters of a line, and contains no nested
 *   quantifier that could backtrack. Markdown reuses markdown-it's *block*
 *   parser with the inline chain switched off — inline tokenization is
 *   superlinear (a megabyte of `![` costs seconds; block-only, 7ms), and the
 *   outline needs headings, not emphasis. Heading titles are then run through
 *   the same `inlineText` the preview uses, one short capped string at a time.
 *
 * Names are never HTML: a heading reading `<script>` comes back as that text
 * and is escaped by whatever renders it, like any other untrusted string.
 */

/** The most entries an outline carries; past this it is reported truncated. */
export const OUTLINE_LIMIT = 1000;

/**
 * Markdown past this many characters is parsed only to the last line boundary
 * before it. Block-only parsing runs at roughly 8MB per 200ms, so this bounds
 * the walk to a fraction of a frame budget on a generated file.
 */
export const MAX_MARKDOWN_BYTES = 4 * 1024 * 1024;

/** How much of a line the code patterns look at. A minified bundle is one line. */
const MAX_LINE_SCAN = 400;

/** How much of a heading's source is worth parsing for its title. */
const MAX_NAME_SOURCE = 512;

/** How long a row's text may be before it is cut. */
const MAX_NAME_LENGTH = 200;

/** What a heading with no text at all is called, rather than drawing a blank row. */
const EMPTY_HEADING = '(empty heading)';

/** The outline of one buffer. */
export interface OutlineResult {
  /**
   * Gritty has outline rules for this language. `false` means *the tool stops
   * here*, not *the file is empty*, and the two must read differently.
   */
  readonly supported: boolean;
  /** The scan hit a bound; entries after it are missing. */
  readonly truncated: boolean;
  /** In document order, `line` 1-based. */
  readonly entries: readonly OutlineEntry[];
}

/** The rule sets that exist. Anything else has no outline. */
type OutlineDialect = 'markdown' | 'rust' | 'script';

/**
 * `document.languageId` in the VS Code vocabulary the core produces
 * (`crates/workspace/src/detect.rs`); `.tsx`/`.jsx` arrive as `typescript` and
 * `javascript`, and the `*react` spellings are accepted for the same files.
 */
function dialectOf(languageId: string): OutlineDialect | null {
  switch (languageId.trim().toLowerCase()) {
    case 'markdown':
      return 'markdown';
    case 'rust':
      return 'rust';
    case 'typescript':
    case 'typescriptreact':
    case 'javascript':
    case 'javascriptreact':
      return 'script';
    default:
      return null;
  }
}

/**
 * Whether Gritty can outline this language at all — the question behind "no
 * outline for this language", answerable without a buffer.
 */
export function hasOutlineRules(languageId: string): boolean {
  return dialectOf(languageId) !== null;
}

/** The outline of `text`, read as `languageId`. Pure: no React, no core, no DOM. */
export function outlineOf(languageId: string, text: string): OutlineResult {
  switch (dialectOf(languageId)) {
    case 'markdown':
      return markdownOutline(text);
    case 'rust':
      return scanLines(text, rustItem);
    case 'script':
      return scanLines(text, scriptItem);
    default:
      return { supported: false, truncated: false, entries: [] };
  }
}

// --- Markdown -------------------------------------------------------------

/**
 * markdown-it with the inline chain off: the block parser is what knows that a
 * `#` inside a fence is not a heading, that `Title\n===` is one, and where each
 * heading starts. Both instances are built once — a markdown-it instance holds
 * no state between parses, which is why the preview reuses one too.
 */
let blockParser: ReturnType<typeof createMarkdownInstance> | null = null;
let titleParser: ReturnType<typeof createMarkdownInstance> | null = null;

function blocks(): ReturnType<typeof createMarkdownInstance> {
  if (blockParser === null) {
    const md = createMarkdownInstance();
    md.disable(['inline', 'linkify', 'replacements', 'smartquotes', 'text_join'], true);
    blockParser = md;
  }
  return blockParser;
}

function titles(): ReturnType<typeof createMarkdownInstance> {
  titleParser ??= createMarkdownInstance();
  return titleParser;
}

function markdownOutline(text: string): OutlineResult {
  const limit = text.length > MAX_MARKDOWN_BYTES ? lineBoundaryBefore(text) : text.length;
  let truncated = limit < text.length;
  let tokens: ReturnType<ReturnType<typeof createMarkdownInstance>['parse']>;
  try {
    tokens = blocks().parse(text.slice(0, limit), {});
  } catch {
    // The parser is bounded and should not throw; an outline is never worth
    // failing a keystroke over, so a broken parse is an empty outline.
    return { supported: true, truncated, entries: [] };
  }
  const entries: OutlineEntry[] = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (token?.type !== 'heading_open') continue;
    if (entries.length >= OUTLINE_LIMIT) {
      truncated = true;
      break;
    }
    const inline = tokens[index + 1];
    entries.push({
      name: headingTitle(inline?.type === 'inline' ? inline.content : ''),
      kind: 'heading',
      line: (token.map?.[0] ?? 0) + 1,
      depth: Number(token.tag.slice(1)) || 1,
    });
  }
  return { supported: true, truncated, entries };
}

function lineBoundaryBefore(text: string): number {
  const newline = text.lastIndexOf('\n', MAX_MARKDOWN_BYTES);
  return newline > 0 ? newline : MAX_MARKDOWN_BYTES;
}

/**
 * A heading's plain text: the same `inlineText` walk the preview uses for
 * titles, so `## The **hard** part` is one string rather than its markup. The
 * source is capped first — this is the one inline parse in the module, and the
 * inline chain is the part that misbehaves on adversarial input.
 */
function headingTitle(source: string): string {
  const capped = source.slice(0, MAX_NAME_SOURCE);
  let text = '';
  try {
    text = inlineText(titles().parseInline(capped, {})[0]?.children ?? null);
  } catch {
    text = capped;
  }
  const name = clampName(text !== '' ? text : capped);
  return name !== '' ? name : EMPTY_HEADING;
}

// --- Code -----------------------------------------------------------------

/** What one line declares, if anything. */
interface LineItem {
  readonly name: string;
  readonly kind: OutlineKind;
}

/**
 * One pass over the buffer, one bounded pattern per line. Lines are walked by
 * index rather than `split('\n')` so a 16MB buffer does not also become an
 * array of 200,000 strings.
 */
function scanLines(text: string, item: (line: string) => LineItem | null): OutlineResult {
  const entries: OutlineEntry[] = [];
  let truncated = false;
  let line = 1;
  let start = 0;
  while (start <= text.length) {
    let end = text.indexOf('\n', start);
    if (end === -1) end = text.length;
    const found = item(text.slice(start, Math.min(end, start + MAX_LINE_SCAN)).trim());
    if (found !== null) {
      if (entries.length >= OUTLINE_LIMIT) {
        truncated = true;
        break;
      }
      entries.push({ name: found.name, kind: found.kind, line, depth: 0 });
    }
    start = end + 1;
    line += 1;
  }
  return { supported: true, truncated, entries };
}

/**
 * A Rust item. The optional prefixes (`pub`, `pub(crate)`, `async`, `unsafe`,
 * `extern "C"`, `const`, `default`) are each anchored and each end in required
 * whitespace, so `pubfn` is not a function. `impl` is matched on its own
 * because its subject is a type expression, not an identifier.
 */
const RUST_ITEM =
  /^(?:pub(?:\([^)]{0,32}\))?[\t ]+)?(?:default[\t ]+)?(?:const[\t ]+)?(?:async[\t ]+)?(?:unsafe[\t ]+)?(?:extern[\t ]+"[^"]{0,32}"[\t ]+)?(?:(fn|struct|enum|trait|mod)[\t ]+([A-Za-z_][A-Za-z0-9_]{0,127})|(impl)\b)/;

const RUST_KINDS: Record<string, OutlineKind> = {
  fn: 'function',
  struct: 'struct',
  enum: 'enum',
  trait: 'trait',
  mod: 'module',
};

function rustItem(line: string): LineItem | null {
  if (isComment(line)) return null;
  const match = RUST_ITEM.exec(line);
  if (match === null) return null;
  if (match[3] === 'impl') return { name: implSubject(line), kind: 'impl' };
  const keyword = match[1];
  const kind = keyword === undefined ? undefined : RUST_KINDS[keyword];
  const name = match[2];
  if (kind === undefined || name === undefined) return null;
  return { name, kind };
}

/**
 * `impl<T> Display for Wrapper<T> where T: Debug {` reads best as
 * `impl<T> Display for Wrapper<T>`: the head, without the body or the clause.
 */
function implSubject(line: string): string {
  const start = line.indexOf('impl');
  let head = line.slice(start);
  const brace = head.indexOf('{');
  if (brace !== -1) head = head.slice(0, brace);
  const where = head.indexOf(' where');
  if (where !== -1) head = head.slice(0, where);
  return clampName(head.replace(/;$/, ''));
}

/**
 * A TypeScript/JavaScript declaration. `const`/`let` are only an entry when the
 * same line goes on to bind an arrow — the rest is a value, not a definition —
 * which is checked with a plain substring search rather than a pattern that
 * would have to skip a type annotation.
 */
const SCRIPT_DECL =
  /^(?:export[\t ]+)?(?:default[\t ]+)?(?:declare[\t ]+)?(?:abstract[\t ]+)?(?:async[\t ]+)?(function|class|interface|type|const|let)\b[\t *]*([A-Za-z_$][A-Za-z0-9_$]{0,127})/;

/** `export default …` with nothing named after it still deserves a row. */
const SCRIPT_DEFAULT = /^export[\t ]+default\b[\t ]*(function|class)?/;

const SCRIPT_KINDS: Record<string, OutlineKind> = {
  function: 'function',
  class: 'class',
  interface: 'interface',
  type: 'type',
};

/** Words that follow a keyword where a name would go but never name anything. */
const SCRIPT_NON_NAMES = new Set(['extends', 'implements']);

function scriptItem(line: string): LineItem | null {
  if (isComment(line)) return null;
  const match = SCRIPT_DECL.exec(line);
  const keyword = match?.[1];
  const captured = match?.[2];
  const name = captured !== undefined && SCRIPT_NON_NAMES.has(captured) ? undefined : captured;
  if (keyword !== undefined && name !== undefined) {
    const kind = SCRIPT_KINDS[keyword];
    if (kind !== undefined) return { name, kind };
    // `const`/`let`: a function only if this line binds an arrow to it.
    if (line.slice(match?.[0].length ?? 0).includes('=>')) return { name, kind: 'function' };
    return null;
  }
  const fallback = SCRIPT_DEFAULT.exec(line);
  if (fallback === null) return null;
  const declared = fallback[1];
  const kind: OutlineKind =
    declared === 'function' ? 'function' : declared === 'class' ? 'class' : 'constant';
  return { name: 'export default', kind };
}

/**
 * A line whose first characters are a comment marker. Cheap and deliberately
 * shallow: it keeps `/// fn example()` in a doc comment out of the outline. A
 * keyword inside a block comment or a string literal still gets through, which
 * is the honest cost of pattern matching without a lexer.
 */
function isComment(line: string): boolean {
  return line.startsWith('//') || line.startsWith('/*') || line.startsWith('*');
}

// --- Shared ---------------------------------------------------------------

function clampName(text: string): string {
  const collapsed = text.replace(/\s+/g, ' ').trim();
  return collapsed.length > MAX_NAME_LENGTH ? `${collapsed.slice(0, MAX_NAME_LENGTH)}…` : collapsed;
}
