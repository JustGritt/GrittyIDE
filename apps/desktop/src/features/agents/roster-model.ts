import type { AgentRoster, AgentRun, AgentRunId, AgentSession, Workstream } from '@gritty/protocol';
import { isTerminalRun, needsAttention } from '../runs/runs-reducer';
import { activitySourceLabel, agentStatus } from './agent-status';
import { taskLine, toolLine } from './agents-reducer';
import { type RunStatus, runStatus } from './run-status';

/**
 * The one list the sidebar's Agents section draws (T233-M, design artboard 12a).
 *
 * It merges the two things `docs/architecture/agent-runs.md` is at pains to keep
 * apart, and the point of this module is that it merges them **for display
 * only**:
 *
 * | | `agent_session` (T220) | `agent_run` (T300) |
 * |---|---|---|
 * | Who started it | some other terminal | Gritty |
 * | What Gritty can do | read evidence | start, cancel, account for it |
 *
 * The two are different types with different lifetimes and they never import
 * each other. Nothing here changes that: no common interface is imposed on the
 * domain, no field is copied from one onto the other, and the projection is
 * one-way — a `RosterRow` is what a row *says*, and the ids it carries are how a
 * caller gets back to whichever type it actually came from. `owned` is the
 * discriminator, and it is what decides whether a row may offer a control at
 * all: a run Gritty started can be cancelled, a session it merely watched
 * cannot, and a row that offered otherwise would be lying about the boundary.
 *
 * **Rows are flagged, not filtered.** `elsewhere` marks a session running
 * outside the open workspace and `retired` one that is quietly over; the
 * section shows neither by default and reveals both behind one line. Filtering
 * them out here instead would have made the sidebar the only list that shows a
 * machine-wide session and then silently dropped them when the rail was
 * deleted — a capability lost to a default, which is the kind of thing that is
 * never noticed until it is needed.
 *
 * The island keeps `AgentRoster.needsYou` and this module never recomputes it,
 * so the two surfaces cannot report different totals.
 */
export interface RosterRow {
  /** Unique across both sources, so a run and a session cannot collide. */
  readonly key: string;
  /**
   * Whether Gritty started this. The discriminator the whole module exists to
   * preserve, and the gate on every control a row draws.
   */
  readonly owned: boolean;
  /** The short handle in the row's first line — a run's id, a session's folder. */
  readonly id: string;
  /** What the agent is working on. `null` when it has published nothing. */
  readonly title: string | null;
  /** The second line: what it is doing right now, not what it is for. */
  readonly detail: string | null;
  readonly status: RunStatus;
  readonly badge: RosterBadge | null;
  /** Stopped, with the user holding it up. What sorts a row to the top. */
  readonly needsYou: boolean;
  /**
   * Over, and uneventfully so: a session whose process is gone, a run that
   * finished or was cancelled.
   *
   * **`FAILED` and `INTERRUPTED` are deliberately not retired.** "The agent
   * broke" and "we stopped being able to watch it" are outcomes the user has
   * to see; folding them in with the ordinary dead would hide the two states
   * the run machine exists to distinguish.
   */
  readonly retired: boolean;
  /** Running outside the open workspace. Always false for a run Gritty owns. */
  readonly elsewhere: boolean;
  /**
   * Where an observed session's status came from (T221 §24) — three sources
   * can disagree, and `BLOCKED` is the one status the user is meant to act on.
   * `null` for a run Gritty owns: its state is a transition the core accepted,
   * not a reading of evidence, so there is no source to attribute it to.
   */
  readonly sourceLabel: string | null;
  /** Present only on an owned row: what `runs.cancel` is called with. */
  readonly runId: AgentRunId | null;
  /** Present only on an observed row: what `agents.openTimeline` follows. */
  readonly sessionId: string | null;
}

/**
 * The pill at the right of a row.
 *
 * `attention` is the amber the header count and the status bar already use for
 * the same fact, so an ask reads as one thing across the three places it
 * appears rather than three unrelated highlights.
 */
export interface RosterBadge {
  readonly text: string;
  readonly tone: 'attention' | 'quiet';
}

/**
 * Rows for the section, needs-you first.
 *
 * `runs` is expected in `selectRuns` order (newest first) and `roster.sessions`
 * in the core's own ranking, which already puts live work above quiet work.
 * Neither is re-sorted; the sort below is stable, so both source orders survive
 * inside their group. Re-ranking the sessions here would be the renderer
 * second-guessing a decision the core is documented as owning.
 */
export function rosterRows(
  runs: readonly AgentRun[],
  roster: AgentRoster,
  workstreams: readonly Workstream[],
): RosterRow[] {
  const titles = new Map(workstreams.map((workstream) => [workstream.id, workstream.title]));
  const rows = [...runs.map((run) => runRow(run, titles)), ...roster.sessions.map(sessionRow)];
  // Stable, so within a rank the two source orders are preserved and an owned
  // row precedes an observed one only because it was placed first above.
  return rows.sort((a, b) => rank(a) - rank(b) || Number(b.owned) - Number(a.owned));
}

