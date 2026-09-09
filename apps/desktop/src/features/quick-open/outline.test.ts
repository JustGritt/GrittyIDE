import { describe, expect, it } from 'vitest';
import { hasOutlineRules, MAX_MARKDOWN_BYTES, OUTLINE_LIMIT, outlineOf } from './outline';
import type { OutlineEntry } from './quick-open-types';

/**
 * What the `@` mode can read out of a buffer (T130-S).
 *
 * Pure, so every rule is asserted on a fixture string rather than on a running
 * editor. Three things are worth more than the rest and are tested hardest:
 * the fence case (the one a line scan gets wrong and a user sees immediately),
 * the difference between *no rules for this language* and *no entries in this
 * file*, and the bounds — an outline that can hang the field on a generated or
 * hostile file is worse than no outline.
 */

function rows(entries: readonly OutlineEntry[]): Array<[string, string, number, number]> {
  return entries.map((entry) => [entry.name, entry.kind, entry.line, entry.depth]);
}

function names(entries: readonly OutlineEntry[]): string[] {
  return entries.map((entry) => entry.name);
}

describe('markdown headings', () => {
  it('lists ATX headings in order, with the level as depth and a 1-based line', () => {
    // Line numbers are counted here by hand so the fixture, not the
    // implementation, decides what "line 4" means.
    const text = [
      '# Title', // 1
      '', // 2
      'Some prose.', // 3
      '', // 4
      '## Section', // 5
      '', // 6
      '### Detail', // 7
      '', // 8
      '## Another', // 9
    ].join('\n');
    const result = outlineOf('markdown', text);
    expect(result.supported).toBe(true);
    expect(result.truncated).toBe(false);
    expect(rows(result.entries)).toEqual([
      ['Title', 'heading', 1, 1],
      ['Section', 'heading', 5, 2],
      ['Detail', 'heading', 7, 3],
      ['Another', 'heading', 9, 2],
    ]);
  });

  it('reads setext headings, which have no marker on their own line', () => {
    const text = ['Title', '=====', '', 'Section', '-------', '', 'text'].join('\n');
    expect(rows(outlineOf('markdown', text).entries)).toEqual([
      ['Title', 'heading', 1, 1],
      ['Section', 'heading', 4, 2],
    ]);
  });

  it('does not read a # inside a fenced block as a heading', () => {
    // The case a naive line scan gets wrong. Both fence markers, and the
    // indented code block, are the block parser's business — which is why this
    // module reuses it instead of scanning lines for `#`.
    const text = [
      '# Real', // 1
      '', // 2
      '```sh', // 3
      '# not a heading, a shell comment', // 4
      '```', // 5
      '', // 6
      '~~~', // 7
      '## also not a heading', // 8
      '~~~', // 9
      '', // 10
      '    # an indented code block, also not a heading', // 11
      '', // 12
      '## Real again', // 13
    ].join('\n');
    expect(rows(outlineOf('markdown', text).entries)).toEqual([
      ['Real', 'heading', 1, 1],
      ['Real again', 'heading', 13, 2],
    ]);
  });

  it('shows a heading as its text, not as its markup', () => {
    const text = '## The `hard` part is **naming** it\n';
    expect(names(outlineOf('markdown', text).entries)).toEqual(['The hard part is naming it']);
  });

  it('keeps a heading that contains markup as plain text', () => {
    // Repository content is untrusted (AGENTS.md rule 13). The tag is the
    // heading's text and comes back as text; nothing here builds HTML.
    const text = '# <script>alert(1)</script>\n\n## Fine\n';
    const entries = outlineOf('markdown', text).entries;
    expect(names(entries)).toEqual(['<script>alert(1)</script>', 'Fine']);
    expect(entries[0]?.name).not.toContain('&lt;');
  });

  it('names a heading with no text rather than drawing a blank row', () => {
    expect(names(outlineOf('markdown', '#\n').entries)).toEqual(['(empty heading)']);
  });

  it('reads a heading through CRLF line endings', () => {
    const result = outlineOf('markdown', '# One\r\n\r\n## Two\r\n');
    expect(rows(result.entries)).toEqual([
      ['One', 'heading', 1, 1],
      ['Two', 'heading', 3, 2],
    ]);
  });
});

