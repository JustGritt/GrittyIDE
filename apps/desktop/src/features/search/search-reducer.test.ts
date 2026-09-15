import type {
  CoreErrorShape,
  EventEnvelope,
  Operation,
  OperationId,
  SearchCompletion,
  SearchMatch,
  SearchSummary,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  HELD_EVENT_LIMIT,
  initialSearchState,
  isActive,
  SEARCH_KIND,
  type SearchState,
  searchReducer,
  undeliveredOf,
} from './search-reducer';

/**
 * The search panel's state machine (T080-U), every arm without a transport.
 *
 * The cases that matter most are the ones a happy path never reaches: events
 * that beat the command's response (ADR-009), a hold that overflows, and a
 * search that was superseded while it was still streaming.
 */

const WS = 'ws-1' as WorkspaceId;
const OP = 'op-1' as OperationId;
const OTHER = 'op-0' as OperationId;

function content(path: string, line: number, overrides: Partial<SearchMatch> = {}): SearchMatch {
  return {
    path: path as WorkspacePath,
    kind: 'content',
    line,
    column: 5,
    snippet: `    needle on line ${line}`,
    truncated: false,
    matchStart: 4,
    matchLength: 6,
    ...overrides,
  };
}

function name(path: string): SearchMatch {
  return {
    path: path as WorkspacePath,
    kind: 'filename',
    snippet: path,
    truncated: false,
    matchStart: 0,
    matchLength: path.length,
  };
}

function operation(id: OperationId, kind = SEARCH_KIND): Operation {
  return {
    id,
    kind,
    actor: { type: 'USER' },
    workspaceId: WS,
    status: 'RUNNING',
    createdAt: 1,
    cancellationRequested: false,
    correlationId: 'corr' as Operation['correlationId'],
  };
}

function event(body: Omit<EventEnvelope, 'seq' | 'actor'>): EventEnvelope {
  return { seq: 1, actor: { type: 'SYSTEM' }, ...body } as EventEnvelope;
}

function batch(id: OperationId, matches: SearchMatch[], truncatedMatches = 0): EventEnvelope {
  return event({
    name: 'SearchMatchesFound',
    payload: { operationId: id, matches, truncatedMatches },
  });
}

function progress(id: OperationId, current: number): EventEnvelope {
  return event({ name: 'OperationProgress', payload: { operationId: id, progress: { current } } });
}

function summary(overrides: Partial<SearchSummary> = {}): SearchSummary {
  return {
    filesScanned: 12,
    filesMatched: 2,
    directoriesScanned: 3,
    bytesScanned: 4096,
    matchesFound: 3,
    snippetsTruncated: 0,
    skipped: {
      binary: 0,
      invalidUtf8: 0,
      tooLarge: 0,
      unreadable: 0,
      outsideRoot: 0,
      ignoredDirectories: 0,
    },
    limitReached: false,
    elapsedMs: 9,
    ...overrides,
  };
}

function completed(id: OperationId, result: SearchCompletion, kind = SEARCH_KIND): EventEnvelope {
  return event({
    name: 'OperationCompleted',
    payload: { operation: operation(id, kind), result: result as never },
  });
}

function cancelled(id: OperationId): EventEnvelope {
  return event({ name: 'OperationCancelled', payload: { operation: operation(id) } });
}

const BOOM: CoreErrorShape = { code: 'Io', message: 'the root went away', retryable: false };

function failed(id: OperationId): EventEnvelope {
  return event({ name: 'OperationFailed', payload: { operation: operation(id), error: BOOM } });
}

function apply(state: SearchState, ...events: EventEnvelope[]): SearchState {
  return events.reduce(
    (current, next) => searchReducer(current, { type: 'event', event: next }),
    state,
  );
}

function starting(token = 1): SearchState {
  return searchReducer(initialSearchState, {
    type: 'start',
    token,
    query: 'needle',
    options: { caseSensitive: false, wholeWord: false, useRegex: false, scope: 'both' },
  });
}

function running(token = 1, id = OP): SearchState {
  return searchReducer(starting(token), { type: 'started', token, operationId: id });
}

