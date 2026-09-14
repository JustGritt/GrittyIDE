import type { DirEntry } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import type { ExplorerNode, ExplorerRow } from './explorer-reducer';
import { planRowMotion, rowKey } from './use-row-motion';

/**
 * Which rows move between two renders (TREE-DISCLOSURE). The motion itself can
 * only be judged on a screen; what is decided here — which rows, how many, and
 * never on a delete — can be pinned down.
 */

function node(path: string, overrides: Partial<ExplorerNode> = {}): ExplorerNode {
  return {
    path,
    entry: null,
    expanded: false,
    loading: false,
    pending: null,
    children: null,
    total: null,
    truncated: false,
    cursor: null,
    error: null,
    ...overrides,
  };
}

function entry(path: string, depth: number, opts: { dir?: boolean; open?: boolean } = {}) {
  const name = path.slice(path.lastIndexOf('/') + 1);
  const dirEntry: DirEntry = {
    name,
    kind: opts.dir ? 'dir' : 'file',
    hidden: false,
    ignoredHint: false,
  };
  return {
    kind: 'entry',
    node: node(path, { entry: dirEntry, expanded: opts.open ?? false }),
    entry: dirEntry,
    depth,
  } satisfies ExplorerRow;
}

function status(path: string, depth: number): ExplorerRow {
  return { kind: 'status', node: node(path, { loading: true }), depth };
}

const keys = (rows: readonly ExplorerRow[]) => rows.map(rowKey);

describe('entering rows', () => {
  it("fades in a folder's children, and nothing that was already there", () => {
    const before = [entry('src', 0, { dir: true }), entry('README.md', 0)];
    const after = [
      entry('src', 0, { dir: true, open: true }),
      entry('src/a.ts', 1),
      entry('src/b.ts', 1),
      entry('README.md', 0),
    ];
    const plan = planRowMotion(before, after);
    expect([...plan.enter]).toEqual(['entry:src/a.ts', 'entry:src/b.ts']);
    expect(plan.exits).toEqual([]);
  });

  it('fades in the Listing… row, then the children that replace it', () => {
    const closed = [entry('src', 0, { dir: true })];
    const listing = [entry('src', 0, { dir: true, open: true }), status('src', 1)];
    const listed = [entry('src', 0, { dir: true, open: true }), entry('src/a.ts', 1)];
    expect([...planRowMotion(closed, listing).enter]).toEqual(['status:src']);
    const arrived = planRowMotion(listing, listed);
    expect([...arrived.enter]).toEqual(['entry:src/a.ts']);
    // The Listing… row going is not a collapse: it simply goes.
    expect(arrived.exits).toEqual([]);
  });

  it("does not fade in the first listing of a workspace, after the root's Listing… row either", () => {
    const listed = [entry('src', 0, { dir: true }), entry('README.md', 0)];
    expect(planRowMotion([], listed).enter.size).toBe(0);
    // The DOM test caught this one: the root shows `Listing…` first.
    expect(planRowMotion([status('', 0)], listed).enter.size).toBe(0);
  });

  it('animates at most the limit, the first rows, however big the folder', () => {
    const children = Array.from({ length: 5000 }, (_, i) => entry(`big/f${i}`, 1));
    const plan = planRowMotion(
      [entry('big', 0, { dir: true })],
      [entry('big', 0, { dir: true, open: true }), ...children],
      50,
    );
    expect(plan.enter.size).toBe(50);
    expect(plan.enter.has('entry:big/f0')).toBe(true);
    expect(plan.enter.has('entry:big/f50')).toBe(false);
  });
});

describe('exiting rows', () => {
  it('keeps the rows a collapse removed, nested folders included, anchored to the folder', () => {
    const before = [
      entry('src', 0, { dir: true, open: true }),
      entry('src/lib', 1, { dir: true, open: true }),
      entry('src/lib/x.ts', 2),
      entry('src/a.ts', 1),
      status('src', 1),
      entry('README.md', 0),
    ];
    const after = [entry('src', 0, { dir: true }), entry('README.md', 0)];
    const plan = planRowMotion(before, after);
    expect(plan.exits).toHaveLength(1);
    expect(plan.exits[0]?.parent).toBe('src');
    expect(keys(plan.exits[0]?.rows ?? [])).toEqual([
      'entry:src/lib',
      'entry:src/lib/x.ts',
      'entry:src/a.ts',
      'status:src',
    ]);
    expect(plan.enter.size).toBe(0);
  });

  it('does not treat a sibling that shares a name prefix as a child', () => {
    const before = [
      entry('src', 0, { dir: true, open: true }),
      entry('src/a.ts', 1),
      entry('src-old', 0, { dir: true }),
    ];
    const after = [entry('src', 0, { dir: true }), entry('src-old', 0, { dir: true })];
    expect(keys(planRowMotion(before, after).exits[0]?.rows ?? [])).toEqual(['entry:src/a.ts']);
  });

  it('keeps at most the limit of a huge folder', () => {
    const children = Array.from({ length: 5000 }, (_, i) => entry(`big/f${i}`, 1));
    const plan = planRowMotion(
      [entry('big', 0, { dir: true, open: true }), ...children],
      [entry('big', 0, { dir: true })],
      50,
    );
    expect(plan.exits[0]?.rows).toHaveLength(50);
  });

  it('lets a delete, a rename or hidden files simply go', () => {
    const before = [
      entry('src', 0, { dir: true, open: true }),
      entry('src/a.ts', 1),
      entry('src/b.ts', 1),
      entry('.env', 0),
    ];
    // `a.ts` renamed to `c.ts`, `.env` hidden; `src` is still open.
    const after = [
      entry('src', 0, { dir: true, open: true }),
      entry('src/b.ts', 1),
      entry('src/c.ts', 1),
    ];
    const plan = planRowMotion(before, after);
    expect(plan.exits).toEqual([]);
    expect([...plan.enter]).toEqual(['entry:src/c.ts']);
  });

  it('plans nothing when the rows did not change', () => {
    const rows = [entry('src', 0, { dir: true, open: true }), entry('src/a.ts', 1)];
    const plan = planRowMotion(rows, rows);
    expect(plan.enter.size).toBe(0);
    expect(plan.exits).toEqual([]);
  });
});
