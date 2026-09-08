import { describe, expect, it } from 'vitest';
import { type ExplorerChord, explorerIntentFor } from './explorer-keymap';

function chord(code: string, modifiers: Partial<ExplorerChord> = {}): ExplorerChord {
  return {
    code,
    key: code.startsWith('Key') ? code.slice(3).toLowerCase() : code,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...modifiers,
  };
}

describe('navigation', () => {
  it('arrows move one row and Shift extends the range', () => {
    expect(explorerIntentFor(chord('ArrowDown'))).toEqual({
      kind: 'move',
      delta: 1,
      extend: false,
    });
    expect(explorerIntentFor(chord('ArrowUp', { shiftKey: true }))).toEqual({
      kind: 'move',
      delta: -1,
      extend: true,
    });
  });

  it('Home and End go to the edges', () => {
    expect(explorerIntentFor(chord('Home'))).toEqual({
      kind: 'edge',
      edge: 'first',
      extend: false,
    });
    expect(explorerIntentFor(chord('End', { shiftKey: true }))).toEqual({
      kind: 'edge',
      edge: 'last',
      extend: true,
    });
  });

  it('left and right open and close a directory', () => {
    expect(explorerIntentFor(chord('ArrowRight'))).toEqual({ kind: 'expand' });
    expect(explorerIntentFor(chord('ArrowLeft'))).toEqual({ kind: 'collapse' });
  });

  it('leaves a modified arrow to the window, so ⌘← is not swallowed', () => {
    expect(explorerIntentFor(chord('ArrowLeft', { metaKey: true }))).toBeNull();
    expect(explorerIntentFor(chord('ArrowDown', { altKey: true }))).toBeNull();
  });
});

describe('opening and renaming', () => {
  it('Enter opens and F2 renames, rather than VS Code macOS’s Enter-to-rename', () => {
    expect(explorerIntentFor(chord('Enter'))).toEqual({ kind: 'open' });
    expect(explorerIntentFor(chord('F2'))).toEqual({ kind: 'rename' });
    expect(explorerIntentFor(chord('Space'))).toEqual({ kind: 'preview' });
  });

  it('ignores a modified Enter', () => {
    expect(explorerIntentFor(chord('Enter', { metaKey: true }))).toBeNull();
    expect(explorerIntentFor(chord('Enter', { shiftKey: true }))).toBeNull();
  });
});

describe('deleting', () => {
  it('both delete keys trash, and Shift makes it permanent', () => {
    for (const code of ['Backspace', 'Delete']) {
      expect(explorerIntentFor(chord(code)), code).toEqual({
        kind: 'delete',
        permanent: false,
      });
      expect(explorerIntentFor(chord(code, { shiftKey: true })), code).toEqual({
        kind: 'delete',
        permanent: true,
      });
    }
  });

  it('⌘⌫ means the same as ⌫, because the two habits must not diverge', () => {
    expect(explorerIntentFor(chord('Backspace', { metaKey: true }))).toEqual({
      kind: 'delete',
      permanent: false,
    });
  });
});

describe('clipboard, undo and paths', () => {
  it('maps the standard editing chords', () => {
    expect(explorerIntentFor(chord('KeyX', { metaKey: true }))).toEqual({ kind: 'cut' });
    expect(explorerIntentFor(chord('KeyC', { metaKey: true }))).toEqual({ kind: 'copy' });
    expect(explorerIntentFor(chord('KeyV', { metaKey: true }))).toEqual({ kind: 'paste' });
    expect(explorerIntentFor(chord('KeyD', { metaKey: true }))).toEqual({ kind: 'duplicate' });
    expect(explorerIntentFor(chord('KeyA', { metaKey: true }))).toEqual({ kind: 'selectAll' });
  });

  it('accepts Ctrl as the command modifier too', () => {
    expect(explorerIntentFor(chord('KeyC', { ctrlKey: true }))).toEqual({ kind: 'copy' });
  });

  it('undoes and redoes, including the Windows redo', () => {
    expect(explorerIntentFor(chord('KeyZ', { metaKey: true }))).toEqual({ kind: 'undo' });
    expect(explorerIntentFor(chord('KeyZ', { metaKey: true, shiftKey: true }))).toEqual({
      kind: 'redo',
    });
    expect(explorerIntentFor(chord('KeyY', { metaKey: true }))).toEqual({ kind: 'redo' });
  });

  it('⌥⌘C copies the path and ⇧⌥⌘C the relative one', () => {
    expect(explorerIntentFor(chord('KeyC', { metaKey: true, altKey: true }))).toEqual({
      kind: 'copyPath',
      relative: false,
    });
    expect(
      explorerIntentFor(chord('KeyC', { metaKey: true, altKey: true, shiftKey: true })),
    ).toEqual({ kind: 'copyPath', relative: true });
  });

  it('leaves the bare letters to type-ahead, not to the clipboard', () => {
    // A letter that carries a shortcut must still be typeable to jump with,
    // or `x`, `a`, `v` and `z` would be the only letters that do nothing.
    for (const [code, character] of [
      ['KeyX', 'x'],
      ['KeyC', 'c'],
      ['KeyV', 'v'],
      ['KeyA', 'a'],
      ['KeyZ', 'z'],
      ['KeyD', 'd'],
      ['KeyY', 'y'],
    ] as const) {
      expect(explorerIntentFor({ ...chord(code), key: character }), code).toEqual({
        kind: 'typeahead',
        character,
      });
    }
  });
});

describe('type-ahead', () => {
  it('takes a single printable character', () => {
    expect(explorerIntentFor({ ...chord('KeyR'), key: 'r' })).toEqual({
      kind: 'typeahead',
      character: 'r',
    });
    expect(explorerIntentFor({ ...chord('Digit2'), key: '2' })).toEqual({
      kind: 'typeahead',
      character: '2',
    });
  });

  it('ignores modified keys, dead keys and named keys', () => {
    expect(explorerIntentFor({ ...chord('KeyR'), key: 'r', metaKey: true })).toBeNull();
    // ⌥e composes on macOS; what it produces is not what is being spelled.
    expect(explorerIntentFor({ ...chord('KeyE'), key: 'e', altKey: true })).toBeNull();
    expect(explorerIntentFor({ ...chord('Tab'), key: 'Tab' })).toBeNull();
    expect(explorerIntentFor({ ...chord('Escape'), key: 'Escape' })).toBeNull();
  });
});
