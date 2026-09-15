/**
 * The search panel's state machine (T080-U): one run at a time, folded from
 * the command's answer and the events that stream behind it.
 *
 * Pure, so every arm is tested without a transport. `use-search.ts` owns the
 * side effects — sending `search.query`, cancelling — and this file owns what
 * the panel believes about the run it is showing.
 *
 * Three facts shape it:
 *
 * - **A run is named by a token before it has an operation id.** `search.query`
 *   answers with the operation, and until then the only thing that tells this
 *   run from the one it superseded is the number the controller gave it. A
 *   `started` or `rejected` for any other token is an answer to a question
 *   nobody is asking any more.
 * - **Events can arrive before the answer** (ADR-009 orders a command against
 *   the listener registration, not against the events that follow its
 *   response). A fast search can stream a batch — or finish — while its
 *   response is still crossing the IPC, so a `starting` run holds what it sees
 *   and folds the part that turns out to be its own. The hold is bounded (rule
 *   8), and a batch pushed out of it is counted as undelivered rather than
 *   quietly lost.
 * - **Undelivered is two numbers.** The core counts matches it dropped
 *   (`truncatedMatches`); this side counts the ones its own hold dropped. The
 *   panel shows the sum, because both mean the list is not complete.
 */

import type {
  CoreErrorShape,
  EventEnvelope,
  OperationId,
  SearchCompletion,
  SearchMatch,
  SearchOptions,
  SearchSummary,
  WorkspacePath,
} from '@gritty/protocol';

/** `SEARCH_KIND` in `crates/gritty-core/src/search/mod.rs`. */
export const SEARCH_KIND = 'search.query';

/**
 * How many events a `starting` run holds before it drops the oldest.
 *
 * The response normally lands within a batch interval or two, so a handful is
 * plenty; the bound exists for the case where it does not, and it is what
 * stops a stalled response from growing an unbounded queue in the renderer.
 */
export const HELD_EVENT_LIMIT = 64;

/** Every match found in one file, in the order they arrived. */
export interface FileResult {
  readonly path: WorkspacePath;
  /** The path itself matched (scope `filenames` or `both`). */
  readonly nameMatch: SearchMatch | null;
  /** Content matches, one per line (T080 known limitation 1). */
  readonly lines: readonly SearchMatch[];
}

export type RunPhase = 'starting' | 'running' | 'completed' | 'cancelled' | 'failed';

export interface SearchRun {
  readonly token: number;
  readonly query: string;
  readonly options: SearchOptions;
  readonly phase: RunPhase;
  /** `null` until `search.query` answers. */
  readonly operationId: OperationId | null;
  /** Files in the order their first match arrived. */
  readonly files: readonly FileResult[];
  /** Name and content matches together — what the panel calls results. */
  readonly matchCount: number;
  /** The core's `truncatedMatches`: batches so far, the completion's total at the end. */
  readonly coreUndelivered: number;
  /** Matches in batches this side's own hold pushed out before they were folded. */
  readonly heldUndelivered: number;
  /** The walk's own count, from `OperationProgress`; `null` before the first report. */
  readonly filesScanned: number | null;
  /** `COMPLETED` only. */
  readonly summary: SearchSummary | null;
  /** `failed` only: the command's refusal or the operation's error. */
  readonly error: CoreErrorShape | null;
  /** `starting` only: events that may be this run's, in arrival order. */
  readonly held: readonly EventEnvelope[];
  /** Matches dropped from `held`, per operation id, until `started` says whose they were. */
  readonly heldDropped: Readonly<Record<string, number>>;
  /** Stop was pressed; the cancel is sent (or waits for the id) and the phase follows the event. */
  readonly stopRequested: boolean;
}

export interface SearchState {
  readonly run: SearchRun | null;
}

export const initialSearchState: SearchState = { run: null };

export type SearchAction =
  | {
      readonly type: 'start';
      readonly token: number;
      readonly query: string;
      readonly options: SearchOptions;
    }
  | { readonly type: 'started'; readonly token: number; readonly operationId: OperationId }
  | { readonly type: 'rejected'; readonly token: number; readonly error: CoreErrorShape }
  | { readonly type: 'stop' }
  | { readonly type: 'event'; readonly event: EventEnvelope }
  | { readonly type: 'clear' };

export function searchReducer(state: SearchState, action: SearchAction): SearchState {
  switch (action.type) {
    case 'start':
      return {
        run: {
          token: action.token,
          query: action.query,
          options: action.options,
          phase: 'starting',
          operationId: null,
          files: [],
          matchCount: 0,
          coreUndelivered: 0,
          heldUndelivered: 0,
          filesScanned: null,
          summary: null,
          error: null,
          held: [],
          heldDropped: {},
          stopRequested: false,
        },
      };
    case 'started': {
      const run = state.run;
      if (run === null || run.token !== action.token || run.phase !== 'starting') return state;
      let next: SearchRun = {
        ...run,
        phase: 'running',
        operationId: action.operationId,
        held: [],
        heldDropped: {},
        heldUndelivered: run.heldDropped[action.operationId] ?? 0,
      };
      // Folded in arrival order, and only what was this run's: anything else
      // in the hold belonged to a search this one superseded.
      for (const event of run.held) {
        if (idOf(event) === action.operationId) next = fold(next, event);
      }
      return { run: next };
    }
    case 'rejected': {
      const run = state.run;
      if (run === null || run.token !== action.token || run.phase !== 'starting') return state;
      return {
        run: { ...run, phase: 'failed', error: action.error, held: [], heldDropped: {} },
      };
    }
    case 'stop': {
      const run = state.run;
      if (run === null || !isActive(run) || run.stopRequested) return state;
      return { run: { ...run, stopRequested: true } };
    }
    case 'event': {
      const run = state.run;
      if (run === null || !isActive(run)) return state;
      const id = idOf(action.event);
      if (id === null) return state;
      if (run.phase === 'starting') return { run: hold(run, action.event, id) };
      if (id !== run.operationId) return state;
      return { run: fold(run, action.event) };
    }
    case 'clear':
      return state.run === null ? state : initialSearchState;
  }
}

