import { describe, expect, it } from 'vitest';
import {
  emptySelection,
  focusAt,
  focusIndex,
  keepVisible,
  moveFocus,
  type Selection,
  selectAll,
  selectOn,
  typeahead,
} from './explorer-selection';

const VISIBLE = ['a.ts', 'b.ts', 'c.ts', 'dir', 'dir/x.ts', 'e.ts'];

/** The state after a plain click on `path`. */
function clicked(path: string): Selection {
  return selectOn(emptySelection, VISIBLE, path);
}

describe('selectOn', () => {
  it('a plain click replaces the selection and sets the anchor', () => {
    expect(clicked('b.ts')).toEqual({ paths: ['b.ts'], anchor: 'b.ts', focused: 'b.ts' });
  });

  it('Cmd-click adds and removes one row, keeping the rest in visible order', () => {
    const two = selectOn(clicked('c.ts'), VISIBLE, 'a.ts', { toggle: true });
    expect(two.paths).toEqual(['a.ts', 'c.ts']);
    expect(two.anchor).toBe('a.ts');

    const back = selectOn(two, VISIBLE, 'c.ts', { toggle: true });
    expect(back.paths).toEqual(['a.ts']);
  });

  it('Shift-click selects the range from the anchor, in either direction', () => {
    const down = selectOn(clicked('b.ts'), VISIBLE, 'dir', { range: true });
    expect(down.paths).toEqual(['b.ts', 'c.ts', 'dir']);
    expect(down.anchor).toBe('b.ts');

    const up = selectOn(clicked('dir'), VISIBLE, 'b.ts', { range: true });
    expect(up.paths).toEqual(['b.ts', 'c.ts', 'dir']);
  });

  it('a second Shift-click replaces the first range rather than adding to it', () => {
    const first = selectOn(clicked('a.ts'), VISIBLE, 'c.ts', { range: true });
    expect(first.paths).toHaveLength(3);
    const second = selectOn(first, VISIBLE, 'b.ts', { range: true });
    expect(second.paths).toEqual(['a.ts', 'b.ts']);
  });

  it('Shift with no anchor behaves like a plain click', () => {
    expect(selectOn(emptySelection, VISIBLE, 'c.ts', { range: true }).paths).toEqual(['c.ts']);
  });
});

describe('focus movement', () => {
  it('Down from nothing lands on the first row and Up on the last', () => {
    expect(moveFocus(emptySelection, VISIBLE, 1).focused).toBe('a.ts');
    expect(moveFocus(emptySelection, VISIBLE, -1).focused).toBe('e.ts');
  });

  it('clamps at both ends instead of wrapping', () => {
    expect(moveFocus(clicked('a.ts'), VISIBLE, -1).focused).toBe('a.ts');
    expect(moveFocus(clicked('e.ts'), VISIBLE, 1).focused).toBe('e.ts');
  });

  it('takes the selection with it, unless extending', () => {
    const moved = moveFocus(clicked('a.ts'), VISIBLE, 1);
    expect(moved.paths).toEqual(['b.ts']);

    const extended = moveFocus(clicked('a.ts'), VISIBLE, 2, true);
    expect(extended.paths).toEqual(['a.ts', 'b.ts', 'c.ts']);
    expect(extended.focused).toBe('c.ts');
    expect(extended.anchor).toBe('a.ts');
  });

  it('Home and End are just a clamped focusAt', () => {
    expect(focusAt(clicked('c.ts'), VISIBLE, 0).focused).toBe('a.ts');
    expect(focusAt(clicked('c.ts'), VISIBLE, 99).focused).toBe('e.ts');
  });

  it('reports where the roving tabIndex sits', () => {
    expect(focusIndex(clicked('c.ts'), VISIBLE)).toBe(2);
    expect(focusIndex(emptySelection, VISIBLE)).toBe(-1);
  });

  it('an empty tree focuses nothing rather than throwing', () => {
    expect(focusAt(clicked('a.ts'), [], 0)).toEqual(emptySelection);
  });
});

describe('selectAll', () => {
  it('takes every visible row', () => {
    expect(selectAll(emptySelection, VISIBLE).paths).toEqual(VISIBLE);
  });
});

describe('keepVisible', () => {
  it('drops what a re-list removed and keeps the rest', () => {
    const three = selectOn(clicked('a.ts'), VISIBLE, 'c.ts', { range: true });
    const after = keepVisible(three, ['a.ts', 'c.ts']);
    expect(after.paths).toEqual(['a.ts', 'c.ts']);
  });

  it('moves the focus to a survivor when the focused row vanished', () => {
    const deleted = keepVisible(clicked('b.ts'), ['a.ts', 'c.ts']);
    expect(deleted.paths).toEqual([]);
    expect(deleted.focused).toBe('a.ts');
  });

  it('leaves the tree focusable when everything vanished', () => {
    expect(keepVisible(clicked('b.ts'), [])).toEqual({
      paths: [],
      anchor: null,
      focused: null,
    });
  });

  it('returns the same object when nothing changed, so React does not re-render', () => {
    const selection = clicked('b.ts');
    expect(keepVisible(selection, VISIBLE)).toBe(selection);
  });
});

describe('typeahead', () => {
  const nameOf = (path: string) => path.split('/').pop() ?? path;

  it('jumps to the next match after the focus and wraps', () => {
    expect(typeahead(clicked('a.ts'), VISIBLE, 'b', nameOf)).toBe('b.ts');
    // From the last row, the search wraps to the top.
    expect(typeahead(clicked('e.ts'), VISIBLE, 'a', nameOf)).toBe('a.ts');
  });

  it('matches on the name, not the path', () => {
    expect(typeahead(clicked('a.ts'), VISIBLE, 'x', nameOf)).toBe('dir/x.ts');
  });

  it('is case-insensitive and returns null when nothing matches', () => {
    expect(typeahead(clicked('a.ts'), VISIBLE, 'DIR', nameOf)).toBe('dir');
    expect(typeahead(clicked('a.ts'), VISIBLE, 'zz', nameOf)).toBeNull();
    expect(typeahead(clicked('a.ts'), VISIBLE, '', nameOf)).toBeNull();
  });
});
