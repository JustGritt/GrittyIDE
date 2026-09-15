import type {
  CommitPage,
  CoreErrorShape,
  DiffSummary,
  EventEnvelope,
  GitWriteAvailability,
  GitWriteResult,
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
 * Three reads live here, and they are deliberately independent: `git.status`
 * (which files changed), `git.diff` (what changed in the one file the user
 * selected) and `git.log`. Asking for one file's hunks is what the wire's
 * `path` argument is for — the bounded way to reach the hunks of the selected
 * file instead of paging every changed file's hunks to get at one of them.
 *
 * And one write at a time (T102-U): stage, unstage, discard or commit. A write
 * answers with the repository as it now stands, so the status converges on the
 * response instead of being read again behind it.
 */

/** The kind the core stamps on a `git.status` that outlived its budget. */
export const STATUS_OPERATION_KIND = 'git.status';
/** The same, for `git.diff`. */
export const DIFF_OPERATION_KIND = 'git.diff';
/** The same, for `git.log`. */
export const LOG_OPERATION_KIND = 'git.log';
/**
 * The same, for any of the four writes. One kind for all of them, the way
 * `files.transfer` covers a move and a delete (T102-C); in practice only a
 * commit whose hooks are running takes it.
 */
export const WRITE_OPERATION_KIND = 'git.write';

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

/**
 * One write, as it was sent. Kept on every state of the write after it, so a
 * refusal carries what it refused: the dirty-document replay re-sends *this*
 * request with `force`, and there is no second slot that could disagree with
 * it (the defect `EXPLORER-FORCE-REPLAY` and `EXPLORER-DISCARD-NOTICE` fixed).
 */
export type WriteRequest =
  | { readonly verb: 'stage'; readonly paths: readonly WorkspacePath[] }
  | { readonly verb: 'unstage'; readonly paths: readonly WorkspacePath[] }
  | {
      readonly verb: 'discard';
      readonly paths: readonly WorkspacePath[];
      readonly force: boolean;
    }
  | {
      readonly verb: 'commit';
      readonly message: string;
      readonly amend: boolean;
      readonly noVerify: boolean;
    };

/**
 * The write in flight, or the last one's outcome until it is dismissed or
 * replaced.
 *
 * `done` is kept rather than dropped straight back to `idle` because two
 * results have something to say: a discard says where the work went, and a
 * commit names the commit it made.
 */
export type Write =
  | { readonly kind: 'idle' }
  | { readonly kind: 'running'; readonly request: WriteRequest }
  | {
      readonly kind: 'pending';
      readonly request: WriteRequest;
      readonly operationId: OperationId;
    }
  | { readonly kind: 'done'; readonly request: WriteRequest; readonly result: GitWriteResult }
  | { readonly kind: 'failed'; readonly request: WriteRequest; readonly error: CoreErrorShape }
  | { readonly kind: 'cancelled'; readonly request: WriteRequest };

/**
 * What is typed into the commit box.
 *
 * In the reducer rather than in a component's own state so the rule that
 * matters is a pure, tested transition: **only a commit that happened clears
 * it.** A hook that refuses, a cancel and a failure all leave the message where
 * the user left it (T102-U, failure behaviour).
 */
export interface CommitDraft {
  readonly message: string;
  /** A visible choice, never a default; back to `false` after every commit. */
  readonly amend: boolean;
  /** The same: skipping the project's own hooks is never where the box starts. */
  readonly noVerify: boolean;
}

export const emptyDraft: CommitDraft = { message: '', amend: false, noVerify: false };

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
   * The History tab's page of commits.
   *
   * A third read rather than a field on the status: it answers a different
   * question, it pages on its own cursor, and it is asked for only while the
   * History tab is showing. Keeping it beside the other two is what lets one
   * `unmatched` map serve all three.
   */
  readonly history: Read<CommitPage>;
  /** A `cursor` page of history is in flight and the page shown is kept. */
  readonly appendingHistory: boolean;
  /**
   * Whether `git` can write here, as the last `git.status` that carried the
   * probe's verdict said. `null` until one has: absence is never read as
   * "yes" (T102-C), so the write affordances stay disabled until it arrives.
   */
  readonly availability: GitWriteAvailability | null;
  readonly write: Write;
  readonly draft: CommitDraft;
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
  history: { kind: 'idle' },
  appendingHistory: false,
  availability: null,
  write: { kind: 'idle' },
  draft: emptyDraft,
  unmatched: {},
};

