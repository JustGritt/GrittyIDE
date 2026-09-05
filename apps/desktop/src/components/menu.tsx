import { Fragment, type KeyboardEvent, useEffect, useLayoutEffect, useRef, useState } from 'react';

export interface MenuItem {
  readonly label: string;
  readonly onSelect: () => void;
  readonly disabled?: boolean;
  /**
   * A colour chip before the label, as a CSS colour. For a menu that picks a
   * colour, where the swatch is the actual content and the label names it.
   */
  readonly swatch?: string;
  /**
   * Present when the item is one of a set the menu picks from: it becomes a
   * `menuitemradio` and reports whether it is the current choice. Absent for an
   * ordinary command, which is neither on nor off.
   */
  readonly checked?: boolean;
  /** Draws a rule above this item, separating it from what came before. */
  readonly separatorBefore?: boolean;
}

/** Where the menu opens, in viewport coordinates (the pointer, or the control's corner). */
export interface MenuAnchor {
  readonly x: number;
  readonly y: number;
}

const ITEM_SELECTOR = '.menu__item:not([disabled])';
const MARGIN = 4;

/**
 * A pop-up menu, shared by the editor's tab strip and the terminal dock.
 * Fixed to the viewport rather than nested in whatever opened it, so a menu on
 * the last item of a scrolling strip is not clipped by it. Dismissed by
 * Escape, a click anywhere else, a scroll or a window resize: a menu that
 * outlives the thing it describes would act on the wrong one.
 */
export function Menu({
  label,
  anchor,
  items,
  onDismiss,
}: {
  label: string;
  anchor: MenuAnchor;
  items: readonly MenuItem[];
  onDismiss: () => void;
}) {
  const container = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState(anchor);

  // Measured before paint: a menu near the right or bottom edge flips back
  // inside the window instead of growing it a scrollbar.
  useLayoutEffect(() => {
    const element = container.current;
    if (!element) return;
    const { width, height } = element.getBoundingClientRect();
    setPosition({
      x: Math.max(MARGIN, Math.min(anchor.x, window.innerWidth - width - MARGIN)),
      y:
        anchor.y + height > window.innerHeight - MARGIN
          ? Math.max(MARGIN, anchor.y - height)
          : anchor.y,
    });
  }, [anchor]);

  useEffect(() => {
    container.current?.querySelector<HTMLButtonElement>(ITEM_SELECTOR)?.focus();
  }, []);

  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      const element = container.current;
      if (element && event.target instanceof Node && element.contains(event.target)) return;
      onDismiss();
    };
    // Capture: the tab underneath must not activate on the dismissing click.
    window.addEventListener('mousedown', onPointerDown, true);
    window.addEventListener('resize', onDismiss);
    window.addEventListener('scroll', onDismiss, true);
    return () => {
      window.removeEventListener('mousedown', onPointerDown, true);
      window.removeEventListener('resize', onDismiss);
      window.removeEventListener('scroll', onDismiss, true);
    };
  }, [onDismiss]);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === 'Escape' || event.key === 'Tab') {
      event.preventDefault();
      onDismiss();
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    const element = container.current;
    if (!element) return;
    const buttons = [...element.querySelectorAll<HTMLButtonElement>(ITEM_SELECTOR)];
    if (buttons.length === 0) return;
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const step = event.key === 'ArrowDown' ? 1 : -1;
    buttons[(current + step + buttons.length) % buttons.length]?.focus();
  }

  return (
    <div
      ref={container}
      className="menu"
      role="menu"
      aria-label={label}
      onKeyDown={onKeyDown}
      style={{ left: `${position.x}px`, top: `${position.y}px` }}
    >
      {items.map((item) => (
        <Fragment key={item.label}>
          {item.separatorBefore && <hr className="menu__separator" />}
          <button
            type="button"
            // Kept together deliberately: `aria-checked` is only meaningful on
            // `menuitemradio`, so the role and the state are one decision, not
            // two attributes that have to be remembered to agree.
            {...(item.checked === undefined
              ? ({ role: 'menuitem' } as const)
              : ({ role: 'menuitemradio', 'aria-checked': item.checked } as const))}
            className="menu__item"
            disabled={item.disabled}
            onClick={() => {
              onDismiss();
              item.onSelect();
            }}
          >
            {item.swatch !== undefined && (
              <span
                className="menu__swatch"
                style={{ background: item.swatch }}
                aria-hidden="true"
              />
            )}
            {item.label}
          </button>
        </Fragment>
      ))}
    </div>
  );
}
