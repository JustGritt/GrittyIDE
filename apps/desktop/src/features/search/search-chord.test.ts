import { describe, expect, it } from 'vitest';
import { isSearchChord, type SearchKeyChord } from './search-chord';

/** The find-in-files chord (SEARCH-CHORD), as a rule rather than a listener. */

function chord(overrides: Partial<SearchKeyChord> = {}): SearchKeyChord {
  return {
    key: 'F',
    code: 'KeyF',
    metaKey: false,
    ctrlKey: false,
    shiftKey: true,
    altKey: false,
    ...overrides,
  };
}

describe('isSearchChord', () => {
  it('is ⌘⇧F on macOS and Ctrl+Shift+F elsewhere', () => {
    expect(isSearchChord(chord({ metaKey: true }), 'meta')).toBe(true);
    expect(isSearchChord(chord({ ctrlKey: true }), 'ctrl')).toBe(true);
  });

  it("takes only the platform's own accelerator", () => {
    // Ctrl+Shift+F on macOS belongs to whatever has focus.
    expect(isSearchChord(chord({ ctrlKey: true }), 'meta')).toBe(false);
    expect(isSearchChord(chord({ metaKey: true }), 'ctrl')).toBe(false);
  });

  it('leaves ⌘F to Monaco, and rejects every superset', () => {
    expect(isSearchChord(chord({ metaKey: true, shiftKey: false }), 'meta')).toBe(false);
    expect(isSearchChord(chord({ metaKey: true, altKey: true }), 'meta')).toBe(false);
    expect(isSearchChord(chord({ metaKey: true, ctrlKey: true }), 'meta')).toBe(false);
    expect(isSearchChord(chord({ ctrlKey: true, metaKey: true }), 'ctrl')).toBe(false);
  });

  it('reads the physical key, not the character it produced', () => {
    // A layout or a composing modifier can change `key`; the key pressed is `code`.
    expect(isSearchChord(chord({ metaKey: true, key: 'ƒ' }), 'meta')).toBe(true);
    expect(isSearchChord(chord({ metaKey: true, key: 'F', code: 'KeyG' }), 'meta')).toBe(false);
  });
});
