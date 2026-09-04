import { describe, expect, it } from 'vitest';
import {
  type Accelerator,
  acceleratorForOs,
  type KeyChord,
  matchOpenShortcut,
  ownsRawKeystrokes,
} from './open-shortcuts';

function chord(overrides: Partial<KeyChord> = {}): KeyChord {
  return { key: 'o', metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...overrides };
}

describe('acceleratorForOs', () => {
  it('follows the core, which knows what it is running on', () => {
    expect(acceleratorForOs('macos')).toBe<Accelerator>('meta');
    expect(acceleratorForOs('linux')).toBe<Accelerator>('ctrl');
    expect(acceleratorForOs('windows')).toBe<Accelerator>('ctrl');
  });
});

describe('matchOpenShortcut', () => {
  it('reads the two chords the launcher advertises', () => {
    expect(matchOpenShortcut(chord({ metaKey: true }), 'meta')).toBe('folder');
    expect(matchOpenShortcut(chord({ metaKey: true, shiftKey: true }), 'meta')).toBe('path');
    expect(matchOpenShortcut(chord({ ctrlKey: true }), 'ctrl')).toBe('folder');
    expect(matchOpenShortcut(chord({ ctrlKey: true, shiftKey: true }), 'ctrl')).toBe('path');
  });

  it('accepts an uppercase key, which is what Shift produces', () => {
    expect(matchOpenShortcut(chord({ key: 'O', metaKey: true, shiftKey: true }), 'meta')).toBe(
      'path',
    );
  });

  it('ignores the other platform s modifier rather than answering to both', () => {
    // Ctrl+O on macOS belongs to whatever has focus — a shell reads it as a
    // control character — so the accelerator is exclusive, not permissive.
    expect(matchOpenShortcut(chord({ ctrlKey: true }), 'meta')).toBeNull();
    expect(matchOpenShortcut(chord({ metaKey: true }), 'ctrl')).toBeNull();
    expect(matchOpenShortcut(chord({ metaKey: true, ctrlKey: true }), 'meta')).toBeNull();
  });

  it('ignores anything else', () => {
    expect(matchOpenShortcut(chord({ metaKey: true, altKey: true }), 'meta')).toBeNull();
    expect(matchOpenShortcut(chord({ key: 's', metaKey: true }), 'meta')).toBeNull();
    expect(matchOpenShortcut(chord(), 'meta')).toBeNull();
  });
});

describe('ownsRawKeystrokes', () => {
  it('leaves a textarea alone, which is what Monaco and xterm focus', () => {
    expect(ownsRawKeystrokes({ tagName: 'TEXTAREA' })).toBe(true);
    expect(ownsRawKeystrokes({ tagName: 'textarea' })).toBe(true);
    expect(ownsRawKeystrokes({ tagName: 'DIV', isContentEditable: true })).toBe(true);
  });

  it('claims the chord everywhere else, including the launcher s own field', () => {
    expect(ownsRawKeystrokes({ tagName: 'INPUT' })).toBe(false);
    expect(ownsRawKeystrokes({ tagName: 'BODY' })).toBe(false);
    expect(ownsRawKeystrokes(null)).toBe(false);
    expect(ownsRawKeystrokes(undefined)).toBe(false);
  });
});
