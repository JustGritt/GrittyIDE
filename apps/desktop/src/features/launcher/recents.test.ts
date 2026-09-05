import type { RecentWorkspace } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  COLLAPSED_RECENTS,
  describeLastSession,
  formatRelativeTime,
  hasMore,
  parentOf,
  pathToOpen,
  reopenCandidate,
  visibleRecents,
} from './recents';

function entry(overrides: Partial<RecentWorkspace> = {}): RecentWorkspace {
  return {
    root: '/home/me/ws',
    displayRoot: '~/ws',
    name: 'ws',
    lastOpenedAt: 0,
    exists: true,
    ...overrides,
  };
}

/** A fixed local noon, so day arithmetic never straddles a boundary by accident. */
const NOON = new Date(2026, 8, 4, 12, 0, 0).getTime();

describe('formatRelativeTime', () => {
  it('counts minutes, then hours, then calendar days', () => {
    expect(formatRelativeTime(NOON - 5_000, NOON)).toBe('just now');
    expect(formatRelativeTime(NOON - 17 * 60_000, NOON)).toBe('17 min ago');
    expect(formatRelativeTime(NOON - 60 * 60_000, NOON)).toBe('1 hour ago');
    expect(formatRelativeTime(NOON - 5 * 60 * 60_000, NOON)).toBe('5 hours ago');
    expect(formatRelativeTime(new Date(2026, 8, 3, 23, 0, 0).getTime(), NOON)).toBe('yesterday');
    expect(formatRelativeTime(new Date(2026, 8, 1, 9, 0, 0).getTime(), NOON)).toBe('3 days ago');
  });

  it('says "yesterday" by the calendar, not by 24 hours', () => {
    // 90 minutes earlier, but on the previous day: a person calls that
    // yesterday, and an "2 hours ago" that crosses midnight reads as today.
    const justBeforeMidnight = new Date(2026, 8, 3, 23, 0, 0).getTime();
    const justAfter = new Date(2026, 8, 4, 0, 30, 0).getTime();
    expect(formatRelativeTime(justBeforeMidnight, justAfter)).toBe('yesterday');
  });

  it('falls back to a date once relative time stops meaning anything', () => {
    const longAgo = new Date(2026, 5, 1, 9, 0, 0).getTime();
    expect(formatRelativeTime(longAgo, NOON)).toBe(new Date(longAgo).toLocaleDateString());
  });

  it('never reports a future timestamp as a prediction', () => {
    // A clock that moved backwards, or a database written by a machine ahead
    // of this one. "in 3 days" would be nonsense on a list of things you did.
    expect(formatRelativeTime(NOON + 3 * 24 * 60 * 60_000, NOON)).toBe('just now');
  });
});

describe('parentOf', () => {
  it('drops the last component, on either separator', () => {
    expect(parentOf('~/Desktop/CurrentProjects/GrittyIDE')).toBe('~/Desktop/CurrentProjects');
    expect(parentOf('C:\\Users\\me\\code\\gritty')).toBe('C:\\Users\\me\\code');
    expect(parentOf('~/code/')).toBe('~');
  });

  it('keeps a root a root rather than emptying it', () => {
    expect(parentOf('/repo')).toBe('/');
    expect(parentOf('~')).toBe('~');
  });
});

describe('the recents list', () => {
  const many = Array.from({ length: 8 }, (_, index) =>
    entry({ root: `/ws-${index}`, name: `ws-${index}` }),
  );

  it('shows a few until it is expanded', () => {
    expect(visibleRecents(many, false)).toHaveLength(COLLAPSED_RECENTS);
    expect(visibleRecents(many, true)).toHaveLength(8);
    expect(hasMore(many)).toBe(true);
    expect(hasMore(many.slice(0, COLLAPSED_RECENTS))).toBe(false);
  });

  it('offers the most recent workspace that is still on disk', () => {
    const withMissing = [
      entry({ root: '/gone', name: 'gone', exists: false }),
      entry({ root: '/here', name: 'here', exists: true }),
    ];
    expect(reopenCandidate(withMissing)?.root).toBe('/here');
    expect(reopenCandidate([entry({ exists: false })])).toBeNull();
    expect(reopenCandidate([])).toBeNull();
  });
});

describe('pathToOpen', () => {
  it('sends a trimmed path and nothing else', () => {
    expect(pathToOpen('  /home/me/code  ')).toBe('/home/me/code');
    expect(pathToOpen('')).toBeNull();
    expect(pathToOpen('   ')).toBeNull();
  });

  it('leaves judgment about the path to the core', () => {
    // Relative, nonexistent and not-a-directory all reach the core and come
    // back as InvalidInput or NotFound. Guessing here would only let the two
    // rulesets disagree.
    expect(pathToOpen('../secrets')).toBe('../secrets');
  });
});

describe('describing the last session', () => {
  it('counts tabs and names the unsaved ones', () => {
    expect(describeLastSession(4, 0)).toBe('4 tabs were open when the window closed.');
    expect(describeLastSession(1, 0)).toBe('1 tab was open when the window closed.');
    expect(describeLastSession(4, 1)).toBe(
      '4 tabs were open when the window closed, and one had unsaved edits.',
    );
    expect(describeLastSession(4, 3)).toBe(
      '4 tabs were open when the window closed, and 3 of them had unsaved edits.',
    );
  });
});
