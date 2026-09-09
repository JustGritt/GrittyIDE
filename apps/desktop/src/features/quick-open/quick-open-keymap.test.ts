import { describe, expect, it } from 'vitest';
import { type QuickOpenChord, quickOpenIntentFor } from './quick-open-keymap';

/**
 * What the bar claims from the keyboard, and — the half that matters more —
 * what it leaves alone (T130-U §8).
 */

function chord(code: string, modifiers: Partial<QuickOpenChord> = {}): QuickOpenChord {
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

describe('moving through the list', () => {
  it('arrows move one row', () => {
    expect(quickOpenIntentFor(chord('ArrowDown'))).toEqual({ kind: 'move', delta: 1 });
    expect(quickOpenIntentFor(chord('ArrowUp'))).toEqual({ kind: 'move', delta: -1 });
  });

  it('Home and End go to the edges', () => {
    expect(quickOpenIntentFor(chord('Home'))).toEqual({ kind: 'edge', edge: 'first' });
    expect(quickOpenIntentFor(chord('End'))).toEqual({ kind: 'edge', edge: 'last' });
  });

  it('does not extend anything with Shift, because a palette picks one row', () => {
    // The tree's arrows carry `extend` for a Shift range; quick open opens
    // exactly one thing, so a range would be a selection nothing can act on.
    expect(quickOpenIntentFor(chord('ArrowDown', { shiftKey: true }))).toEqual({
      kind: 'move',
      delta: 1,
    });
    expect(quickOpenIntentFor(chord('Home', { shiftKey: true }))).toEqual({
      kind: 'edge',
      edge: 'first',
    });
  });

  it('leaves a modified arrow to the field, so ⌘↓ still reaches the caret', () => {
    // Inside a text input ⌘↓ and ⌥↑ are caret movements. Swallowing them would
    // take away editing the query in exchange for a second way to do what the
    // plain arrow already does.
    expect(quickOpenIntentFor(chord('ArrowDown', { metaKey: true }))).toBeNull();
    expect(quickOpenIntentFor(chord('ArrowUp', { altKey: true }))).toBeNull();
    expect(quickOpenIntentFor(chord('Home', { metaKey: true }))).toBeNull();
    expect(quickOpenIntentFor(chord('End', { altKey: true }))).toBeNull();
  });
});

describe('accepting and dismissing', () => {
  it('Enter opens a preview, which is what makes quick open a look', () => {
    // Preview is the default (§5), matching the explorer's single click: the
    // common case is glancing at a file, not committing a tab to it.
    expect(quickOpenIntentFor(chord('Enter'))).toEqual({ kind: 'accept', preview: true });
    expect(quickOpenIntentFor(chord('NumpadEnter'))).toEqual({ kind: 'accept', preview: true });
  });

  it('⌘Enter and Ctrl+Enter open for keeps', () => {
    // Either modifier, the repo's convention, so the chord works on both
    // platforms without the bar needing to know which one it is on.
    expect(quickOpenIntentFor(chord('Enter', { metaKey: true }))).toEqual({
      kind: 'accept',
      preview: false,
    });
    expect(quickOpenIntentFor(chord('Enter', { ctrlKey: true }))).toEqual({
      kind: 'accept',
      preview: false,
    });
  });

  it('leaves ⇧Enter and ⌥Enter unclaimed rather than aliasing them to Enter', () => {
    // A chord that quietly does something else is worse than one that does
    // nothing; neither has a meaning here.
    expect(quickOpenIntentFor(chord('Enter', { shiftKey: true }))).toBeNull();
    expect(quickOpenIntentFor(chord('Enter', { altKey: true }))).toBeNull();
  });

  it('Escape dismisses, with or without Shift', () => {
    expect(quickOpenIntentFor(chord('Escape'))).toEqual({ kind: 'dismiss' });
    expect(quickOpenIntentFor(chord('Escape', { shiftKey: true }))).toEqual({ kind: 'dismiss' });
  });

  it('leaves ⌘Escape and ⌥Escape to whatever owns them', () => {
    expect(quickOpenIntentFor(chord('Escape', { metaKey: true }))).toBeNull();
    expect(quickOpenIntentFor(chord('Escape', { altKey: true }))).toBeNull();
  });
});

describe('everything unclaimed reaches the field', () => {
  it('returns null for every printable key, so the query can be typed', () => {
    // The reason explorer-keymap.ts:52 gives, with more at stake: the field is
    // a real input that keeps focus (§10), and a swallowed letter is a letter
    // the user cannot type.
    for (const code of ['KeyA', 'KeyE', 'Digit1', 'Slash', 'Space', 'Period', 'Minus']) {
      expect(quickOpenIntentFor(chord(code))).toBeNull();
    }
  });

  it('returns null for Tab, Backspace and the page keys', () => {
    // Tab must move focus, Backspace must edit, and PageUp/PageDown are not in
    // §8's list — claiming them would be scope the ticket did not ask for.
    for (const code of ['Tab', 'Backspace', 'Delete', 'PageUp', 'PageDown', 'F2']) {
      expect(quickOpenIntentFor(chord(code))).toBeNull();
    }
  });

  it('leaves the shell chords alone, so ⌘S and ⌘O still work with the bar open', () => {
    for (const code of ['KeyS', 'KeyO', 'KeyP', 'KeyW']) {
      expect(quickOpenIntentFor(chord(code, { metaKey: true }))).toBeNull();
    }
  });
});

describe('chords are matched on the physical key', () => {
  it('acts on `code` and ignores `key` entirely', () => {
    // On macOS a modifier composes, so `key` is whatever character came out
    // the other side. A layout where the Enter key reports some other `key` —
    // or an ArrowDown that reports one — must still do the right thing.
    expect(quickOpenIntentFor(chord('Enter', { key: 'q' }))).toEqual({
      kind: 'accept',
      preview: true,
    });
    expect(quickOpenIntentFor(chord('ArrowDown', { key: '' }))).toEqual({
      kind: 'move',
      delta: 1,
    });
  });

  it('does not accept on a letter key that merely reports key: Enter', () => {
    // The inverse, and the bug matching on `key` would produce: a keystroke
    // that is not the Enter key must not open anything.
    expect(quickOpenIntentFor(chord('KeyE', { key: 'Enter' }))).toBeNull();
    expect(quickOpenIntentFor(chord('KeyJ', { key: 'ArrowDown' }))).toBeNull();
  });
});