describe('a run from start to finish', () => {
  it('starts with nothing found and nothing known', () => {
    const run = starting().run;
    expect(run?.phase).toBe('starting');
    expect(run?.operationId).toBeNull();
    expect(run?.files).toEqual([]);
    expect(run && isActive(run)).toBe(true);
  });

  it('folds batches by file, in the order files first matched', () => {
    const state = apply(
      running(),
      batch(OP, [content('src/a.ts', 3), content('src/b.ts', 1)]),
      batch(OP, [content('src/a.ts', 9), name('src/needle.ts')]),
    );
    const run = state.run;
    expect(run?.files.map((file) => file.path)).toEqual(['src/a.ts', 'src/b.ts', 'src/needle.ts']);
    expect(run?.files[0]?.lines.map((match) => match.line)).toEqual([3, 9]);
    expect(run?.files[2]?.nameMatch?.kind).toBe('filename');
    expect(run?.files[2]?.lines).toEqual([]);
    expect(run?.matchCount).toBe(4);
  });

  it('keeps a file name match and its content matches in one group', () => {
    const run = apply(running(), batch(OP, [name('src/a.ts'), content('src/a.ts', 2)])).run;
    expect(run?.files).toHaveLength(1);
    expect(run?.files[0]?.nameMatch).not.toBeNull();
    expect(run?.files[0]?.lines).toHaveLength(1);
  });

  it('records the walk count from progress', () => {
    expect(apply(running(), progress(OP, 40)).run?.filesScanned).toBe(40);
  });

  it('completes with the summary, and the completion total replaces the running sum', () => {
    const state = apply(
      running(),
      batch(OP, [content('src/a.ts', 1)], 2),
      batch(OP, [content('src/a.ts', 2)], 3),
    );
    expect(state.run?.coreUndelivered).toBe(5);
    const done = apply(state, completed(OP, { summary: summary(), truncatedMatches: 7 })).run;
    expect(done?.phase).toBe('completed');
    expect(done?.summary?.filesScanned).toBe(12);
    expect(done?.filesScanned).toBe(12);
    expect(done?.coreUndelivered).toBe(7);
    expect(done && isActive(done)).toBe(false);
  });

  it('keeps what was found when it is cancelled', () => {
    const run = apply(running(), batch(OP, [content('src/a.ts', 1)]), cancelled(OP)).run;
    expect(run?.phase).toBe('cancelled');
    expect(run?.files).toHaveLength(1);
  });

  it('keeps what was found when it fails, and carries the error', () => {
    const run = apply(running(), batch(OP, [content('src/a.ts', 1)]), failed(OP)).run;
    expect(run?.phase).toBe('failed');
    expect(run?.error).toEqual(BOOM);
    expect(run?.files).toHaveLength(1);
  });

  it('ignores everything once it has ended', () => {
    const done = apply(running(), cancelled(OP));
    expect(apply(done, batch(OP, [content('src/late.ts', 1)]), progress(OP, 99))).toBe(done);
  });

  it('clears', () => {
    expect(searchReducer(running(), { type: 'clear' })).toEqual(initialSearchState);
    expect(searchReducer(initialSearchState, { type: 'clear' })).toBe(initialSearchState);
  });
});

describe('another operation', () => {
  it('is not this run', () => {
    const state = running();
    expect(apply(state, batch(OTHER, [content('src/x.ts', 1)]), progress(OTHER, 3))).toBe(state);
  });

  it('ending does not end this run, even with the same id shape', () => {
    const state = running();
    // Another kind with this id is not a search, whatever its id says.
    const other = completed(OP, { summary: summary(), truncatedMatches: 0 }, 'workspace.list');
    expect(apply(state, other)).toBe(state);
  });
});