describe('rust items', () => {
  const text = [
    '//! A module.', // 1
    'use std::fmt;', // 2
    '', // 3
    'pub struct Wrapper<T>(T);', // 4
    '', // 5
    '#[derive(Debug)]', // 6
    'enum State {', // 7
    '    Idle,', // 8
    '}', // 9
    '', // 10
    'pub(crate) trait Render {', // 11
    '    fn render(&self) -> String;', // 12
    '}', // 13
    '', // 14
    'impl<T: fmt::Debug> Render for Wrapper<T> where T: Clone {', // 15
    '    pub async fn render(&self) -> String {', // 16
    '        let apply: fn(i32) -> i32 = |x| x;', // 17
    '        String::new()', // 18
    '    }', // 19
    '}', // 20
    '', // 21
    'mod tests {}', // 22
    '', // 23
    '/// fn documented_but_not_real()', // 24
    'unsafe fn danger() {}', // 25
  ].join('\n');

  it('lists fn, struct, enum, trait, impl and mod, and nothing else', () => {
    expect(rows(outlineOf('rust', text).entries)).toEqual([
      ['Wrapper', 'struct', 4, 0],
      ['State', 'enum', 7, 0],
      ['Render', 'trait', 11, 0],
      ['render', 'function', 12, 0],
      ['impl<T: fmt::Debug> Render for Wrapper<T>', 'impl', 15, 0],
      ['render', 'function', 16, 0],
      ['tests', 'module', 22, 0],
      ['danger', 'function', 25, 0],
    ]);
  });

  it('ignores a keyword that is only part of an identifier or a type', () => {
    // `fn` inside a type annotation (line 17 above) is already excluded; these
    // are the shapes that would fall out of a looser pattern.
    const result = outlineOf(
      'rust',
      ['pubfn nope() {}', 'implements(x);', 'modest = 1;'].join('\n'),
    );
    expect(result.entries).toEqual([]);
    expect(result.supported).toBe(true);
  });
});

describe('typescript and javascript declarations', () => {
  const text = [
    "import { thing } from './thing';", // 1
    '', // 2
    'export interface Options {', // 3
    '  readonly kind: string;', // 4
    '}', // 5
    '', // 6
    'export type Mode = "on" | "off";', // 7
    '', // 8
    'export function build(options: Options) {}', // 9
    '', // 10
    'async function* stream() {}', // 11
    '', // 12
    'export class Panel {}', // 13
    '', // 14
    'export const render = (options: Options) => build(options);', // 15
    '', // 16
    'const later = async () => {};', // 17
    '', // 18
    'let handler = (event: Event) => void event;', // 19
    '', // 20
    'const LIMIT = 100;', // 21
    '', // 22
    '// const commented = () => {};', // 23
    '', // 24
    'export default {', // 25
    '  value: 1,', // 26
    '};', // 27
  ].join('\n');

  it('lists functions, classes, interfaces, types and arrow-bound bindings', () => {
    expect(rows(outlineOf('typescript', text).entries)).toEqual([
      ['Options', 'interface', 3, 0],
      ['Mode', 'type', 7, 0],
      ['build', 'function', 9, 0],
      ['stream', 'function', 11, 0],
      ['Panel', 'class', 13, 0],
      ['render', 'function', 15, 0],
      ['later', 'function', 17, 0],
      ['handler', 'function', 19, 0],
      ['export default', 'constant', 25, 0],
    ]);
  });

  it('leaves a const that is not a function out of the outline', () => {
    // `const LIMIT = 100` (line 21) is a value, not a definition to jump to.
    expect(names(outlineOf('typescript', text).entries)).not.toContain('LIMIT');
  });

  it('names what an anonymous default export declares', () => {
    expect(rows(outlineOf('javascript', 'export default function () {}').entries)).toEqual([
      ['export default', 'function', 1, 0],
    ]);
    expect(rows(outlineOf('javascript', 'export default class extends Base {}').entries)).toEqual([
      ['export default', 'class', 1, 0],
    ]);
    expect(rows(outlineOf('typescriptreact', 'export default class Screen {}').entries)).toEqual([
      ['Screen', 'class', 1, 0],
    ]);
  });
});

