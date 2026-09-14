import { describe, expect, it } from 'vitest';
import type { Accelerator } from '../shell/open-shortcuts';
import { isSettingsChord, type SettingsKeyChord } from './settings-chord';

/**
 * The settings chord (T120-M §4).
 *
 * Pure, so the rules are a test rather than a click — the same split
 * `open-shortcuts.test.ts` and `quick-open-chord.test.ts` use.
 */

function chord(overrides: Partial<SettingsKeyChord> = {}): SettingsKeyChord {
  return {
    key: ',',
    code: 'Comma',
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...overrides,
  };
}

const each: readonly Accelerator[] = ['meta', 'ctrl'];

describe('isSettingsChord', () => {
  it('matches the platform accelerator with Comma', () => {
    expect(isSettingsChord(chord({ metaKey: true }), 'meta')).toBe(true);
    expect(isSettingsChord(chord({ ctrlKey: true }), 'ctrl')).toBe(true);
  });

  it('rejects the other platform accelerator', () => {
    // Accepting both would swallow Ctrl+, on macOS, where it belongs to
    // whatever has focus. `matchOpenShortcut` gives the same reasoning for ⌘O.
    expect(isSettingsChord(chord({ ctrlKey: true }), 'meta')).toBe(false);
    expect(isSettingsChord(chord({ metaKey: true }), 'ctrl')).toBe(false);
  });

  it('rejects the bare key', () => {
    for (const accelerator of each) {
      expect(isSettingsChord(chord(), accelerator)).toBe(false);
    }
  });

  it('rejects a superset, so a chord somebody else claims is left alone', () => {
    expect(isSettingsChord(chord({ metaKey: true, shiftKey: true }), 'meta')).toBe(false);
    expect(isSettingsChord(chord({ metaKey: true, altKey: true }), 'meta')).toBe(false);
    expect(isSettingsChord(chord({ metaKey: true, ctrlKey: true }), 'meta')).toBe(false);
    expect(isSettingsChord(chord({ ctrlKey: true, shiftKey: true }), 'ctrl')).toBe(false);
    expect(isSettingsChord(chord({ ctrlKey: true, altKey: true }), 'ctrl')).toBe(false);
  });

  it('rejects another key', () => {
    for (const accelerator of each) {
      const held = accelerator === 'meta' ? { metaKey: true } : { ctrlKey: true };
      expect(isSettingsChord(chord({ ...held, code: 'Period' }), accelerator)).toBe(false);
      expect(isSettingsChord(chord({ ...held, code: 'KeyP' }), accelerator)).toBe(false);
    }
  });

  it('reads the physical key, not the character it produced', () => {
    // On macOS a modifier composes, so `key` is whatever came out the other
    // side. Five other chord files in this tree record the same thing.
    expect(isSettingsChord(chord({ metaKey: true, key: '≤' }), 'meta')).toBe(true);
    expect(isSettingsChord(chord({ metaKey: true, code: 'IntlBackslash', key: ',' }), 'meta')).toBe(
      false,
    );
  });
});