/** Still waiting on the core: nothing it says has ended the run yet. */
export function isActive(run: SearchRun): boolean {
  return run.phase === 'starting' || run.phase === 'running';
}

/** Every match that was found and is not in the list. */
export function undeliveredOf(run: SearchRun): number {
  return run.coreUndelivered + run.heldUndelivered;
}

/**
 * The operation an event is about, or `null` when it is not a search event.
 *
 * `OperationProgress` carries no kind, so a progress report for any operation
 * passes here; the id comparison against the run is what filters it.
 */
function idOf(event: EventEnvelope): OperationId | null {
  switch (event.name) {
    case 'SearchMatchesFound':
    case 'OperationProgress':
      return event.payload.operationId;
    case 'OperationCompleted':
    case 'OperationFailed':
    case 'OperationCancelled':
      return event.payload.operation.kind === SEARCH_KIND ? event.payload.operation.id : null;
    default:
      return null;
  }
}

function hold(run: SearchRun, event: EventEnvelope, id: OperationId): SearchRun {
  // `OperationProgress` carries no kind, so every running operation's reports
  // reach the hold. Each one supersedes the last for its operation, so only the
  // latest is kept: a busy listing elsewhere must not displace this run's
  // batches.
  const held =
    event.name === 'OperationProgress'
      ? [...run.held.filter((candidate) => !isProgressOf(candidate, id)), event]
      : [...run.held, event];
  let heldDropped = run.heldDropped;
  while (held.length > HELD_EVENT_LIMIT) {
    // The oldest non-terminal event goes first. A terminal event is the one
    // thing whose loss would leave the run `running` for ever, and there is at
    // most one per operation, so they cannot fill the bound on their own.
    const victim = held.findIndex((candidate) => !isTerminal(candidate));
    const [dropped] = held.splice(victim === -1 ? 0 : victim, 1);
    if (dropped?.name === 'SearchMatchesFound') {
      const owner = dropped.payload.operationId;
      const lost = dropped.payload.matches.length + dropped.payload.truncatedMatches;
      heldDropped = { ...heldDropped, [owner]: (heldDropped[owner] ?? 0) + lost };
    }
  }
  return { ...run, held, heldDropped };
}

function isProgressOf(event: EventEnvelope, id: OperationId): boolean {
  return event.name === 'OperationProgress' && event.payload.operationId === id;
}

function isTerminal(event: EventEnvelope): boolean {
  return (
    event.name === 'OperationCompleted' ||
    event.name === 'OperationFailed' ||
    event.name === 'OperationCancelled'
  );
}

/** One event of the run's own operation, applied. */
function fold(run: SearchRun, event: EventEnvelope): SearchRun {
  if (!isActive(run)) return run;
  switch (event.name) {
    case 'SearchMatchesFound':
      return {
        ...addMatches(run, event.payload.matches),
        coreUndelivered: run.coreUndelivered + event.payload.truncatedMatches,
      };
    case 'OperationProgress':
      return { ...run, filesScanned: event.payload.progress.current };
    case 'OperationCompleted': {
      const completion = event.payload.result as SearchCompletion | undefined;
      return {
        ...run,
        phase: 'completed',
        summary: completion?.summary ?? null,
        // The completion's count is the whole search's, batches included, so
        // it replaces the running sum rather than adding to it.
        coreUndelivered: completion?.truncatedMatches ?? run.coreUndelivered,
        filesScanned: completion?.summary.filesScanned ?? run.filesScanned,
      };
    }
    case 'OperationFailed':
      return { ...run, phase: 'failed', error: event.payload.error };
    case 'OperationCancelled':
      return { ...run, phase: 'cancelled' };
    default:
      return run;
  }
}

function addMatches(run: SearchRun, matches: readonly SearchMatch[]): SearchRun {
  if (matches.length === 0) return run;
  const files = [...run.files];
  const index = new Map<string, number>();
  files.forEach((file, position) => {
    index.set(file.path, position);
  });
  for (const match of matches) {
    let position = index.get(match.path);
    if (position === undefined) {
      position = files.length;
      index.set(match.path, position);
      files.push({ path: match.path, nameMatch: null, lines: [] });
    }
    const file = files[position] as FileResult;
    files[position] =
      match.kind === 'filename'
        ? { ...file, nameMatch: match }
        : { ...file, lines: [...file.lines, match] };
  }
  return { ...run, files, matchCount: run.matchCount + matches.length };
}
