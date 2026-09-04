import { describe, expect, it } from 'vitest';
import {
  boundsWithin,
  clampSize,
  type SplitterBounds,
  sizeFromDelta,
  sizeFromKey,
} from './splitter-geometry';

const bounds: SplitterBounds = { min: 100, max: 400 };

describe('clampSize', () => {
  it('keeps a size inside its bounds, rounded to a whole pixel', () => {
    expect(clampSize(250.4, bounds)).toBe(250);
    expect(clampSize(20, bounds)).toBe(100);
    expect(clampSize(900, bounds)).toBe(400);
  });

  it('resolves to the minimum when the bounds have collapsed', () => {
    // A window too small for both panes. A pane at its minimum is usable; a
    // pane at a negative width is a broken layout.
    expect(clampSize(250, { min: 300, max: 120 })).toBe(300);
  });

  it('never returns NaN', () => {
    expect(clampSize(Number.NaN, bounds)).toBe(100);
    expect(clampSize(Number.POSITIVE_INFINITY, bounds)).toBe(400);
  });
});

describe('sizeFromDelta', () => {
  it('grows with the pointer for a pane that grows with the axis', () => {
    expect(sizeFromDelta(200, 50, 1, bounds)).toBe(250);
    expect(sizeFromDelta(200, -50, 1, bounds)).toBe(150);
  });

  it('grows against the pointer for a pane anchored to the far edge', () => {
    // The dock is at the bottom: dragging up (a negative delta) makes it taller.
    expect(sizeFromDelta(200, -50, -1, bounds)).toBe(250);
    expect(sizeFromDelta(200, 50, -1, bounds)).toBe(150);
  });

  it('stops at the bounds instead of running past them', () => {
    expect(sizeFromDelta(390, 999, 1, bounds)).toBe(400);
    expect(sizeFromDelta(110, -999, 1, bounds)).toBe(100);
  });
});

describe('sizeFromKey', () => {
  it('moves a vertical separator with left and right', () => {
    expect(sizeFromKey('ArrowRight', 200, 16, 1, bounds, 'vertical')).toBe(216);
    expect(sizeFromKey('ArrowLeft', 200, 16, 1, bounds, 'vertical')).toBe(184);
  });

  it('moves a horizontal separator with up and down, respecting direction', () => {
    // The dock grows upwards, so Up makes it bigger.
    expect(sizeFromKey('ArrowUp', 200, 16, -1, bounds, 'horizontal')).toBe(216);
    expect(sizeFromKey('ArrowDown', 200, 16, -1, bounds, 'horizontal')).toBe(184);
  });

  it('jumps to the bounds with Home and End', () => {
    expect(sizeFromKey('Home', 200, 16, 1, bounds, 'vertical')).toBe(100);
    expect(sizeFromKey('End', 200, 16, 1, bounds, 'vertical')).toBe(400);
  });

  it('leaves every other key alone, including the wrong axis', () => {
    expect(sizeFromKey('ArrowUp', 200, 16, 1, bounds, 'vertical')).toBeNull();
    expect(sizeFromKey('ArrowLeft', 200, 16, -1, bounds, 'horizontal')).toBeNull();
    expect(sizeFromKey('Enter', 200, 16, 1, bounds, 'vertical')).toBeNull();
  });
});

describe('boundsWithin', () => {
  it('leaves the neighbour its minimum', () => {
    expect(boundsWithin(1000, 200, 300)).toEqual({ min: 200, max: 700 });
  });

  it('honours a hard maximum when there is room to spare', () => {
    expect(boundsWithin(4000, 200, 300, 600)).toEqual({ min: 200, max: 600 });
  });

  it('collapses to the minimum rather than inverting when the window is small', () => {
    expect(boundsWithin(400, 200, 300)).toEqual({ min: 200, max: 200 });
  });
});
