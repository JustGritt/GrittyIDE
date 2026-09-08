/**
 * What the explorer has selected, and how a click or an arrow key changes it
 * (T042).
 *
 * The tree had no selection at all before this: no `selected`, no focus, no
 * `tabIndex`. Every file operation needs an answer to "which rows is this
 * about", so the algebra lives here, pure and over a plain list of visible
 * paths, and the reducer and the panel both defer to it.
 *
 * "Visible" means the rows `selectRows` currently produces, in order — so a
 * range never reaches into a collapsed directory the user cannot see, and
 * Shift+Down after collapsing a folder does what the screen shows.
 */

export interface Selection {
  /** Selected paths, in visible order. */
  readonly paths: readonly string[];
  /** Where a Shift-range measures from; the last row picked without Shift. */
  readonly anchor: string | null;
  /** The row that owns the roving `tabIndex`. */
  readonly focused: string | null;
}

export const emptySelection: Selection = { paths: [], anchor: null, focused: null };

/** Which modifier the pointer or keyboard carried. */
export interface SelectModifiers {
  /** Cmd/Ctrl: add or remove one row, keeping the rest. */
  readonly toggle?: boolean;
  /** Shift: select from the anchor to here. */
  readonly range?: boolean;
}

function ordered(visible: readonly string[], chosen: Iterable<string>): string[] {
  const wanted = new Set(chosen);
  return visible.filter((path) => wanted.has(path));
}

function rangeBetween(visible: readonly string[], from: string, to: string): string[] {
  const start = visible.indexOf(from);
  const end = visible.indexOf(to);
  if (start === -1 || end === -1) return [to];
  const [low, high] = start <= end ? [start, end] : [end, start];
  return visible.slice(low, high + 1);
}

/**
 * Apply a click (or an Enter/Space) on `path`.
 *
 * Plain replaces the selection. Cmd toggles one row and moves the anchor to it,
 * so a following Shift measures from where the user last acted. Shift extends
 * from the anchor, and — like every file manager — *replaces* what the previous
 * Shift selected rather than accumulating ranges.
 */
export function selectOn(
  selection: Selection,
  visible: readonly string[],
  path: string,
  modifiers: SelectModifiers = {},
): Selection {
  if (modifiers.range && selection.anchor !== null) {
    return {
      paths: ordered(visible, rangeBetween(visible, selection.anchor, path)),
      anchor: selection.anchor,
      focused: path,
    };
  }
  if (modifiers.toggle) {
    const held = new Set(selection.paths);
    if (held.has(path)) held.delete(path);
    else held.add(path);
    return { paths: ordered(visible, held), anchor: path, focused: path };
  }
  return { paths: [path], anchor: path, focused: path };
}

/** Select every visible row (⌘A). */
export function selectAll(selection: Selection, visible: readonly string[]): Selection {
  return {
    paths: [...visible],
    anchor: selection.anchor ?? visible[0] ?? null,
    focused: selection.focused ?? visible[0] ?? null,
  };
}

/** Where the roving `tabIndex` sits, as an index into `visible`. */
export function focusIndex(selection: Selection, visible: readonly string[]): number {
  if (selection.focused === null) return -1;
  return visible.indexOf(selection.focused);
}

/**
 * Move the focus to `index`, clamped to the list.
 *
 * `extend` is Shift+Arrow: the focus moves and the range from the anchor grows
 * with it. Without it the focus takes the selection with it, which is what
 * Up/Down do in every tree.
 */
export function focusAt(
  selection: Selection,
  visible: readonly string[],
  index: number,
  extend = false,
): Selection {
  if (visible.length === 0) return emptySelection;
  const clamped = Math.max(0, Math.min(visible.length - 1, index));
  const path = visible[clamped];
  if (path === undefined) return selection;
  if (extend && selection.anchor !== null) {
    return {
      paths: ordered(visible, rangeBetween(visible, selection.anchor, path)),
      anchor: selection.anchor,
      focused: path,
    };
  }
  return { paths: [path], anchor: path, focused: path };
}

/** Move the focus by `delta` rows from where it is. */
export function moveFocus(
  selection: Selection,
  visible: readonly string[],
  delta: number,
  extend = false,
): Selection {
  const current = focusIndex(selection, visible);
  // Nothing focused yet: Down lands on the first row, Up on the last.
  const from = current === -1 ? (delta > 0 ? -1 : visible.length) : current;
  return focusAt(selection, visible, from + delta, extend);
}

/**
 * Drop what is no longer on screen.
 *
 * Called after a re-list, which every file operation causes. A selected path
 * that vanished (it was renamed, deleted, or its directory collapsed) leaves
 * the selection; the focus moves to the nearest surviving row rather than
 * disappearing, so the keyboard still has somewhere to be.
 */
export function keepVisible(selection: Selection, visible: readonly string[]): Selection {
  const present = new Set(visible);
  const paths = selection.paths.filter((path) => present.has(path));
  const anchor =
    selection.anchor !== null && present.has(selection.anchor) ? selection.anchor : null;
  let focused = selection.focused;
  if (focused !== null && !present.has(focused)) {
    focused = paths[0] ?? visible[0] ?? null;
  }
  if (
    paths.length === selection.paths.length &&
    anchor === selection.anchor &&
    focused === selection.focused
  ) {
    return selection;
  }
  return { paths, anchor: anchor ?? focused, focused };
}

/**
 * Select something after it appeared — the file a create just made, or the
 * entry a rename produced. Nothing else stays selected, because the user's
 * attention is on the thing they just made.
 */
export function reveal(visible: readonly string[], path: string): Selection {
  if (!visible.includes(path)) return { paths: [path], anchor: path, focused: path };
  return { paths: [path], anchor: path, focused: path };
}

/**
 * Type-to-jump: the next row whose name starts with `query`, searching from
 * after the focused row and wrapping, so typing the same letter cycles through
 * the matches the way a file manager does.
 *
 * `nameOf` maps a path to the name shown, because a path is not its label.
 */
export function typeahead(
  selection: Selection,
  visible: readonly string[],
  query: string,
  nameOf: (path: string) => string,
): string | null {
  if (query === '') return null;
  const needle = query.toLowerCase();
  const start = focusIndex(selection, visible);
  for (let step = 1; step <= visible.length; step += 1) {
    const candidate = visible[(start + step + visible.length) % visible.length];
    if (candidate === undefined) continue;
    if (nameOf(candidate).toLowerCase().startsWith(needle)) return candidate;
  }
  return null;
}