export type GitAction =
  /**
   * `message` is a draft kept from an earlier mount of the panel for this
   * workspace; the panel unmounts whenever another sidebar tab is showing.
   */
  | { type: 'workspace'; workspaceId: WorkspaceId | null; message?: string }
  /** A fresh read; `append` continues the page already shown. */
  | { type: 'statusStarted'; append: boolean }
  | { type: 'statusPending'; operationId: OperationId }
  | {
      type: 'statusReady';
      repository: RepositoryStatus;
      append: boolean;
      /** The probe's verdict, when the response carried one. */
      write?: GitWriteAvailability;
    }
  | { type: 'statusNotARepository' }
  | { type: 'statusFailed'; error: CoreErrorShape }
  | { type: 'select'; path: WorkspacePath | null }
  | { type: 'diffStarted' }
  | { type: 'diffPending'; operationId: OperationId }
  | { type: 'diffReady'; diff: DiffSummary }
  | { type: 'diffNotARepository' }
  | { type: 'diffFailed'; error: CoreErrorShape }
  | { type: 'logStarted'; append: boolean }
  | { type: 'logPending'; operationId: OperationId }
  | { type: 'logReady'; page: CommitPage; append: boolean }
  | { type: 'logNotARepository' }
  | { type: 'logFailed'; error: CoreErrorShape }
  /** Refused while another write is in flight: one at a time, never queued. */
  | { type: 'writeStarted'; request: WriteRequest }
  | { type: 'writePending'; operationId: OperationId }
  | { type: 'writeReady'; result: GitWriteResult }
  | { type: 'writeNotARepository' }
  | { type: 'writeFailed'; error: CoreErrorShape }
  /** Put away the last write's outcome. Nothing in flight is ever dismissed. */
  | { type: 'writeDismissed' }
  | { type: 'draftChanged'; draft: Partial<CommitDraft> }
  | { type: 'event'; event: EventEnvelope };

export function gitReducer(state: GitState, action: GitAction): GitState {
  switch (action.type) {
    case 'workspace':
      // Same workspace, same reads: this action fires on every mount and a
      // reset here would re-read the repository each time the tab is shown.
      if (action.workspaceId === state.workspaceId) return state;
      return {
        ...initialGitState,
        workspaceId: action.workspaceId,
        draft: { ...emptyDraft, message: action.message ?? '' },
      };

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
        // Kept when a response carries no verdict: a status that finished as an
        // operation arrives as a bare `RepositoryStatus`, and the probe's answer
        // does not change within a process.
        availability: action.write ?? state.availability,
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

    case 'logStarted':
      return {
        ...state,
        appendingHistory: action.append && state.history.kind === 'ready',
        history:
          action.append && state.history.kind === 'ready' ? state.history : { kind: 'loading' },
      };

    case 'logPending':
      return claim(
        { ...state, history: { kind: 'pending', operationId: action.operationId } },
        action.operationId,
      );

    case 'logReady':
      return {
        ...state,
        appendingHistory: false,
        history: {
          kind: 'ready',
          value:
            action.append && state.history.kind === 'ready'
              ? mergeHistory(state.history.value, action.page)
              : action.page,
        },
      };

    case 'logNotARepository':
      return { ...state, appendingHistory: false, history: { kind: 'notARepository' } };

    case 'logFailed':
      return {
        ...state,
        appendingHistory: false,
        history: { kind: 'failed', error: action.error },
      };

    case 'writeStarted':
      if (writeBusy(state.write)) return state;
      return { ...state, write: { kind: 'running', request: action.request } };

    case 'writePending':
      if (state.write.kind !== 'running') return state;
      return claim(
        {
          ...state,
          write: {
            kind: 'pending',
            request: state.write.request,
            operationId: action.operationId,
          },
        },
        action.operationId,
      );

    case 'writeReady':
      if (state.write.kind !== 'running') return state;
      return converge(state, state.write.request, action.result);

    case 'writeNotARepository':
      if (state.write.kind !== 'running') return state;
      return {
        ...state,
        write: { kind: 'idle' },
        appending: false,
        status: { kind: 'notARepository' },
        selected: null,
        diff: { kind: 'idle' },
      };

    case 'writeFailed':
      if (state.write.kind !== 'running' && state.write.kind !== 'pending') return state;
      return {
        ...state,
        write: { kind: 'failed', request: state.write.request, error: action.error },
      };

    case 'writeDismissed':
      if (writeBusy(state.write)) return state;
      return { ...state, write: { kind: 'idle' } };

    case 'draftChanged':
      return { ...state, draft: { ...state.draft, ...action.draft } };

    case 'event':
      return applyEvent(state, action.event);
  }
}

