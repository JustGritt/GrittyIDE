import { describe, expect, it } from 'vitest';
import { type ChordEvent, terminalChordIndex } from './terminal-chords';

function chord(overrides: Partial<ChordEvent> = {}): ChordEvent {
  return {
    altKey: true,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    code: 'Digit1',
    ...overrides,
  };
}

describe('terminalChordIndex', () => {
  it('maps ⌥1…⌥9 onto the first nine sessions', () => {
    for (let digit = 1; digit <= 9; digit += 1) {
      expect(terminalChordIndex(chord({ code: `Digit${digit}` }))).toBe(digit - 1);
    }
  });

  it('matches on the physical key, not the character Option composed', () => {
    // ⌥1 arrives as `¡` on a US macOS layout; the code is what identifies it.
    expect(terminalChordIndex({ ...chord(), code: 'Digit1' })).toBe(0);
  });

  it('ignores ⌥0 — there is no zeroth session', () => {
    expect(terminalChordIndex(chord({ code: 'Digit0' }))).toBeNull();
  });

  it('ignores a digit without Option', () => {
    expect(terminalChordIndex(chord({ altKey: false }))).toBeNull();
  });

  it('ignores every other modifier combination', () => {
    expect(terminalChordIndex(chord({ metaKey: true }))).toBeNull();
    expect(terminalChordIndex(chord({ ctrlKey: true }))).toBeNull();
    expect(terminalChordIndex(chord({ shiftKey: true }))).toBeNull();
  });

  it('ignores keys that are not digits', () => {
    expect(terminalChordIndex(chord({ code: 'KeyA' }))).toBeNull();
    expect(terminalChordIndex(chord({ code: 'Numpad1' }))).toBeNull();
    expect(terminalChordIndex(chord({ code: '' }))).toBeNull();
  });
});
