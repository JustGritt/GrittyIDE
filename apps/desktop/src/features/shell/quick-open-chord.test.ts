import { describe, expect, it } from 'vitest';
import { acceleratorForOs, ownsRawKeystrokes } from './open-shortcuts';
import {
  initialQueryFor,
  matchQuickOpenShortcut,
  type QuickOpenKeyChord,
} from './quick-open-chord';

/**
 * The quick-open chord (T130-M §2, §4, §5).
 *
 * Pure, and the cases that matter are the ones a hand test on one machine
 * would never reach: the other platform's modifier, a composed key, and the
 * modifier supersets that belong to something else.
 */

function chord(overrides: Partial<QuickOpenKeyChord> = {}): QuickOpenKeyChord {
  return {
    code: 'KeyP',
    key: 'p',
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...overrides,
  };
}

describe('matchQuickOpenShortcut', () => {
  it('takes ⌘P on macOS and Ctrl+P everywhere else', () => {
    expect(matchQuickOpenShortcut(chord({ metaKey: true }), 'meta')).toBe('files');
    expect(matchQuickOpenShortcut(chord({ ctrlKey: true }), 'ctrl')).toBe('files');
  });

  it('ignores the other platform’s modifier, so it steals nothing', () => {
    // Ctrl+P on macOS belongs to whatever has focus — a shell reads it as a
    // control character. ⌘P on Linux is not a chord at all.
    expect(matchQuickOpenShortcut(chord({ ctrlKey: true }), 'meta')).toBeNull();
    expect(matchQuickOpenShortcut(chord({ metaKey: true }), 'ctrl')).toBeNull();
  });

  it('distinguishes ⌘⇧P from ⌘P, and pre-fills the mode it named', () => {
    expect(matchQuickOpenShortcut(chord({ metaKey: true, shiftKey: true }), 'meta')).toBe(
      'commands',
    );
    expect(initialQueryFor('commands')).toBe('>');
    expect(initialQueryFor('files')).toBe('');
  });

  it('reads the physical key, so a composed character still matches', () => {
    // ⌥P produces `π` on macOS. A rule written against `key` would miss the
    // chord entirely and, worse, would match `π` typed by other means.
    expect(matchQuickOpenShortcut(chord({ metaKey: true, key: 'π' }), 'meta')).toBe('files');
    expect(
      matchQuickOpenShortcut(chord({ metaKey: true, code: 'KeyO', key: 'p' }), 'meta'),
    ).toBeNull();
  });

  it('refuses a modifier superset', () => {
    // ⌘⌥P and ⌘⌃P belong to something else; claiming them here is how a
    // hand-gated listener quietly grows.
    expect(matchQuickOpenShortcut(chord({ metaKey: true, altKey: true }), 'meta')).toBeNull();
    expect(matchQuickOpenShortcut(chord({ metaKey: true, ctrlKey: true }), 'meta')).toBeNull();
    expect(matchQuickOpenShortcut(chord({ ctrlKey: true, altKey: true }), 'ctrl')).toBeNull();
  });

  it('returns null for an unclaimed chord, rather than guessing', () => {
    expect(matchQuickOpenShortcut(chord(), 'meta')).toBeNull();
    expect(matchQuickOpenShortcut(chord({ metaKey: true, code: 'KeyK' }), 'meta')).toBeNull();
    expect(matchQuickOpenShortcut(chord({ metaKey: true, code: 'Enter' }), 'meta')).toBeNull();
  });

  it('derives the accelerator from the core’s os, never from navigator', () => {
    expect(acceleratorForOs('macos')).toBe('meta');
    expect(acceleratorForOs('linux')).toBe('ctrl');
    expect(acceleratorForOs('windows')).toBe('ctrl');
  });
});

describe('what the window listener must not take', () => {
  it('leaves a live shell alone, which is what makes Ctrl+P safe on Linux', () => {
    // xterm puts a <textarea> under the cursor, so `Ctrl+P` reaching a shell
    // is readline's "previous command" and must not open a palette. Monaco
    // does the same, which is why the editor gets its own registration
    // instead of relying on this listener (§2).
    expect(ownsRawKeystrokes({ tagName: 'TEXTAREA' })).toBe(true);
    expect(ownsRawKeystrokes({ tagName: 'DIV', isContentEditable: true })).toBe(true);
    expect(ownsRawKeystrokes({ tagName: 'BUTTON' })).toBe(false);
    expect(ownsRawKeystrokes(null)).toBe(false);
  });
});
