import { describe, expect, it } from 'vitest';
import { viewStateFrom } from './view-state';

describe('viewStateFrom', () => {
  it('needs at least a cursor and fills the rest from it', () => {
    expect(viewStateFrom({})).toBeNull();
    expect(viewStateFrom({ anchor: { line: 1, column: 1 }, scrollLine: 3 })).toBeNull();
    expect(viewStateFrom({ cursor: { line: 4, column: 2 } })).toEqual({
      cursor: { line: 4, column: 2 },
      anchor: { line: 4, column: 2 },
      scrollLine: 4,
    });
  });

  it('keeps every valid part and rejects malformed ones', () => {
    expect(
      viewStateFrom({
        cursor: { line: 4, column: 2 },
        anchor: { line: 1, column: 1 },
        scrollLine: 2,
      }),
    ).toEqual({ cursor: { line: 4, column: 2 }, anchor: { line: 1, column: 1 }, scrollLine: 2 });
    expect(viewStateFrom({ cursor: { line: 0, column: 2 } })).toBeNull();
    expect(viewStateFrom({ cursor: { line: 2.5, column: 1 } })).toBeNull();
    expect(viewStateFrom({ cursor: { line: 2, column: 1 }, anchor: 'x', scrollLine: -1 })).toEqual({
      cursor: { line: 2, column: 1 },
      anchor: { line: 2, column: 1 },
      scrollLine: 2,
    });
  });
});
