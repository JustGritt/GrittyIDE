import type {
  AgentRun,
  AgentRunEvent,
  AgentRunEventLog,
  AgentRunEventRecord,
  AgentRunId,
  AgentRunState,
  EventEnvelope,
  Workspace,
} from '@gritty/protocol';

/**
 * Agent runs Gritty **owns**, as the renderer holds them (T300-CMD, §§16-17).
 *
 * Pure, so the whole of what the panel says — which runs exist, what each is
 * doing, what it reported and where its event stream has a hole in it — is
 * tested without React, a core or a DOM.
 *
 * Three rules hold it together, and each is one the core decided first:
 *
 * 1. **`AgentRunChanged` replaces, `AgentRunEventAppended` appends.** The
 *    first carries a whole run because a run is small and a consumer that
 *    missed one converges on the next; the second is a delta because an event
 *    stream grows without bound and resending it would be the unbounded
 *    forwarding hard rule 8 forbids.
 * 2. **An event for a run this state has never seen is dropped**, not stored
 *    against a run that does not exist. It is the renderer's half of the
 *    core's own rule ("an event names a `run_id` matching no known run →
 *    dropped, logged once, never forwarded"), and it is what stops a late
 *    batch from resurrecting a run whose workspace has already closed.
 * 3. **A loss the core reported is kept, never repaired.** `dropped` is
 *    carried through so the panel can draw a gap; splicing the events either
 *    side of one together would be the UI inventing a continuity the core
 *    deliberately refused to claim.
 *
 * Nothing here is optimistic. A run the core refused to start must not appear
 * to exist, so only a command's own answer or an event ever adds one.
 */

/**
 * How many events the view keeps per run, mirroring the core's ring
 * (`MAX_RUN_EVENTS` in `crates/gritty-core/src/agent_run/wire.rs`).
 *
 * Not a second policy: the core drops from the oldest end at exactly this
 * count and *reports* the loss. Mirroring it means a client that has been
 * listening since the run started trims the same events the core already
 * counted, instead of growing without bound behind the core's back.
 */
export const MAX_RUN_EVENTS = 512;

/** One run's event stream, and what fell off the front of it. */
export interface RunLog {
  readonly events: readonly AgentRunEventRecord[];
  /** Cumulative, as the core reports it. A gap the panel draws. */
  readonly dropped: number;
}

export const emptyRunLog: RunLog = { events: [], dropped: 0 };

export interface RunsState {
  readonly byId: Readonly<Record<string, AgentRun>>;
  readonly logs: Readonly<Record<string, RunLog>>;
  /** False until the first `runs.list` lands, so "none" and "not yet" differ. */
  readonly loaded: boolean;
}

export const initialRunsState: RunsState = { byId: {}, logs: {}, loaded: false };

export type RunsAction =
  /** The open workspace changed; a run belongs to one and none survives. */
  | { readonly type: 'workspace'; readonly workspace: Workspace | null }
  | { readonly type: 'loaded'; readonly runs: readonly AgentRun[] }
  /** A command answered with a run snapshot (`startFake`, `cancel`). */
  | { readonly type: 'run'; readonly run: AgentRun }
  /** `runs.events` rehydrated one run's stream. */
  | { readonly type: 'log'; readonly log: AgentRunEventLog }
  | { readonly type: 'event'; readonly event: EventEnvelope };

const TERMINAL: readonly AgentRunState[] = ['FINISHED', 'FAILED', 'CANCELLED', 'INTERRUPTED'];

/** A dead end. Terminal states have no way back — a retry is a second run. */
export function isTerminalRun(state: AgentRunState): boolean {
  return TERMINAL.includes(state);
}

/** Stopped, with the user holding it up. What an attention count would total. */
export function needsAttention(state: AgentRunState): boolean {
  return (
    state === 'WAITING' || state === 'NEEDS_DECISION' || state === 'BLOCKED' || state === 'REVIEW'
  );
}

/**
 * Later wins, and a terminal snapshot is never undone.
 *
 * `updatedAt` is what the core moves on every state or activity change, so it
 * orders two snapshots of the same run. The terminal guard covers the one case
 * a timestamp cannot settle: two snapshots stamped in the same millisecond,
 * where letting `RUNNING` win would leave a finished run spinning in the panel
 * forever.
 */
function merge(existing: AgentRun | undefined, incoming: AgentRun): AgentRun {
  if (!existing) return incoming;
  if (isTerminalRun(existing.state) && !isTerminalRun(incoming.state)) return existing;
  if (existing.updatedAt > incoming.updatedAt) return existing;
  return incoming;
}

function upsert(state: RunsState, run: AgentRun): RunsState {
  const merged = merge(state.byId[run.id], run);
  if (merged === state.byId[run.id]) return state;
  return { ...state, byId: { ...state.byId, [run.id]: merged } };
}

/**
 * Append a batch, keeping `seq` strictly increasing and the ring bounded.
 *
 * `seq` is the core's own per-run counter, so an event already seen is a
 * duplicate delivery — the rehydrating call and a live batch racing — and not
 * new information. Dropping it is how the two paths stay idempotent.
 */
