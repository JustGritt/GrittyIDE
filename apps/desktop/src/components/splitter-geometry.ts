/**
 * The arithmetic behind a draggable gutter, kept out of the component so it
 * can be tested without a DOM, a pointer or a layout pass.
 *
 * Every size here is a number of CSS pixels along one axis: the sidebar's
 * width, the dock's height. Which axis, and which way "bigger" points, is the
 * caller's business — see [`Direction`].
 */

/** Smallest and largest a pane may be, in pixels. */
export interface SplitterBounds {
  readonly min: number;
  readonly max: number;
}

/**
 * Which way the pointer must move to make the pane bigger: `1` when the pane
 * grows with the coordinate (a sidebar on the left grows rightwards), `-1`
 * when it grows against it (a dock at the bottom grows upwards).
 */
export type Direction = 1 | -1;

/**
 * Clamp to the bounds, tolerating bounds that have collapsed.
 *
 * A window narrower than both panes' minimums produces `max < min`, and a
 * gutter must still resolve to something drawable rather than a negative
 * track. The minimum wins: a pane at its minimum is usable, a pane at a
 * negative width is a broken layout.
 */
export function clampSize(size: number, bounds: SplitterBounds): number {
  // NaN carries no information, so it becomes the minimum. An infinity does
  // carry one — "as far as it goes" — and clamps like any other number.
  if (Number.isNaN(size)) return bounds.min;
  if (bounds.max < bounds.min) return bounds.min;
  return Math.min(Math.max(Math.round(size), bounds.min), bounds.max);
}

/**
 * The size a drag has reached.
 *
 * Deliberately relative to where the drag started rather than to the pane's
 * own edge: the pane's edge moves as it resizes, and measuring against it
 * makes the gutter drift away from the pointer over a long drag.
 */
export function sizeFromDelta(
  startSize: number,
  delta: number,
  direction: Direction,
  bounds: SplitterBounds,
): number {
  return clampSize(startSize + delta * direction, bounds);
}

/**
 * The size a key press asks for, or `null` when the key is not ours — which
 * is what lets the component leave every other key to the browser.
 *
 * `vertical` is the *separator's* own orientation, matching `aria-orientation`:
 * a vertical separator divides left from right, so it responds to Left/Right.
 */
export function sizeFromKey(
  key: string,
  current: number,
  step: number,
  direction: Direction,
  bounds: SplitterBounds,
  orientation: 'vertical' | 'horizontal',
): number | null {
  const [smaller, larger] =
    orientation === 'vertical' ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown'];
  switch (key) {
    case smaller:
      return clampSize(current - step * direction, bounds);
    case larger:
      return clampSize(current + step * direction, bounds);
    case 'Home':
      return bounds.min;
    case 'End':
      return clampSize(bounds.max, bounds);
    default:
      return null;
  }
}

/**
 * Bounds for a pane sharing an axis with a neighbour that has its own minimum.
 *
 * The pane may not grow so far that the neighbour drops below `otherMin`, so a
 * small window shrinks the maximum rather than letting one pane win.
 */
export function boundsWithin(
  available: number,
  min: number,
  otherMin: number,
  hardMax = Number.POSITIVE_INFINITY,
): SplitterBounds {
  return { min, max: Math.min(hardMax, Math.max(min, available - otherMin)) };
}
