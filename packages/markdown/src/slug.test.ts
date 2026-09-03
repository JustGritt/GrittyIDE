import { describe, expect, it } from 'vitest';
import { anchorId, createSlugger, slugify } from './slug';

describe('slugs', () => {
  it('follows GitHub rules', () => {
    expect(slugify('Hello, World!')).toBe('hello-world');
    expect(slugify('  Ünïcödé — heading, with: punctuation! ')).toBe(
      'ünïcödé--heading-with-punctuation',
    );
    expect(slugify('a  b')).toBe('a--b');
    expect(slugify('snake_case stays')).toBe('snake_case-stays');
    expect(slugify('C# and F#')).toBe('c-and-f');
  });

  it('prefixes and de-duplicates', () => {
    const slug = createSlugger();
    expect(slug('Duplicate')).toBe('md-duplicate');
    expect(slug('Duplicate')).toBe('md-duplicate-1');
    expect(slug('duplicate')).toBe('md-duplicate-2');
    expect(slug('Other')).toBe('md-other');
    expect(anchorId('Links Fixture')).toBe('md-links-fixture');
  });
});