/** A write is in flight: the affordances wait for it rather than queue behind it. */
export function writeBusy(write: Write): boolean {
  return write.kind === 'running' || write.kind === 'pending';
}

/**
 * Apply a write's result: the repository it describes replaces the status, and
 * nothing reads `git.status` behind it (T102-U requirement 7).
 *
 * What else goes stale is decided per verb, because the reads measure against
 * different things. `git.diff` compares HEAD with the working tree, so staging
 * and unstaging leave the selected file's diff exactly as it was; a discard
 * changes it for the paths it discarded, and a commit moves HEAD, which changes
 * it for everything — and adds a commit to a history already shown.
 */
function converge(state: GitState, request: WriteRequest, result: GitWriteResult): GitState {
  const commit = request.verb === 'commit';
  const selected = state.selected;
  const diffStale =
    selected !== null &&
    (commit || (request.verb === 'discard' && result.changed.includes(selected)));
  return {
    ...state,
    write: { kind: 'done', request, result },
    appending: false,
    status: { kind: 'ready', value: result.repository },
    diff: diffStale ? { kind: 'idle' } : state.diff,
    // `idle` is how the panel knows to walk it again while it is showing.
    history: commit && state.history.kind !== 'idle' ? { kind: 'idle' } : state.history,
    appendingHistory: commit ? false : state.appendingHistory,
    draft: commit ? emptyDraft : state.draft,
  };
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

/**
 * Append a page of history to the one already shown.
 *
 * Concatenation, like `mergeStatus`: pages are disjoint because the cursor is
 * the last id of the previous page, and the walk order is the core's. The
 * newer page's `truncated`/`cursor` win, because they describe the boundary the
 * next call continues from.
 */
function mergeHistory(shown: CommitPage, page: CommitPage): CommitPage {
  return { ...page, commits: [...shown.commits, ...page.commits] };
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
  if (kind === LOG_OPERATION_KIND) {
    if (state.history.kind === 'pending' && state.history.operationId === operationId) {
      return { ...state, appendingHistory: false, history: applyLogOutcome(outcome) };
    }
    return park(state, operationId, outcome);
  }
  if (kind === WRITE_OPERATION_KIND) {
    if (state.write.kind === 'pending' && state.write.operationId === operationId) {
      return applyWriteOutcome(state, state.write.request, outcome);
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
  if (claimed.history.kind === 'pending' && claimed.history.operationId === operationId) {
    return { ...claimed, appendingHistory: false, history: applyLogOutcome(outcome) };
  }
  if (claimed.write.kind === 'pending' && claimed.write.operationId === operationId) {
    return applyWriteOutcome(claimed, claimed.write.request, outcome);
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

function applyLogOutcome(outcome: OperationOutcome): Read<CommitPage> {
  switch (outcome.kind) {
    case 'result':
      return outcome.result
        ? { kind: 'ready', value: outcome.result as CommitPage }
        : { kind: 'failed', error: missingResult('log') };
    case 'failed':
      return { kind: 'failed', error: outcome.error };
    case 'cancelled':
      return { kind: 'cancelled' };
  }
}

/**
 * A write that became an operation — a commit whose hooks outlived the budget.
 *
 * A cancel is the one outcome that leaves the repository unknown: the core
 * kills the hook's process group, and whether `git commit` got as far as
 * moving HEAD first is not something the cancel reports. So the status (and
 * the diff it measures against) is read again rather than trusted.
 */
function applyWriteOutcome(
  state: GitState,
  request: WriteRequest,
  outcome: OperationOutcome,
): GitState {
  switch (outcome.kind) {
    case 'result':
      return outcome.result
        ? converge(state, request, outcome.result as GitWriteResult)
        : { ...state, write: { kind: 'failed', request, error: missingResult('write') } };
    case 'failed':
      return { ...state, write: { kind: 'failed', request, error: outcome.error } };
    case 'cancelled':
      return {
        ...state,
        write: { kind: 'cancelled', request },
        appending: false,
        status: { kind: 'idle' },
        diff: state.selected !== null ? { kind: 'idle' } : state.diff,
      };
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

/** The next page of history's cursor, or `null` when the walk is complete. */
export function historyCursor(state: GitState): string | null {
  if (state.history.kind !== 'ready') return null;
  const { truncated, cursor } = state.history.value;
  return truncated && cursor !== undefined ? cursor : null;
}

/** The operation a read or a write handed over to, so the UI can offer to cancel it. */
export function pendingOperation(read: Read<unknown> | Write): OperationId | null {
  return read.kind === 'pending' ? read.operationId : null;
}
