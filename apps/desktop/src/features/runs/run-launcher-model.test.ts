import { describe, expect, it } from 'vitest';
import { canLaunch, DERIVED_TITLE_LENGTH, MAX_PROMPT, titleFromPrompt } from './run-launcher-model';

describe('titleFromPrompt', () => {
  it('takes the first line that says anything', () => {
    expect(titleFromPrompt('\n   \n  Fix the flaky watcher test\nthen run the gate')).toBe(
      'Fix the flaky watcher test',
    );
  });

  it('collapses whitespace inside the line', () => {
    expect(titleFromPrompt('rename\t the   ring   buffer')).toBe('rename the ring buffer');
  });

  it('handles CRLF line endings', () => {
    expect(titleFromPrompt('first\r\nsecond')).toBe('first');
  });

  it('keeps a title exactly at the limit untouched', () => {
    const exact = 'a'.repeat(DERIVED_TITLE_LENGTH);
    expect(titleFromPrompt(exact)).toBe(exact);
  });

  it('clips a long line with an ellipsis, within the limit', () => {
    const title = titleFromPrompt('word '.repeat(40));
    expect([...title].length).toBeLessThanOrEqual(DERIVED_TITLE_LENGTH);
    expect(title.endsWith('…')).toBe(true);
    // The space before the cut is trimmed, so the ellipsis sits on a word.
    expect(title.endsWith(' …')).toBe(false);
  });

  it('never splits a character outside the basic plane', () => {
    const title = titleFromPrompt('😀'.repeat(DERIVED_TITLE_LENGTH + 5));
    expect([...title]).toHaveLength(DERIVED_TITLE_LENGTH);
    expect(title).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])/);
  });

  it('is empty for a prompt of only whitespace', () => {
    expect(titleFromPrompt(' \n\t\n ')).toBe('');
  });
});

describe('canLaunch', () => {
  it('refuses a blank prompt', () => {
    expect(canLaunch('')).toBe(false);
    expect(canLaunch('  \n ')).toBe(false);
  });

  it('takes a prompt up to the core limit, counted in characters', () => {
    expect(canLaunch('x'.repeat(MAX_PROMPT))).toBe(true);
    expect(canLaunch('x'.repeat(MAX_PROMPT + 1))).toBe(false);
    // Two UTF-16 units each, one character each: still within the limit.
    expect(canLaunch('😀'.repeat(MAX_PROMPT))).toBe(true);
  });
});