describe('languages with no rules', () => {
  it('says so, and says it differently from a file with no entries', () => {
    // The distinction the row model turns into two sentences: one is a limit
    // of the tool, the other a fact about the file.
    const unsupported = outlineOf('python', 'def main():\n    pass\n');
    expect(unsupported).toEqual({ supported: false, truncated: false, entries: [] });

    const empty = outlineOf('markdown', 'Prose with no headings at all.\n');
    expect(empty).toEqual({ supported: true, truncated: false, entries: [] });

    expect(unsupported.supported).not.toBe(empty.supported);
  });

  it('answers the same question without a buffer', () => {
    expect(['markdown', 'rust', 'typescript', 'javascript'].map(hasOutlineRules)).toEqual([
      true,
      true,
      true,
      true,
    ]);
    expect(['python', 'json', 'plaintext', '', 'toml'].map(hasOutlineRules)).toEqual([
      false,
      false,
      false,
      false,
      false,
    ]);
    for (const language of ['markdown', 'rust', 'typescript', 'python', '']) {
      expect(outlineOf(language, '# x\n').supported).toBe(hasOutlineRules(language));
    }
  });
});

describe('bounds', () => {
  it('stops at the entry limit and says the outline is truncated', () => {
    const many = Array.from({ length: OUTLINE_LIMIT + 500 }, (_, index) => `# H${index}`).join(
      '\n\n',
    );
    const result = outlineOf('markdown', many);
    expect(result.entries).toHaveLength(OUTLINE_LIMIT);
    expect(result.truncated).toBe(true);
    expect(result.entries[0]?.name).toBe('H0');
  });

  it('does not call an outline of exactly the limit truncated', () => {
    const exact = Array.from({ length: OUTLINE_LIMIT }, (_, index) => `fn f${index}() {}`).join(
      '\n',
    );
    const result = outlineOf('rust', exact);
    expect(result.entries).toHaveLength(OUTLINE_LIMIT);
    expect(result.truncated).toBe(false);
  });

  it('bounds a code file the same way', () => {
    const many = Array.from(
      { length: OUTLINE_LIMIT + 200 },
      (_, index) => `export function f${index}() {}`,
    ).join('\n');
    const result = outlineOf('typescript', many);
    expect(result.entries).toHaveLength(OUTLINE_LIMIT);
    expect(result.truncated).toBe(true);
  });

  it('parses only the first megabytes of a very long markdown file', () => {
    const filler = `${'lorem ipsum dolor sit amet '.repeat(4)}\n`;
    const prose = filler.repeat(Math.ceil(MAX_MARKDOWN_BYTES / filler.length) + 10);
    const result = outlineOf('markdown', `# Early\n\n${prose}\n# Late\n`);
    expect(names(result.entries)).toEqual(['Early']);
    expect(result.truncated).toBe(true);
  });

  it('stays fast on input written to make a parser backtrack', () => {
    // A megabyte of `![` costs markdown-it's inline parser seconds and its
    // block parser milliseconds; the generous budget is here to fail if the
    // inline chain is ever switched back on, not to measure a machine.
    const hostile = `# Heading\n\n${'!['.repeat(500_000)}\n`;
    const started = Date.now();
    const result = outlineOf('markdown', hostile);
    expect(Date.now() - started).toBeLessThan(2_000);
    expect(names(result.entries)).toEqual(['Heading']);
  });

  it('stays fast on a single enormous line of code', () => {
    const minified = `const x=${'('.repeat(200_000)}1${')'.repeat(200_000)};`;
    const started = Date.now();
    const result = outlineOf('javascript', `${minified}\nfunction after() {}`);
    expect(Date.now() - started).toBeLessThan(2_000);
    expect(rows(result.entries)).toEqual([['after', 'function', 2, 0]]);
  });

  it('cuts a name long enough to break a row', () => {
    const long = `# ${'name '.repeat(200)}`;
    const [entry] = outlineOf('markdown', long).entries;
    expect(entry?.name.length).toBeLessThanOrEqual(201);
    expect(entry?.name.endsWith('…')).toBe(true);
  });
});

describe('empty and degenerate buffers', () => {
  it('returns an empty outline for an empty buffer of a supported language', () => {
    for (const language of ['markdown', 'rust', 'typescript']) {
      expect(outlineOf(language, '')).toEqual({
        supported: true,
        truncated: false,
        entries: [],
      });
    }
  });

  it('accepts a language id in any case, with stray whitespace', () => {
    expect(outlineOf('  Markdown ', '# Hi\n').entries).toHaveLength(1);
    expect(hasOutlineRules('RUST')).toBe(true);
  });
});
