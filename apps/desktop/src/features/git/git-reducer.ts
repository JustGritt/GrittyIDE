import type {
  CoreErrorShape,
  DiffSummary,
  EventEnvelope,
  OperationId,
  RepositoryStatus,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';

/**
 * The Changes tab's state (T100-U). Pure, so the three-armed response, the
 * operation hand-over and the race between them are unit-tested without React,
 * Tauri or a repository.
 *
 * Two reads live here, and they are deliberately independent: `git.status`
 * (which files changed) and `git.diff` (what changed in the one file the user
 * selected). Asking for one file's hunks is what the wire's `path` argument is
 * for — the bounded way to reach the hunks of the selected file instead of
 * paging every changed file's hunks to get at one of them.
 *
 * Neither read writes anything. There is no stage, commit, checkout or fetch
 * on the wire to call.
 */

/** The kind the core stamps on a `git.status` that outlived its budget. */
export const STATUS_OPERATION_KIND = 'git.status';
/** The same, for `git.diff`. */
export const DIFF_OPERATION_KIND = 'git.diff';

/**
 * How many operation outcomes may wait for the `pending` response that names
 * them before the oldest is dropped. Two reads can be in flight at once, so
 * this is slack for a pathological ordering rather than a queue.
 */
const UNMATCHED_LIMIT = 8;

/**
 * One read of the repository, in every state it can be in.
 *
 * `notARepository` is a **value**, not a failure: opening a folder that is not
 * a repository is not a mistake, and an error envelope in front of every
 * non-repository workspace would be the UI making one up.
 *
 * `pending` is the other arm the wire forces on us: a read that outlives its
 * budget continues as a cancellable `Operation` and finishes on
 * `OperationCompleted`, so the UI has to have a state for "still going, and
 * you can stop it".
 */
export type Read<T> =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'pending'; readonly operationId: OperationId }
  | { readonly kind: 'ready'; readonly value: T }
  | { readonly kind: 'notARepository' }
  | { readonly kind: 'failed'; readonly error: CoreErrorShape }
  | { readonly kind: 'cancelled' };

/** What an `Operation*` event said, kept until the response that names it arrives. */
export type OperationOutcome =
  | { readonly kind: 'result'; readonly result: unknown }
  | { readonly kind: 'failed'; readonly error: CoreErrorShape }
  | { readonly kind: 'cancelled' };

export interface GitState {
  /** The workspace these reads belong to; everything resets when it changes. */
  readonly workspaceId: WorkspaceId | null;
  readonly status: Read<RepositoryStatus>;
  /** A `cursor` page is in flight and the page already shown is being kept. */
  readonly appending: boolean;
  /** The path whose hunks are being shown; `null` when the list is all there is. */
  readonly selected: WorkspacePath | null;
  readonly diff: Read<DiffSummary>;
  /**
   * Outcomes of operations whose `pending` response has not been applied yet.
   * The command response and the completion event are separate IPC messages
   * and their order is not guaranteed — the explorer learned this first
   * (`explorer-reducer.ts`) and the shape is deliberately the same.
   */
  readonly unmatched: Readonly<Record<string, OperationOutcome>>;
}

export const initialGitState: GitState = {
  workspaceId: null,
  status: { kind: 'idle' },
  appending: false,
  selected: null,
  diff: { kind: 'idle' },
  unmatched: {},
};

export type GitAction =
  | { type: 'workspace'; workspaceId: WorkspaceId | null }
  /** A fresh read; `append` continues the page already shown. */
  | { type: 'statusStarted'; append: boolean }
  | { type: 'statusPending'; operationId: OperationId }
  | { type: 'statusReady'; repository: RepositoryStatus; append: boolean }
  | { type: 'statusNotARepository' }
  | { type: 'statusFailed'; error: CoreErrorShape }
  | { type: 'select'; path: WorkspacePath | null }
  | { type: 'diffStarted' }
  | { type: 'diffPending'; operationId: OperationId }
  | { type: 'diffReady'; diff: DiffSummary }
  | { type: 'diffNotARepository' }
  | { type: 'diffFailed'; error: CoreErrorShape }
  | { type: 'event'; event: EventEnvelope };

export function gitReducer(state: GitState, action: GitAction): GitState {
  switch (action.type) {
    case 'workspace':
      // Same workspace, same reads: this action fires on every mount and a
      // reset here would re-read the repository each time the tab is shown.
      if (action.workspaceId === state.workspaceId) return state;
      return { ...initialGitState, workspaceId: action.workspaceId };

    case 'statusStarted':
      return {
        ...state,
        appending: action.append && state.status.kind === 'ready',
        status: action.append && state.status.kind === 'ready' ? state.status : { kind: 'loading' },
      };

    case 'statusPending':
      return claim(
        { ...state, status: { kind: 'pending', operationId: action.operationId } },
        action.operationId,
      );

    case 'statusReady':
      return {
        ...state,
        appending: false,
        status: {
          kind: 'ready',
          value:
            action.append && state.status.kind === 'ready'
              ? mergeStatus(state.status.value, action.repository)
              : action.repository,
        },
      };

    case 'statusNotARepository':
      // Nothing to select in a folder that is not a repository, and a stale
      // selection would ask for a diff that cannot exist.
      return {
        ...state,
        appending: false,
        status: { kind: 'notARepository' },
        selected: null,
        diff: { kind: 'idle' },
      };

    case 'statusFailed':
      return { ...state, appending: false, status: { kind: 'failed', error: action.error } };

    case 'select':
      if (action.path === state.selected) return state;
      return { ...state, selected: action.path, diff: { kind: 'idle' } };

    case 'diffStarted':
      return { ...state, diff: { kind: 'loading' } };

    case 'diffPending':
      return claim(
        { ...state, diff: { kind: 'pending', operationId: action.operationId } },
        action.operationId,
      );

    case 'diffReady':
      return { ...state, diff: { kind: 'ready', value: action.diff } };

    case 'diffNotARepository':
      return { ...state, diff: { kind: 'notARepository' } };

    case 'diffFailed':
      return { ...state, diff: { kind: 'failed', error: action.error } };

    case 'event':
      return applyEvent(state, action.event);
  }
}