describe('the command answer, and events that beat it (ADR-009)', () => {
  it('holds events that arrive before the response and folds its own on start', () => {
    const early = apply(
      starting(),
      batch(OP, [content('src/a.ts', 1)]),
      batch(OTHER, [content('src/stale.ts', 1)]),
      progress(OP, 5),
    );
    expect(early.run?.files).toEqual([]);
    expect(early.run?.held).toHaveLength(3);
    const run = searchReducer(early, { type: 'started', token: 1, operationId: OP }).run;
    expect(run?.phase).toBe('running');
    expect(run?.held).toEqual([]);
    expect(run?.files.map((file) => file.path)).toEqual(['src/a.ts']);
    expect(run?.filesScanned).toBe(5);
  });

  it('finishes at once when the whole search beat the response', () => {
    const early = apply(
      starting(),
      batch(OP, [content('src/a.ts', 1)]),
      completed(OP, { summary: summary(), truncatedMatches: 0 }),
    );
    const run = searchReducer(early, { type: 'started', token: 1, operationId: OP }).run;
    expect(run?.phase).toBe('completed');
    expect(run?.matchCount).toBe(1);
  });

  it('keeps only the latest progress per operation in the hold', () => {
    const early = apply(starting(), progress(OTHER, 1), progress(OTHER, 2), progress(OP, 3));
    expect(early.run?.held).toHaveLength(2);
  });

  it('bounds the hold and counts the matches it pushed out as undelivered', () => {
    const batches = Array.from({ length: HELD_EVENT_LIMIT + 3 }, (_, index) =>
      batch(OP, [content('src/a.ts', index + 1), content('src/a.ts', index + 1000)], 1),
    );
    const early = apply(starting(), ...batches);
    expect(early.run?.held).toHaveLength(HELD_EVENT_LIMIT);
    const run = searchReducer(early, { type: 'started', token: 1, operationId: OP }).run;
    // Three batches of two matches, each also reporting one the core dropped.
    expect(run?.heldUndelivered).toBe(9);
    expect(run?.matchCount).toBe(HELD_EVENT_LIMIT * 2);
    expect(run && undeliveredOf(run)).toBe(9 + HELD_EVENT_LIMIT);
  });

  it('never pushes a terminal event out of the hold', () => {
    const early = apply(
      starting(),
      completed(OP, { summary: summary(), truncatedMatches: 0 }),
      ...Array.from({ length: HELD_EVENT_LIMIT + 5 }, (_, index) =>
        batch(OTHER, [content('src/other.ts', index + 1)]),
      ),
    );
    const run = searchReducer(early, { type: 'started', token: 1, operationId: OP }).run;
    expect(run?.phase).toBe('completed');
  });

  it('does not charge this run for matches dropped from another operation', () => {
    const early = apply(
      starting(),
      ...Array.from({ length: HELD_EVENT_LIMIT + 2 }, (_, index) =>
        batch(OTHER, [content('src/other.ts', index + 1)]),
      ),
    );
    const run = searchReducer(early, { type: 'started', token: 1, operationId: OP }).run;
    expect(run?.heldUndelivered).toBe(0);
  });

  it('drops an answer for a superseded token', () => {
    const state = starting(2);
    expect(searchReducer(state, { type: 'started', token: 1, operationId: OTHER })).toBe(state);
    expect(searchReducer(state, { type: 'rejected', token: 1, error: BOOM })).toBe(state);
  });

  it('fails a refused command with no operation, which is how an invalid query reads', () => {
    const invalid: CoreErrorShape = {
      code: 'InvalidInput',
      message: 'regex parse error: unclosed group',
      retryable: false,
    };
    const run = searchReducer(starting(), { type: 'rejected', token: 1, error: invalid }).run;
    expect(run?.phase).toBe('failed');
    expect(run?.operationId).toBeNull();
    expect(run?.error?.code).toBe('InvalidInput');
  });

  it('ignores a second start answer', () => {
    const state = running();
    expect(searchReducer(state, { type: 'started', token: 1, operationId: OTHER })).toBe(state);
  });
});

describe('stop', () => {
  it('is recorded while the run is active, and the phase waits for the event', () => {
    const run = searchReducer(running(), { type: 'stop' }).run;
    expect(run?.stopRequested).toBe(true);
    expect(run?.phase).toBe('running');
  });

  it('can be pressed before the response', () => {
    expect(searchReducer(starting(), { type: 'stop' }).run?.stopRequested).toBe(true);
  });

  it('does nothing to a run that has ended, or twice', () => {
    const done = apply(running(), cancelled(OP));
    expect(searchReducer(done, { type: 'stop' })).toBe(done);
    const once = searchReducer(running(), { type: 'stop' });
    expect(searchReducer(once, { type: 'stop' })).toBe(once);
  });
});