function append(log: RunLog, records: readonly AgentRunEventRecord[], dropped: number): RunLog {
  const last = log.events.at(-1)?.seq ?? 0;
  const fresh = records.filter((record) => record.seq > last);
  const nextDropped = Math.max(log.dropped, dropped);
  if (fresh.length === 0) {
    return nextDropped === log.dropped ? log : { ...log, dropped: nextDropped };
  }
  const events = [...log.events, ...fresh];
  return {
    events: events.length > MAX_RUN_EVENTS ? events.slice(events.length - MAX_RUN_EVENTS) : events,
    dropped: nextDropped,
  };
}

function withLog(state: RunsState, runId: AgentRunId, merged: RunLog): RunsState {
  if (merged === (state.logs[runId] ?? emptyRunLog)) return state;
  return { ...state, logs: { ...state.logs, [runId]: merged } };
}

export function runsReducer(state: RunsState, action: RunsAction): RunsState {
  switch (action.type) {
    case 'workspace':
      // A run is scoped to the workspace it was started against, so changing
      // one empties the panel rather than leaving rows pointing at work the
      // core has already cancelled.
      return initialRunsState;
    case 'loaded': {
      const next = action.runs.reduce(upsert, state);
      return next.loaded ? next : { ...next, loaded: true };
    }
    case 'run':
      return upsert(state, action.run);
    case 'log': {
      const { log } = action;
      // A stream for a run that is not in the roster has nothing to attach to
      // (rule 2 above).
      if (!state.byId[log.runId]) return state;
      const existing = state.logs[log.runId] ?? emptyRunLog;
      return withLog(state, log.runId, append(existing, log.events, log.dropped));
    }
    case 'event': {
      const { event } = action;
      if (event.name === 'AgentRunChanged') return upsert(state, event.payload.run);
      if (event.name === 'AgentRunEventAppended') {
        const { runId, events, dropped } = event.payload;
        if (!state.byId[runId]) return state;
        const existing = state.logs[runId] ?? emptyRunLog;
        return withLog(state, runId, append(existing, events, dropped));
      }
      return state;
    }
    default:
      return state;
  }
}

/** Newest first, so the run just started is the one at the top. */
export function selectRuns(state: RunsState): AgentRun[] {
  return Object.values(state.byId).sort(
    (a, b) => b.startedAt - a.startedAt || b.id.localeCompare(a.id),
  );
}

/** One run's stream, empty rather than absent when nothing has arrived. */
export function selectLog(state: RunsState, runId: AgentRunId): RunLog {
  return state.logs[runId] ?? emptyRunLog;
}

/**
 * `RUNNING · Editing src/auth/session.ts` — the state and the activity read
 * together, which is the pair the two fields exist to be read as.
 *
 * Every part is a string the core already bounded and collapsed; nothing here
 * parses, resolves or interprets one (hard rule 13). The activity's `command`
 * is preferred over its path because a command says more about what is
 * happening than the file it happens to touch.
 */
export function describeRun(run: AgentRun): string {
  const { activity } = run;
  if (!activity) return run.state;
  const subject = activity.command ?? activity.path?.display ?? null;
  const kind = titleCase(activity.kind);
  return subject ? `${run.state} · ${kind} ${subject}` : `${run.state} · ${kind}`;
}

/**
 * One event as a line of text. Display only, and text only: never Markdown,
 * never HTML, never an openable path — the panel renders the result into a
 * text node, and nothing here makes a string more powerful than it arrived.
 *
 * An unknown variant still produces a line rather than throwing, which is the
 * whole reason `AgentRunEvent` is tagged: a renderer built against an older
 * protocol shows what it cannot name instead of failing to draw the batch it
 * arrived in.
 */
export function describeEvent(event: AgentRunEvent): string {
  switch (event.type) {
    case 'RUN_STARTED':
      return 'started';
    case 'RUN_FINISHED':
      return event.summary ? `finished · ${event.summary}` : 'finished';
    case 'PROMPT_STARTED':
      return `prompt · ${event.text}`;
    case 'PROMPT_FINISHED':
      return event.text ? `reply · ${event.text}` : 'reply';
    case 'FILE_READ':
      return `read · ${event.path.display}`;
    case 'FILE_WRITE_REQUESTED':
      return `write requested · ${event.path.display}`;
    case 'FILE_WRITTEN':
      return `wrote · ${event.path.display}`;
    case 'COMMAND_STARTED':
      return `ran · ${event.command}`;
    case 'COMMAND_FINISHED':
      return `${event.ok ? 'ok' : 'failed'} · ${event.command}${
        event.exitCode === undefined ? '' : ` (exit ${event.exitCode})`
      }`;
    case 'PERMISSION_REQUESTED':
      return `permission · ${event.tool}${event.detail ? ` · ${event.detail}` : ''}`;
    case 'DECISION_REQUESTED':
      return `decision · ${event.question}`;
    case 'RUN_BLOCKED':
      return `blocked · ${event.reason}`;
    case 'RUN_RESUMED':
      return 'resumed';
    case 'RUN_FAILED':
      return `failed · ${event.error.code}: ${event.error.message}`;
    default:
      // A variant this build does not know. Named, not swallowed.
      return (event as { type: string }).type;
  }
}

/** `NEEDS_DECISION` → `Needs decision`. Presentation only. */
function titleCase(kind: string): string {
  const words = kind.toLowerCase().replace(/_/g, ' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}