/**
 * Append a page to the one already shown.
 *
 * The newer page's totals win — it is the more recent count — and its cursor
 * is the one to continue from. Entries are concatenated rather than merged by
 * path: pages are disjoint and sorted by path, so concatenation preserves the
 * order the core chose.
 */
function mergeStatus(shown: RepositoryStatus, page: RepositoryStatus): RepositoryStatus {
  return {
    ...page,
    entries: [...shown.entries, ...page.entries],
    // Every page reports what *it* could not represent, so the counts add up
    // rather than replace each other. Dropping the earlier one would quietly
    // shrink a number whose whole purpose is to say nothing was hidden.
    unrepresentable: shown.unrepresentable + page.unrepresentable,
  };
}

function applyEvent(state: GitState, event: EventEnvelope): GitState {
  switch (event.name) {
    case 'OperationCompleted': {
      const outcome: OperationOutcome = { kind: 'result', result: event.payload.result };
      return settle(state, event.payload.operation.kind, event.payload.operation.id, outcome);
    }
    case 'OperationFailed':
      return settle(state, event.payload.operation.kind, event.payload.operation.id, {
        kind: 'failed',
        error: event.payload.error,
      });
    case 'OperationCancelled':
      return settle(state, event.payload.operation.kind, event.payload.operation.id, {
        kind: 'cancelled',
      });
    default:
      return state;
  }
}

/** Route an outcome to the read waiting for it, or park it until one is. */
function settle(
  state: GitState,
  kind: string,
  operationId: OperationId,
  outcome: OperationOutcome,
): GitState {
  if (kind === STATUS_OPERATION_KIND) {
    if (state.status.kind === 'pending' && state.status.operationId === operationId) {
      return { ...state, appending: false, status: applyStatusOutcome(outcome) };
    }
    return park(state, operationId, outcome);
  }
  if (kind === DIFF_OPERATION_KIND) {
    if (state.diff.kind === 'pending' && state.diff.operationId === operationId) {
      return { ...state, diff: applyDiffOutcome(outcome) };
    }
    return park(state, operationId, outcome);
  }
  return state;
}

/** The outcome arrived first; apply it as soon as a read claims the id. */
function claim(state: GitState, operationId: OperationId): GitState {
  const outcome = state.unmatched[operationId];
  if (!outcome) return state;
  const { [operationId]: _matched, ...unmatched } = state.unmatched;
  const claimed: GitState = { ...state, unmatched };
  if (claimed.status.kind === 'pending' && claimed.status.operationId === operationId) {
    return { ...claimed, appending: false, status: applyStatusOutcome(outcome) };
  }
  if (claimed.diff.kind === 'pending' && claimed.diff.operationId === operationId) {
    return { ...claimed, diff: applyDiffOutcome(outcome) };
  }
  return claimed;
}

function park(state: GitState, operationId: OperationId, outcome: OperationOutcome): GitState {
  const entries = Object.entries(state.unmatched);
  // Bounded: an operation whose response never arrives must not grow this map
  // for the life of the window. Oldest first, since insertion order is arrival
  // order.
  const kept = entries.slice(Math.max(0, entries.length - (UNMATCHED_LIMIT - 1)));
  return { ...state, unmatched: { ...Object.fromEntries(kept), [operationId]: outcome } };
}

/**
 * A completed operation carries its payload in `OperationCompleted.result`,
 * typed on the wire as arbitrary JSON. Casting it is the same trust the
 * explorer places in `DirectoryPage`; a *missing* result is reported rather
 * than rendered as an empty repository, which would be the "partial data shown
 * as complete" the task's failure table forbids.
 */
function applyStatusOutcome(outcome: OperationOutcome): Read<RepositoryStatus> {
  switch (outcome.kind) {
    case 'result':
      return outcome.result
        ? { kind: 'ready', value: outcome.result as RepositoryStatus }
        : { kind: 'failed', error: missingResult('status') };
    case 'failed':
      return { kind: 'failed', error: outcome.error };
    case 'cancelled':
      return { kind: 'cancelled' };
  }
}

function applyDiffOutcome(outcome: OperationOutcome): Read<DiffSummary> {
  switch (outcome.kind) {
    case 'result':
      return outcome.result
        ? { kind: 'ready', value: outcome.result as DiffSummary }
        : { kind: 'failed', error: missingResult('diff') };
    case 'failed':
      return { kind: 'failed', error: outcome.error };
    case 'cancelled':
      return { kind: 'cancelled' };
  }
}

function missingResult(what: string): CoreErrorShape {
  return {
    code: 'Internal',
    message: `git.${what} completed without a result`,
    retryable: false,
  };
}

/** The next page's cursor, or `null` when the list is complete. */
export function statusCursor(state: GitState): string | null {
  if (state.status.kind !== 'ready') return null;
  const { truncated, cursor } = state.status.value;
  return truncated && cursor !== undefined ? cursor : null;
}

/** The operation a read handed over to, so the UI can offer to cancel it. */
export function pendingOperation(read: Read<unknown>): OperationId | null {
  return read.kind === 'pending' ? read.operationId : null;
}
