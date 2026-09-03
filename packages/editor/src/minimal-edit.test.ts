import { describe, expect, it } from 'vitest';
import { applyMinimalEdit, computeMinimalEdit } from './minimal-edit';

function roundTrip(oldText: string, newText: string) {
  const edit = computeMinimalEdit(oldText, newText);
  if (edit === null) {
    expect(oldText).toBe(newText);
    return edit;
  }
  expect(applyMinimalEdit(oldText, edit)).toBe(newText);
  return edit;
}

describe('computeMinimalEdit', () => {
  it('returns null for identical text', () => {
    expect(computeMinimalEdit('same', 'same')).toBeNull();
    expect(computeMinimalEdit('', '')).toBeNull();
  });

  it('trims the common prefix and suffix', () => {
    expect(roundTrip('line one\nline two\nline three\n', 'line one\nline 2\nline three\n')).toEqual(
      {
        start: 14,
        end: 17,
        text: '2',
      },
    );
  });

  it('handles pure insertions and deletions at every position', () => {
    expect(roundTrip('abc', 'Xabc')).toEqual({ start: 0, end: 0, text: 'X' });
    expect(roundTrip('abc', 'abcX')).toEqual({ start: 3, end: 3, text: 'X' });
    expect(roundTrip('abc', 'abXc')).toEqual({ start: 2, end: 2, text: 'X' });
    expect(roundTrip('Xabc', 'abc')).toEqual({ start: 0, end: 1, text: '' });
    expect(roundTrip('abcX', 'abc')).toEqual({ start: 3, end: 4, text: '' });
    expect(roundTrip('', 'text')).toEqual({ start: 0, end: 0, text: 'text' });
    expect(roundTrip('text', '')).toEqual({ start: 0, end: 4, text: '' });
  });

  it('does not let the prefix and suffix overlap on repeated text', () => {
    expect(roundTrip('aaa', 'aaaa')).toEqual({ start: 3, end: 3, text: 'a' });
    expect(roundTrip('aaaa', 'aa')).toEqual({ start: 2, end: 4, text: '' });
    expect(roundTrip('abab', 'ab')).toEqual({ start: 2, end: 4, text: '' });
  });

  it('never splits a surrogate pair', () => {
    // 😀 = D83D DE00, 😁 = D83D DE01: only the low surrogate differs.
    const grin = roundTrip('😀x', '😁x');
    expect(grin).toEqual({ start: 0, end: 2, text: '😁' });
    const tail = roundTrip('x😀', 'x😁');
    expect(tail).toEqual({ start: 1, end: 3, text: '😁' });
    roundTrip('a😀b', 'a😀😀b');
    roundTrip('😀😀', '😀');
  });

  it('round-trips a reload of a CRLF document', () => {
    const before = 'one\r\ntwo\r\nthree\r\n';
    const after = 'one\r\ntwo changed\r\nthree\r\n';
    expect(roundTrip(before, after)).toEqual({ start: 8, end: 8, text: ' changed' });
  });
});