/**
 * Three bands, not a score: something wants you, something is alive, something
 * is over. A finer ordering would be inventing precision the two sources cannot
 * agree on — a run has `updatedAt` and a session may have no timestamp at all.
 */
function rank(row: RosterRow): number {
  if (row.needsYou) return 0;
  return row.status.tone === 'ended' || row.status.tone === 'failed' ? 2 : 1;
}

function runRow(run: AgentRun, titles: ReadonlyMap<string, string>): RosterRow {
  const status = runStatus(run.state);
  return {
    key: `run:${run.id}`,
    owned: true,
    // Short like a commit hash, for the same reason: the full id is a uuid
    // nobody reads, and the row needs a handle it can say in one word.
    id: run.id.slice(0, 7),
    // The workstream is the work (`docs/architecture/workstreams.md`); a run
    // brings no task model of its own, so its title is the workstream's.
    title: titles.get(run.workstreamId) ?? null,
    detail: activityLine(run),
    status,
    badge: runBadge(run),
    needsYou: needsAttention(run.state),
    // A run belongs to the workspace it was started against, so there is no
    // such thing as one from elsewhere.
    retired: run.state === 'FINISHED' || run.state === 'CANCELLED',
    elsewhere: false,
    sourceLabel: null,
    runId: run.id,
    sessionId: null,
  };
}

function sessionRow(session: AgentSession): RosterRow {
  return {
    key: `session:${session.id}`,
    owned: false,
    // What the core groups the roster by, and what the user recognises: the
    // last component of the working directory.
    id: session.rootName ?? session.id.slice(0, 7),
    title: taskLine(session),
    detail: toolLine(session),
    status: agentStatus(session.activity),
    badge: session.pending ? { text: 'asks', tone: 'attention' } : null,
    // The core's own definition of the count it publishes, applied to one
    // session. Not a second policy: `roster.needsYou` stays the total.
    needsYou: session.activity === 'BLOCKED' || session.activity === 'WAITING',
    retired: session.activity === 'ENDED',
    elsewhere: !session.inWorkspace,
    sourceLabel: activitySourceLabel(session.activitySource),
    runId: null,
    sessionId: session.id,
  };
}

/**
 * `Editing src/auth/session.ts` — the activity alone, without the state.
 *
 * `describeRun` in `runs-reducer.ts` composes the same two halves into one
 * line for the dev panel. A row cannot use it: the state is drawn as a status
 * word and a coloured marker, so repeating it in the text below would say the
 * same thing twice. The two therefore agree on the shape and not the code, and
 * the three-line title-casing is duplicated rather than reached for across a
 * ticket boundary.
 *
 * Every part is a string the core already bounded and collapsed; nothing here
 * parses, resolves or interprets one (hard rule 13). The command is preferred
 * over the path because it says more about what is happening than the file it
 * happens to touch.
 */
export function activityLine(run: AgentRun): string | null {
  const { activity } = run;
  if (!activity) return null;
  const subject = activity.command ?? activity.path?.display ?? null;
  const kind = titleCase(activity.kind);
  return subject ? `${kind} ${subject}` : kind;
}

/**
 * What the row's pill says, in the order the user most needs to know it.
 *
 * A requested cancel outranks an ask: it is the more recent intent, and a row
 * that still said `asks` while it was being torn down would be inviting an
 * answer to a question that no longer has anywhere to go.
 */
function runBadge(run: AgentRun): RosterBadge | null {
  if (run.cancellationRequested && !isTerminalRun(run.state)) {
    return { text: 'cancelling', tone: 'quiet' };
  }
  if (run.state === 'NEEDS_DECISION' || run.state === 'BLOCKED') {
    return { text: 'asks', tone: 'attention' };
  }
  if (run.state === 'REVIEW') return { text: 'review', tone: 'attention' };
  return null;
}

/** `NEEDS_DECISION` → `Needs decision`. Presentation only. */
function titleCase(kind: string): string {
  const words = kind.toLowerCase().replace(/_/g, ' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * What the section draws, and what it keeps behind the reveal.
 *
 * Live work in this workspace is what the section is for; everything else is
 * still reachable, one click away, rather than gone. A row that needs you is
 * never hidden — not even a retired one, which cannot happen today but would
 * be the wrong thing to leave to chance.
 */
export function partitionRows(rows: readonly RosterRow[]): {
  shown: RosterRow[];
  hidden: RosterRow[];
} {
  const shown: RosterRow[] = [];
  const hidden: RosterRow[] = [];
  for (const row of rows) {
    if (!row.needsYou && (row.retired || row.elsewhere)) hidden.push(row);
    else shown.push(row);
  }
  return { shown, hidden };
}
