/** @vitest-environment jsdom */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { Splitter } from './splitter';

/**
 * The close-by-dragging gesture, as a pointer performs it.
 *
 * A DOM rather than `splitter-geometry.test.ts`'s arithmetic, because what is
 * asserted here only exists in one: that the preview is an attribute on the
 * gutter while the pointer is still down, that it goes away again, and that a
 * release — not a cancel — is what closes the pane.
 */

beforeAll(() => {
  // jsdom has neither pointer capture nor `PointerEvent`. A `MouseEvent` with
  // a `pointerId` is everything the splitter reads.
  Element.prototype.setPointerCapture = () => {};
  Element.prototype.hasPointerCapture = () => false;
  Element.prototype.releasePointerCapture = () => {};
  class TestPointerEvent extends MouseEvent {
    readonly pointerId: number;
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 1;
    }
  }
  vi.stubGlobal('PointerEvent', TestPointerEvent);
});

afterEach(cleanup);

function Dock({ onCommit, onCollapse }: { onCommit: () => void; onCollapse?: () => void }) {
  const target = useRef<HTMLDivElement>(null);
  return (
    <div ref={target}>
      <Splitter
        orientation="horizontal"
        label="Resize the terminal panel"
        value={280}
        fallback={280}
        direction={-1}
        target={target}
        property="--terminal-height"
        bounds={() => ({ min: 120, max: 900 })}
        onCommit={onCommit}
        onCollapse={onCollapse}
      />
    </div>
  );
}

// From 280 with a minimum of 120, the close line is 60: 221px down leaves 59.
const PAST_THE_LINE = 221;

function press(gutter: HTMLElement) {
  fireEvent.pointerDown(gutter, { button: 0, pointerId: 1, clientY: 500 });
}

describe('Splitter collapse', () => {
  it('previews the close while dragging and closes on release without resizing', () => {
    const onCommit = vi.fn();
    const onCollapse = vi.fn();
    render(<Dock onCommit={onCommit} onCollapse={onCollapse} />);
    const gutter = screen.getByRole('separator');

    press(gutter);
    fireEvent.pointerMove(gutter, { pointerId: 1, clientY: 500 + PAST_THE_LINE });
    expect(gutter.hasAttribute('data-collapsing')).toBe(true);

    fireEvent.pointerUp(gutter, { pointerId: 1, clientY: 500 + PAST_THE_LINE });
    expect(onCollapse).toHaveBeenCalledTimes(1);
    expect(onCommit).not.toHaveBeenCalled();
    expect(gutter.hasAttribute('data-collapsing')).toBe(false);
    // The height the drag started from is what the pane reopens at.
    expect(gutter.parentElement?.style.getPropertyValue('--terminal-height')).toBe('280px');
  });

  it('reopens the preview when the pointer comes back up, and resizes instead', () => {
    const onCommit = vi.fn();
    const onCollapse = vi.fn();
    render(<Dock onCommit={onCommit} onCollapse={onCollapse} />);
    const gutter = screen.getByRole('separator');

    press(gutter);
    fireEvent.pointerMove(gutter, { pointerId: 1, clientY: 500 + PAST_THE_LINE });
    fireEvent.pointerMove(gutter, { pointerId: 1, clientY: 600 });
    expect(gutter.hasAttribute('data-collapsing')).toBe(false);

    fireEvent.pointerUp(gutter, { pointerId: 1, clientY: 600 });
    expect(onCollapse).not.toHaveBeenCalled();
    expect(onCommit).toHaveBeenCalledWith(180);
  });

  it('stops at the minimum while still short of the line', () => {
    const onCommit = vi.fn();
    const onCollapse = vi.fn();
    render(<Dock onCommit={onCommit} onCollapse={onCollapse} />);
    const gutter = screen.getByRole('separator');

    press(gutter);
    fireEvent.pointerMove(gutter, { pointerId: 1, clientY: 500 + PAST_THE_LINE - 1 });
    expect(gutter.hasAttribute('data-collapsing')).toBe(false);
    fireEvent.pointerUp(gutter, { pointerId: 1, clientY: 500 + PAST_THE_LINE - 1 });
    expect(onCollapse).not.toHaveBeenCalled();
    expect(onCommit).toHaveBeenCalledWith(120);
  });

  it('does not close on a cancelled pointer', () => {
    const onCollapse = vi.fn();
    render(<Dock onCommit={() => {}} onCollapse={onCollapse} />);
    const gutter = screen.getByRole('separator');

    press(gutter);
    fireEvent.pointerMove(gutter, { pointerId: 1, clientY: 500 + PAST_THE_LINE });
    fireEvent.pointerCancel(gutter, { pointerId: 1 });
    expect(onCollapse).not.toHaveBeenCalled();
    expect(gutter.hasAttribute('data-collapsing')).toBe(false);
  });

  it('keeps the minimum a hard stop for a pane that cannot close', () => {
    const onCommit = vi.fn();
    render(<Dock onCommit={onCommit} />);
    const gutter = screen.getByRole('separator');

    press(gutter);
    fireEvent.pointerMove(gutter, { pointerId: 1, clientY: 5000 });
    expect(gutter.hasAttribute('data-collapsing')).toBe(false);
    fireEvent.pointerUp(gutter, { pointerId: 1, clientY: 5000 });
    expect(onCommit).toHaveBeenCalledWith(120);
  });
});
