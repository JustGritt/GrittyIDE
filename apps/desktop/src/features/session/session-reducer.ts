import type {
  CoreErrorShape,
  EventEnvelope,
  PersistenceStatus,
  RecoveryCandidate,
  RecoveryState,
  SessionNotice,
  SessionRestoreResponse,
  WorkspaceId,
} from '@gritty/protocol';

/**
 * The session layer as the UI sees it (T060): where startup restoration
 * stands, whether persistence works, notices worth showing, and the recovery
 * candidates of the open workspace with what happened to each. Pure and
 * serializable; the hook drives it.
 */
export type SessionPhase = 'idle' | 'restoring' | 'ready';

export type CandidateStatus = 'pending' | 'restoring' | 'failed';

export interface CandidateItem {
  readonly path: string;
  readonly state: RecoveryState;
  readonly updatedAt: number;
  readonly size: number;
  readonly status: CandidateStatus;
  readonly error: string | null;
}

export interface Candidates {
  readonly workspaceId: WorkspaceId;
  readonly items: readonly CandidateItem[];
}

export interface SessionState {
  readonly phase: SessionPhase;
  readonly persistence: PersistenceStatus | null;
  readonly notices: readonly SessionNotice[];
  readonly candidates: Candidates | null;
  /** Tabs from the persisted session that could not be reopened. */
  readonly skipped: readonly string[];
}

export const initialSessionState: SessionState = {
  phase: 'idle',
  persistence: null,
  notices: [],
  candidates: null,
  skipped: [],
};

export type SessionAction =
  | { type: 'restoreStarted' }
  | { type: 'restoreFinished'; response: SessionRestoreResponse }
  | { type: 'restoreFailed'; error: CoreErrorShape }
  | { type: 'restoreSettled'; skipped: readonly string[] }
  | { type: 'candidatesLoaded'; workspaceId: WorkspaceId; candidates: readonly RecoveryCandidate[] }
  | { type: 'candidateStarted'; path: string }
  | { type: 'candidateFailed'; path: string; message: string }
  | { type: 'candidateRemoved'; path: string }
  | { type: 'dismissNotice'; index: number }
  | { type: 'dismissSkipped' }
  | { type: 'event'; event: EventEnvelope };

function addNotice(
  notices: readonly SessionNotice[],
  notice: SessionNotice,
): readonly SessionNotice[] {
  const duplicate = notices.some((n) => n.kind === notice.kind && n.message === notice.message);
  return duplicate ? notices : [...notices, notice];
}

function withItem(
  state: SessionState,
  path: string,
  update: (item: CandidateItem) => CandidateItem | null,
): SessionState {
  if (!state.candidates) return state;
  const items: CandidateItem[] = [];
  let changed = false;
  for (const item of state.candidates.items) {
    if (item.path !== path) {
      items.push(item);
      continue;
    }
    const next = update(item);
    changed = true;
    if (next) items.push(next);
  }
  if (!changed) return state;
  return { ...state, candidates: { ...state.candidates, items } };
}

/** Same-as-disk candidates that still wait for the user's answer. */
export function pendingChoices(state: SessionState): readonly CandidateItem[] {
  return (state.candidates?.items ?? []).filter(
    (item) => item.state === 'SAME_AS_DISK' && item.status !== 'restoring',
  );
}

/** Candidates the UI recovers by itself (the file moved on or off the disk). */
export function automaticCandidates(state: SessionState): readonly CandidateItem[] {
  return (state.candidates?.items ?? []).filter(
    (item) => item.state !== 'SAME_AS_DISK' && item.status === 'pending',
  );
}

/** Whether the recovery prompt has anything to show. */
export function hasPrompt(state: SessionState): boolean {
  return (state.candidates?.items.length ?? 0) > 0;
}

function onEvent(state: SessionState, event: EventEnvelope): SessionState {
  switch (event.name) {
    case 'WorkspaceClosed':
      return state.candidates?.workspaceId === event.payload.workspaceId
        ? { ...state, candidates: null }
        : state;
    case 'PersistenceStatusChanged': {
      const notices = event.payload.notice
        ? addNotice(state.notices, event.payload.notice)
        : state.notices;
      return { ...state, persistence: event.payload.status, notices };
    }
    default:
      return state;
  }
}

export function sessionReducer(state: SessionState, action: SessionAction): SessionState {
  switch (action.type) {
    case 'restoreStarted':
      return state.phase === 'idle' ? { ...state, phase: 'restoring' } : state;
    case 'restoreFinished': {
      let notices = state.notices;
      for (const notice of action.response.notices) notices = addNotice(notices, notice);
      return { ...state, persistence: action.response.persistence, notices };
    }
    case 'restoreFailed':
      return {
        ...state,
        persistence: {
          available: false,
          healthy: false,
          message: action.error.message,
        },
        notices: addNotice(state.notices, {
          kind: 'PERSISTENCE_UNAVAILABLE',
          message: `The session could not be restored: ${action.error.message}`,
        }),
      };
    case 'restoreSettled':
      return { ...state, phase: 'ready', skipped: action.skipped };
    case 'candidatesLoaded':
      return {
        ...state,
        candidates: {
          workspaceId: action.workspaceId,
          items: action.candidates.map((candidate) => ({
            path: candidate.path,
            state: candidate.state,
            updatedAt: candidate.updatedAt,
            size: candidate.size,
            status: 'pending',
            error: null,
          })),
        },
      };
    case 'candidateStarted':
      return withItem(state, action.path, (item) => ({
        ...item,
        status: 'restoring',
        error: null,
      }));
    case 'candidateFailed':
      return withItem(state, action.path, (item) => ({
        ...item,
        status: 'failed',
        error: action.message,
      }));
    case 'candidateRemoved':
      return withItem(state, action.path, () => null);
    case 'dismissNotice':
      return { ...state, notices: state.notices.filter((_, index) => index !== action.index) };
    case 'dismissSkipped':
      return state.skipped.length === 0 ? state : { ...state, skipped: [] };
    case 'event':
      return onEvent(state, action.event);
  }
}
