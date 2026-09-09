import { describe, expect, it } from 'vitest';
import { parseQuery, QUICK_OPEN_PREFIX } from './quick-open-parser';
import type { QuickOpenMode } from './quick-open-types';

/**
 * The prefix rule (T130-U §1), which is the whole of what the parser owns.
 *
 * Pure, so every prefix, the escape and the whitespace rule are asserted here
 * and nothing downstream ever looks at the raw string again.
 */

describe('the mode comes from the first character, and only the first', () => {
  it('reads an unprefixed query as a file search', () => {
    expect(parseQuery('editor-pane')).toEqual({ mode: 'files', term: 'editor-pane' });
  });

  it('reads an empty query as the files mode with nothing to look for', () => {
    // Not a fourth "no query" mode: the field is empty, and files is what the
    // bar is for when nobody has said otherwise.
    expect(parseQuery('')).toEqual({ mode: 'files', term: '' });
  });

  it('claims >, : and @ for commands, line and symbols', () => {
    expect(parseQuery('>save')).toEqual({ mode: 'commands', term: 'save' });
    expect(parseQuery(':120')).toEqual({ mode: 'line', term: '120' });
    expect(parseQuery('@render')).toEqual({ mode: 'symbols', term: 'render' });
  });

  it('reads a lone prefix as its mode with an empty term', () => {
    // What makes ">" alone list every command rather than nothing: the mode is
    // chosen the moment the character is typed, not once a term follows it.
    expect(parseQuery('>')).toEqual({ mode: 'commands', term: '' });
    expect(parseQuery(':')).toEqual({ mode: 'line', term: '' });
    expect(parseQuery('@')).toEqual({ mode: 'symbols', term: '' });
  });

  it('leaves a prefix character in the middle of a query alone', () => {
    // Nothing rescans past the first character, so a path containing one of
    // them is an ordinary file search.
    expect(parseQuery('src/a>b.ts')).toEqual({ mode: 'files', term: 'src/a>b.ts' });
    expect(parseQuery('a@b')).toEqual({ mode: 'files', term: 'a@b' });
  });

  it('covers every mode in the shared union, so a new mode must decide', () => {
    // The table is a Record over QuickOpenMode: adding a mode to the shared
    // type fails to compile here until it has said whether it has a prefix.
    const modes: readonly QuickOpenMode[] = ['files', 'commands', 'line', 'symbols'];
    expect(Object.keys(QUICK_OPEN_PREFIX).sort()).toEqual([...modes].sort());
    // Exactly one mode has no prefix, which is why it is the default.
    expect(Object.values(QUICK_OPEN_PREFIX).filter((prefix) => prefix === '')).toHaveLength(1);
  });
});

describe('a doubled prefix escapes it, which is how a path starting with one is reachable', () => {
  it('falls back to the files mode and keeps one literal prefix character', () => {
    // The alternative reading — ">>foo" as the command ">foo" — would leave no
    // way at all to search for a file whose name starts with ">", which is the
    // only reason the escape exists.
    expect(parseQuery('>>foo')).toEqual({ mode: 'files', term: '>foo' });
    expect(parseQuery('::foo')).toEqual({ mode: 'files', term: ':foo' });
    expect(parseQuery('@@foo')).toEqual({ mode: 'files', term: '@foo' });
  });

  it('escapes a doubled prefix with nothing after it', () => {
    expect(parseQuery('>>')).toEqual({ mode: 'files', term: '>' });
  });

  it('consumes only the first doubling, so a longer run stays literal', () => {
    // Otherwise a run of prefix characters would collapse one per keystroke
    // and the mode would flicker as the user typed.
    expect(parseQuery('>>>x')).toEqual({ mode: 'files', term: '>>x' });
    expect(parseQuery('>>>>x')).toEqual({ mode: 'files', term: '>>>x' });
  });

  it('needs the same character twice, not any two prefixes', () => {
    // ">:" is the command search ":", not an escape: two different prefixes
    // are a prefix followed by a term.
    expect(parseQuery('>:')).toEqual({ mode: 'commands', term: ':' });
    expect(parseQuery('@>')).toEqual({ mode: 'symbols', term: '>' });
  });
});

describe('whitespace', () => {
  it('ignores leading whitespace before the prefix', () => {
    expect(parseQuery('   >save')).toEqual({ mode: 'commands', term: 'save' });
    expect(parseQuery('\t editor')).toEqual({ mode: 'files', term: 'editor' });
  });

  it('ignores whitespace between the prefix and the term', () => {
    expect(parseQuery('>   save')).toEqual({ mode: 'commands', term: 'save' });
    expect(parseQuery(':  120')).toEqual({ mode: 'line', term: '120' });
  });

  it('keeps trailing whitespace, because a file name may end in one', () => {
    // Deliberately asymmetric with the leading rule: trimming both ends would
    // make a path ending in a space the one thing quick open cannot reach,
    // and this repository ships fixtures/weird-files for exactly that class of
    // name. The line mode trims its own term, where a space is never part of
    // the answer.
    expect(parseQuery('  odd name ')).toEqual({ mode: 'files', term: 'odd name ' });
  });

  it('keeps whitespace inside an escaped term verbatim', () => {
    // Past the escape, the first character is a literal, so what follows is
    // the name and not padding.
    expect(parseQuery('>> odd')).toEqual({ mode: 'files', term: '> odd' });
  });

  it('reads whitespace alone as an empty file search', () => {
    expect(parseQuery('   ')).toEqual({ mode: 'files', term: '' });
  });
});
