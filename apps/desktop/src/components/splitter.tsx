import { type RefObject, useCallback, useEffect, useRef, useState } from 'react';
import {
  clampSize,
  type Direction,
  type SplitterBounds,
  sizeFromDelta,
  sizeFromKey,
} from './splitter-geometry';

/** How far one arrow key press moves a gutter. */
const KEYBOARD_STEP = 16;

export function Splitter({
  orientation,
  label,
  value,
  fallback,
  direction,
  target,
  property,
  bounds,
  onCommit,
}: {
  /**
   * The *separator's* own orientation, as `aria-orientation` means it: a
   * vertical separator divides left from right.
   */
  orientation: 'vertical' | 'horizontal';
  /** Announced name, e.g. "Resize the sidebar". */
  label: string;
  /** The committed size, in pixels. */
  value: number;
  /** Restored on double-click. */
  fallback: number;
  /** Which way the pane grows; see `splitter-geometry`. */
  direction: Direction;
  /** Element carrying the CSS custom property this gutter drives. */
  target: RefObject<HTMLElement | null>;
  /** Name of that property, e.g. `--sidebar-width`. */
  property: string;
  /** Evaluated when a drag or a key press starts, against the live layout. */
  bounds: (target: HTMLElement) => SplitterBounds;
  onCommit: (size: number) => void;
}) {
  // A drag writes the CSS variable directly and never touches React state:
  // re-rendering the shell — Monaco and every xterm with it — on each pointer
  // event is exactly the renderer-blocking work the layout must not do. React
  // hears about the size once, on release.
  const drag = useRef<{
    pointerId: number;
    origin: number;
    startSize: number;
    bounds: SplitterBounds;
    latest: number;
  } | null>(null);

  const preview = useCallback(
    (size: number) => {
      target.current?.style.setProperty(property, `${size}px`);
    },
    [target, property],
  );

  const axis = useCallback(
    (event: { clientX: number; clientY: number }) =>
      orientation === 'vertical' ? event.clientX : event.clientY,
    [orientation],
  );

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const element = target.current;
      // Only the primary button drags; a right-click must not start one.
      if (!element || event.button !== 0) return;
      event.preventDefault();
      const limits = bounds(element);
      drag.current = {
        pointerId: event.pointerId,
        origin: axis(event),
        startSize: clampSize(value, limits),
        bounds: limits,
        latest: clampSize(value, limits),
      };
      // Capture so the drag survives the pointer leaving this thin element,
      // and so a release outside the window still ends it.
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [axis, bounds, target, value],
  );

  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const state = drag.current;
      if (!state || state.pointerId !== event.pointerId) return;
      const size = sizeFromDelta(
        state.startSize,
        axis(event) - state.origin,
        direction,
        state.bounds,
      );
      state.latest = size;
      preview(size);
    },
    [axis, direction, preview],
  );

  const endDrag = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const state = drag.current;
      if (!state || state.pointerId !== event.pointerId) return;
      drag.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
      // Committing an unchanged size would still rewrite the session blob.
      if (state.latest !== value) onCommit(state.latest);
    },
    [onCommit, value],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      const element = target.current;
      if (!element) return;
      const limits = bounds(element);
      const size = sizeFromKey(event.key, value, KEYBOARD_STEP, direction, limits, orientation);
      if (size === null) return;
      event.preventDefault();
      preview(size);
      if (size !== value) onCommit(size);
    },
    [bounds, direction, onCommit, orientation, preview, target, value],
  );

  const reset = useCallback(() => {
    const element = target.current;
    const size = element ? clampSize(fallback, bounds(element)) : fallback;
    preview(size);
    if (size !== value) onCommit(size);
  }, [bounds, fallback, onCommit, preview, target, value]);

  // Announced separately from the drag's own bounds: `target.current` is null
  // on the first render, and a separator whose range is never announced is a
  // control a screen reader cannot describe. Recomputed when the window
  // changes, which is the only thing that moves them.
  const [announced, setAnnounced] = useState<SplitterBounds | null>(null);
  useEffect(() => {
    const measure = () => {
      const element = target.current;
      if (element) setAnnounced(bounds(element));
    };
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, [bounds, target]);

  // `useSemanticElements` is off for this file (biome.json): its suggestion,
  // `<hr>`, is the non-focusable thematic break and cannot carry a value.
  return (
    <div
      className={`splitter splitter--${orientation}`}
      // A separator with a value is focusable and operable by definition;
      // the pointer drag is the shortcut, not the only way in.
      role="separator"
      tabIndex={0}
      aria-label={label}
      aria-orientation={orientation}
      aria-valuenow={value}
      aria-valuemin={announced?.min}
      aria-valuemax={announced?.max}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onKeyDown={onKeyDown}
      onDoubleClick={reset}
    />
  );
}
