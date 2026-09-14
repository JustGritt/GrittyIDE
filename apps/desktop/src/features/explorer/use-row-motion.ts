import { useCallback, useState } from 'react';
import type { ExplorerRow } from './explorer-reducer';

/**
 * The tree's expand and collapse motion (TREE-DISCLOSURE).
 *
 * The rows are one flat list (`selectRows`), so there is no per-folder
 * container to animate a height on. Instead each render is compared with the
 * last one: rows that are new fade in where they mount, and the rows a
 * collapse removed are kept on screen for a moment as an inert copy that
 * folds away, so what sat below them slides up rather than jumping.
 *
 * Both sides are capped. A folder of thousands animates its first rows — the
 * ones that can be on screen — and the rest appear or vanish as before, so
 * motion never costs more than a fixed number of rows (AGENTS.md rules 8, 10).
 * Motion is skipped entirely when the OS asks for reduced motion, or when the
 * WebView has no Web Animations (jsdom).
 */

/** How many new rows fade in, and how many removed rows a collapse keeps. */
export const MOTION_ROW_LIMIT = 50;
export const ENTER_MS = 120;
export const EXIT_MS = 120;
/** Decelerate: fast off the mark, settling gently. */
export const MOTION_EASING = 'cubic-bezier(0.2, 0, 0, 1)';

/** The rows a collapse removed, drawn after the folder row until they fold away. */
export interface ExitBlock {
  readonly id: number;
  /** The collapsed folder's path; its entry row is the anchor. */
  readonly parent: string;
  readonly rows: readonly ExplorerRow[];
}

export interface RowMotionPlan {
  /** Keys (`rowKey`) of rows that should fade in as they mount. */
  readonly enter: ReadonlySet<string>;
  /** New exit blocks, without ids. */
  readonly exits: readonly Omit<ExitBlock, 'id'>[];
}

export function rowKey(row: ExplorerRow): string {
  return `${row.kind}:${row.node.path}`;
}

function isUnder(path: string, folder: string): boolean {
  return path === folder || path.startsWith(`${folder}/`);
}

/**
 * What moves between two renders of the rows.
 *
 * - **Enter**: every row whose key the previous render did not have — a
 *   folder's children, its `Listing…` row, a page loaded by `Load more`, a
 *   file the watcher reported. Nothing enters on the first listing (no entry
 *   in `prev`): a tree that fades in as a whole when a workspace opens is noise.
 * - **Exit**: only rows removed *because a folder collapsed* — the contiguous
 *   run under a folder whose row went from open to closed. A delete, a rename
 *   or hiding dotfiles removes rows too, and those should simply go.
 */
export function planRowMotion(
  prev: readonly ExplorerRow[],
  next: readonly ExplorerRow[],
  limit = MOTION_ROW_LIMIT,
): RowMotionPlan {
  // The first listing shows the root's `Listing…` row before any entry, so
  // "nothing on screen yet" means no entries, not no rows.
  if (prev === next || !prev.some((row) => row.kind === 'entry')) {
    return { enter: new Set(), exits: [] };
  }

  const prevOpen = new Set<string>();
  for (const row of prev) {
    if (row.kind === 'entry' && row.node.expanded) prevOpen.add(row.node.path);
  }
  const nextKeys = new Set<string>();
  const collapsed = new Set<string>();
  for (const row of next) {
    nextKeys.add(rowKey(row));
    if (row.kind === 'entry' && !row.node.expanded && prevOpen.has(row.node.path)) {
      collapsed.add(row.node.path);
    }
  }

  const prevKeys = new Set(prev.map(rowKey));
  const enter = new Set<string>();
  for (const row of next) {
    if (enter.size >= limit) break;
    const key = rowKey(row);
    if (!prevKeys.has(key)) enter.add(key);
  }

  const exits: Omit<ExitBlock, 'id'>[] = [];
  if (collapsed.size > 0) {
    for (let i = 0; i < prev.length; i += 1) {
      const row = prev[i] as ExplorerRow;
      if (row.kind !== 'entry' || !collapsed.has(row.node.path)) continue;
      const parent = row.node.path;
      const removed: ExplorerRow[] = [];
      let j = i + 1;
      for (; j < prev.length; j += 1) {
        const child = prev[j] as ExplorerRow;
        if (!isUnder(child.node.path, parent)) break;
        if (removed.length < limit && !nextKeys.has(rowKey(child))) removed.push(child);
      }
      if (removed.length > 0) exits.push({ parent, rows: removed });
      // Skip the run: a folder nested inside it cannot anchor a block of its own.
      i = j - 1;
    }
  }
  return { enter, exits };
}

/**
 * Whether this WebView can animate and the user has not asked it not to.
 * Asked each time the rows change, so flipping the OS setting takes effect on
 * the next expand without a listener.
 */
export function motionAllowed(): boolean {
  if (typeof window === 'undefined') return false;
  if (typeof HTMLElement.prototype.animate !== 'function') return false;
  if (typeof window.matchMedia !== 'function') return true;
  return !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** Ref callback for a row mounting with `enter`: fade it in and let it settle. */
export function animateRowIn(element: HTMLElement | null): void {
  element?.animate(
    [
      { opacity: 0, transform: 'translateY(-3px)' },
      { opacity: 1, transform: 'none' },
    ],
    { duration: ENTER_MS, easing: MOTION_EASING },
  );
}

/**
 * The plan for the rows being rendered, kept as state derived from the
 * previous rows (React's "storing information from previous renders"), so it
 * is computed once per change of `rows` and survives Strict Mode's double
 * render.
 *
 * `enter` only matters at the moment a row mounts; it is harmless that it
 * lingers until the rows change again.
 */
export function useRowMotion(rows: readonly ExplorerRow[]): {
  enter: ReadonlySet<string>;
  exits: readonly ExitBlock[];
  finishExit: (id: number) => void;
} {
  const [state, setState] = useState<{
    rows: readonly ExplorerRow[];
    enter: ReadonlySet<string>;
    exits: readonly ExitBlock[];
    nextId: number;
  }>(() => ({ rows, enter: new Set(), exits: [], nextId: 1 }));

  if (state.rows !== rows) {
    const plan = motionAllowed() ? planRowMotion(state.rows, rows) : null;
    const anchors = new Set<string>();
    for (const row of rows) {
      // A block stays only while its folder row is on screen and still closed:
      // reopening it mid-fold must not draw the children twice.
      if (row.kind === 'entry' && !row.node.expanded) anchors.add(row.node.path);
    }
    let nextId = state.nextId;
    const exits = [
      ...state.exits.filter((block) => anchors.has(block.parent)),
      ...(plan?.exits ?? []).map((block) => ({ ...block, id: nextId++ })),
    ];
    setState({ rows, enter: plan?.enter ?? new Set(), exits, nextId });
  }

  const finishExit = useCallback((id: number) => {
    setState((current) =>
      current.exits.some((block) => block.id === id)
        ? { ...current, exits: current.exits.filter((block) => block.id !== id) }
        : current,
    );
  }, []);

  return { enter: state.enter, exits: state.exits, finishExit };
}
