import { describe, expect, it } from 'vitest';
import { previewScrollTop } from './scroll-sync';

const anchors = [
  { line: 0, top: 0 },
  { line: 10, top: 200 },
  { line: 20, top: 500 },
];

describe('previewScrollTop', () => {
  it('interpolates between the anchors around the first visible line', () => {
    expect(previewScrollTop(anchors, 1, false, 2000, 400)).toBe(0);
    expect(previewScrollTop(anchors, 6, false, 2000, 400)).toBe(100);
    expect(previewScrollTop(anchors, 11, false, 2000, 400)).toBe(200);
    expect(previewScrollTop(anchors, 16, false, 2000, 400)).toBe(350);
  });

  it('holds the last anchor past the end and reaches the bottom when the editor does', () => {
    expect(previewScrollTop(anchors, 40, false, 2000, 400)).toBe(500);
    expect(previewScrollTop(anchors, 40, true, 2000, 400)).toBe(1600);
    expect(previewScrollTop([], 5, false, 300, 400)).toBe(0);
    expect(previewScrollTop([], 5, true, 300, 400)).toBe(0);
  });

  it('never returns a negative offset', () => {
    expect(previewScrollTop([{ line: 3, top: -20 }], 4, false, 100, 50)).toBe(0);
  });
});
