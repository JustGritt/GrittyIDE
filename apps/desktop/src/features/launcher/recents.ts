import type { RecentWorkspace } from '@gritty/protocol';

/**
 * The launcher's decisions, kept out of the component (T049).
 *
 * Everything here is a pure function of values the core already handed over:
 * no clock of its own (`now` is a parameter), no filesystem, no path API. The
 * core decided what `displayRoot` and `exists` are; this file only decides
 * what to say about them.
 */

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** Rows shown before "View all". Five is what artboard 3a shows. */
export const COLLAPSED_RECENTS = 5;

/**
 * "17 min ago", "yesterday", "3 days ago", and a plain date once relative time
 * stops meaning anything.
 *
 * `now` is passed in rather than read: a formatter that reads the clock cannot
 * be tested, and the caller already knows when it is rendering.
 */
export function formatRelativeTime(then: number, now: number): string {
  const elapsed = now - then;
  // A timestamp in the future is a clock that moved, not a prediction. Say the
  // vaguest true thing rather than "in 3 days".
  if (elapsed < MINUTE) return 'just now';
  if (elapsed < HOUR) {
    const minutes = Math.floor(elapsed / MINUTE);
    return `${minutes} min ago`;
  }
  // Calendar days, not 24-hour blocks: 23:00 seen at 00:30 is "yesterday", and
  // that is what a person means by it.
  const days = calendarDaysBetween(then, now);
  if (days <= 0) {
    const hours = Math.floor(elapsed / HOUR);
    return hours === 1 ? '1 hour ago' : `${hours} hours ago`;
  }
  if (days === 1) return 'yesterday';
  if (days < 30) return `${days} days ago`;
  return new Date(then).toLocaleDateString();
}

/** Whole days between two instants, counted from local midnight. */
function calendarDaysBetween(then: number, now: number): number {
  // Rounded, because a day is 23 or 25 hours long twice a year.
  return Math.round((startOfLocalDay(now) - startOfLocalDay(then)) / DAY);
}

function startOfLocalDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/**
 * The directory a workspace sits in — the right-hand column of a recents row,
 * where the name is already the left-hand one.
 *
 * String work on a path the core produced, not path resolution: the renderer
 * has no path API and never needs one, because nothing is ever opened by this
 * value (`RecentWorkspace.root` is).
 */
export function parentOf(displayRoot: string): string {
  const trimmed = displayRoot.replace(/[/\\]+$/, '');
  const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  if (cut < 0) return trimmed;
  // `/repo` sits in `/`, not in the empty string.
  if (cut === 0) return trimmed.slice(0, 1);
  return trimmed.slice(0, cut);
}

/** The rows to draw: the first few, or all of them once expanded. */
export function visibleRecents(
  items: readonly RecentWorkspace[],
  expanded: boolean,
): readonly RecentWorkspace[] {
  return expanded ? items : items.slice(0, COLLAPSED_RECENTS);
}

/** Whether "View all (n)" has anything to reveal. */
export function hasMore(items: readonly RecentWorkspace[]): boolean {
  return items.length > COLLAPSED_RECENTS;
}

/**
 * The workspace the "Reopen" tile offers: the most recent one still on disk.
 *
 * A missing root is skipped rather than offered and refused — the tile is the
 * fast path, and a fast path that fails is not one. The row for it is still
 * listed below, with Forget.
 */
export function reopenCandidate(items: readonly RecentWorkspace[]): RecentWorkspace | null {
  return items.find((item) => item.exists) ?? null;
}

/**
 * What the "Open path…" field will send, or `null` when there is nothing to
 * send. Deliberately shallow: whether a path is absolute, exists and is a
 * directory is the core's judgment, and it answers with `InvalidInput` or
 * `NotFound`. Duplicating those rules here would only let them disagree.
 */
export function pathToOpen(input: string): string | null {
  const trimmed = input.trim();
  return trimmed === '' ? null : trimmed;
}
