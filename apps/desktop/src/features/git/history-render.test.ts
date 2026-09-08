import type { CommitPage, CommitSummary, CoreErrorShape, OperationId } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { commitAge, commitByline, isMerge } from './git-model';
import type { Read } from './git-reducer';
import { HistoryView } from './history-view';

/**
 * What the History half of the git panel puts on the screen (T101-U).
 *
 * Static markup, matching `changes-view-render.test.ts` next door and for its
 * reason: `apps/desktop`'s vitest project is a node one, and a view model with
 * no caller is the defect this repository keeps catching late. Every arm of the
 * read is mounted here, including the three the happy path never reaches.
 */

const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

function commit(overrides: Partial<CommitSummary> = {}): CommitSummary {
  const time = Math.floor(Date.now() / 1000) - 3600;
  return {
    id: 'a'.repeat(40),
    shortId: 'aaaaaaa',
    parents: ['b'.repeat(40)],
    author: { name: 'Ada', email: 'ada@example.com', time, offsetSeconds: 0 },
    committer: { name: 'Ada', email: 'ada@example.com', time, offsetSeconds: 0 },
    subject: 'the change',
    ...overrides,
  };
}

function page(overrides: Partial<CommitPage> = {}): CommitPage {
  return {
    start: 'HEAD',
    resolved: 'a'.repeat(40),
    commits: [commit()],
    truncated: false,
    ...overrides,
  };
}

function render(state: Read<CommitPage>, appending = false): string {
  return renderToStaticMarkup(
    createElement(HistoryView, {
      state,
      appending,
      onLoadMore: refuse('page the history'),
      onRetry: refuse('retry'),
      onCancel: refuse('cancel'),
    }),
  );
}

describe('history view', () => {
  it('lists commits with their subject, short id and byline', () => {
    const markup = render({ kind: 'ready', value: page() });
    expect(markup).toContain('the change');
    expect(markup).toContain('aaaaaaa');
    expect(markup).toContain('Ada');
    // The full id is reachable without leaving the panel.
    expect(markup).toContain('a'.repeat(40));
  });

  it('marks a merge, and leaves an ordinary commit unmarked', () => {
    const merge = render({
      kind: 'ready',
      value: page({ commits: [commit({ parents: ['b'.repeat(40), 'c'.repeat(40)] })] }),
    });
    expect(merge).toContain('merge');

    expect(render({ kind: 'ready', value: page() })).not.toContain('>merge<');
  });

  it('offers another page only when one follows', () => {
    expect(render({ kind: 'ready', value: page({ truncated: true }) })).toContain('Load more');
    expect(render({ kind: 'ready', value: page() })).not.toContain('Load more');
  });

  it('says a page is on its way rather than offering to ask twice', () => {
    const markup = render({ kind: 'ready', value: page({ truncated: true }) }, true);
    expect(markup).toContain('Loading…');
    expect(markup).toContain('disabled');
  });

  // An unborn branch. The repository exists and has no commits yet, which is a
  // different sentence from a failure and must not render as one.
  it('reports a repository with no commits as a fact, not an error', () => {
    const markup = render({ kind: 'ready', value: page({ commits: [], resolved: undefined }) });
    expect(markup).toContain('No commits yet.');
  });

  it('reports a folder that is not a repository as a value', () => {
    expect(render({ kind: 'notARepository' })).toContain('not a Git repository');
  });

  // The arm the operation model exists for: a walk that outlived its budget is
  // still going, and the user must be able to stop it.
  it('offers to stop a read that became an operation', () => {
    const markup = render({ kind: 'pending', operationId: 'op-1' as OperationId });
    expect(markup).toContain('operation');
    expect(markup).toContain('Stop');
  });

  it('shows the envelope of a refused read, with a retry', () => {
    const error: CoreErrorShape = {
      code: 'Io',
      message: 'could not read the object database',
      retryable: true,
    };
    const markup = render({ kind: 'failed', error });
    expect(markup).toContain('could not read the object database');
    expect(markup).toContain('Try again');
  });

  it('says a cancelled read was cancelled, and offers to ask again', () => {
    const markup = render({ kind: 'cancelled' });
    expect(markup).toContain('cancelled');
    expect(markup).toContain('Read again');
  });

  // Repository content reaches a `title`, so it must be escaped rather than
  // interpreted — the same assertion the Changes tab makes.
  it('escapes a commit subject that looks like markup', () => {
    const markup = render({
      kind: 'ready',
      value: page({ commits: [commit({ subject: '<script>alert(1)</script>' })] }),
    });
    expect(markup).not.toContain('<script>');
    expect(markup).toContain('&lt;script&gt;');
  });
});

describe('commit presentation', () => {
  const now = Date.UTC(2026, 8, 8, 12, 0, 0);
  const at = (secondsAgo: number) => Math.floor(now / 1000) - secondsAgo;

  it('reads recent commits as elapsed time and older ones as a date', () => {
    expect(commitAge(at(30), now)).toBe('just now');
    expect(commitAge(at(60), now)).toBe('1 minute ago');
    expect(commitAge(at(120), now)).toBe('2 minutes ago');
    expect(commitAge(at(3600), now)).toBe('1 hour ago');
    expect(commitAge(at(86_400), now)).toBe('1 day ago');
    expect(commitAge(at(6 * 86_400), now)).toBe('6 days ago');
    // Past a week the relative form stops helping: "37 days ago" is arithmetic
    // the reader has to undo, so it becomes a date.
    expect(commitAge(at(30 * 86_400), now)).not.toContain('ago');
  });

  it('credits the author rather than the committer', () => {
    const rebased = commit({
      author: { name: 'Ada', email: 'ada@example.com', time: at(3600), offsetSeconds: 0 },
      committer: {
        name: 'Someone Else',
        email: 'else@example.com',
        time: at(60),
        offsetSeconds: 0,
      },
    });
    // The two differ after a rebase or a cherry-pick, and `git log` shows the
    // author — showing the committer would credit the wrong person.
    expect(commitByline(rebased, now)).toContain('Ada');
    expect(commitByline(rebased, now)).not.toContain('Someone Else');
  });

  it('derives merge-ness from the parent list rather than a second field', () => {
    expect(isMerge(commit({ parents: [] }))).toBe(false);
    expect(isMerge(commit({ parents: ['b'.repeat(40)] }))).toBe(false);
    expect(isMerge(commit({ parents: ['b'.repeat(40), 'c'.repeat(40)] }))).toBe(true);
  });
});
