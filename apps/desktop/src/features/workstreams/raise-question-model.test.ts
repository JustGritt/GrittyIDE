import { describe, expect, it } from 'vitest';
import { canRaise, MAX_OPTIONS, optionsFrom, slugify } from './raise-question-model';

describe('slugify', () => {
  it('derives a key a person could recognise', () => {
    expect(slugify('Bound by bytes', 0)).toBe('bound-by-bytes');
    expect(slugify('Yes, do it!', 0)).toBe('yes-do-it');
  });

  it('gives a label that reduces to nothing a positional key', () => {
    // An emoji or bare punctuation is not an error. The option still has to
    // work, and lecturing someone about character classes over a label they
    // can read perfectly well would be the form being precious.
    expect(slugify('🚀', 0)).toBe('option-1');
    expect(slugify('…', 3)).toBe('option-4');
  });

  it('stays inside the core key bound', () => {
    // The core refuses a key over 64 characters; a long label is ordinary, so
    // this truncates rather than letting the request bounce.
    expect(slugify('a'.repeat(200), 0)).toHaveLength(64);
  });
});

describe('optionsFrom', () => {
  it('pairs each label with a key', () => {
    expect(optionsFrom(['Bound by bytes', 'Bound by entry count'])).toEqual([
      { key: 'bound-by-bytes', label: 'Bound by bytes' },
      { key: 'bound-by-entry-count', label: 'Bound by entry count' },
    ]);
  });

  it('drops a blank row instead of refusing the whole question', () => {
    // A blank row is someone who clicked "add" and changed their mind.
    expect(optionsFrom(['Keep it', '', '   ']).map((option) => option.label)).toEqual(['Keep it']);
  });

  it('trims what was typed, the way the core would', () => {
    expect(optionsFrom(['  Keep it  '])).toEqual([{ key: 'keep-it', label: 'Keep it' }]);
  });

  it('makes two labels that slug the same distinct', () => {
    // The core rejects a duplicate key, and these two are an honest thing to
    // type — so the second gets a suffix rather than a refusal the user has to
    // decode.
    const options = optionsFrom(['Yes, do it', 'Yes! Do it.']);
    expect(options.map((option) => option.key)).toEqual(['yes-do-it', 'yes-do-it-2']);
    expect(new Set(options.map((option) => option.key)).size).toBe(2);
  });

  it('keeps a de-duplicated key inside the bound as well', () => {
    const long = 'a'.repeat(200);
    const keys = optionsFrom([long, `${long}!`]).map((option) => option.key);
    expect(new Set(keys).size).toBe(2);
    for (const key of keys) expect(key.length).toBeLessThanOrEqual(64);
  });

  it('is empty for a question with no options at all', () => {
    // A free-form question is a first-class kind of decision, not a degenerate
    // one: `DecisionCard` already answers one with a text field.
    expect(optionsFrom([])).toEqual([]);
  });

  it('has a bound the form can stop at', () => {
    expect(MAX_OPTIONS).toBe(12);
  });
});

describe('canRaise', () => {
  it('needs a question, and only a question', () => {
    expect(canRaise('')).toBe(false);
    expect(canRaise('   ')).toBe(false);
    expect(canRaise('Should the ring be bounded by bytes?')).toBe(true);
  });
});
