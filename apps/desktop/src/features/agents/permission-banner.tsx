import type { AgentRun, AgentSession, Workstream } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { askingRuns, runAskCard } from '../runs/run-permission';
import { taskLine } from './agents-reducer';
import { type PermissionCard, verbLabel } from './permission-model';

/** The chord artboard 12b prints on the banner and on the header count. */
export const PERMISSION_QUEUE_CHORD = '⌥⌘A';

/**
 * Whether a keystroke is the one that opens the queue on its permissions.
 *
 * `⌥⌘A`, and deliberately not `⌥A`: that one already toggles the island, and
 * every existing chord matcher in this app rejects `metaKey`, so the two cannot
 * collide. Matched on `event.code` for the usual macOS reason — Option composes,
 * so `event.key` for ⌥A is `å`.
 */
export function isPermissionQueueChord(event: {
  code: string;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
}): boolean {
  if (!event.altKey || event.shiftKey) return false;
  if (!(event.metaKey || event.ctrlKey)) return false;
  return event.code === 'KeyA';
}

/** Every session stopped at a prompt, oldest ask first. */
export function askingSessions(sessions: readonly AgentSession[]): AgentSession[] {
  return sessions
    .filter((session) => session.pending)
    .sort(
      (left, right) => Number(left.pending?.askedAt ?? 0) - Number(right.pending?.askedAt ?? 0),
    );
}

/** One ask waiting on the user, whichever lane it came from. */
export interface WaitingAsk {
  readonly askId: string;
  /** How the banner names the asker: a session's task, a run's workstream. */
  readonly who: string;
  readonly request: PermissionCard;
}

/**
 * Every ask waiting on the user — observed sessions' (T335) and owned runs'
 * (T310-ANSWER) — oldest first.
 *
 * One list rather than two because the banner, the header count and the status
 * bar each say one number and name one oldest ask, and a run's ask held up for
 * five minutes is older than a session's from ten seconds ago whichever lane
 * either is in.
 */
export function waitingAsks(
  sessions: readonly AgentSession[],
  runs: readonly AgentRun[] = [],
  workstreams: readonly Workstream[] = [],
): WaitingAsk[] {
  const titles = new Map(workstreams.map((workstream) => [workstream.id as string, workstream]));
  const fromSessions = askingSessions(sessions).flatMap((session) =>
    session.pending
      ? [
          {
            askId: session.pending.askId,
            who: taskLine(session) ?? session.id,
            request: session.pending,
          },
        ]
      : [],
  );
  const fromRuns = askingRuns(runs).map((run) => ({
    askId: run.pending.askId,
    who: titles.get(run.workstreamId as string)?.title ?? run.id.slice(0, 8),
    request: runAskCard(run.pending),
  }));
  return [...fromSessions, ...fromRuns].sort(
    (left, right) => Number(left.request.askedAt) - Number(right.request.askedAt),
  );
}

/**
 * A background agent needs you (T335, artboard 12b).
 *
 * One line above the editor, and the restraint is the design: *a background ask
 * never steals focus*. It is not a modal, it does not take the keyboard, and it
 * can be dismissed with Later — the count in the header and the row in the rail
 * are still there afterwards, so dismissing it loses nothing.
 *
 * The banner is deliberately not the place an ask is answered. It names the
 * oldest one and offers the queue, because judging an ask means reading a diff
 * and a blast radius, and a single line has room for neither.
 */
export function PermissionBanner({
  sessions,
  runs = [],
  workstreams = [],
  onOpenQueue,
}: {
  sessions: readonly AgentSession[];
  /** Runs Gritty owns; their asks count and can be the oldest (T310-ANSWER). */
  runs?: readonly AgentRun[];
  /** What names a run: its workstream's title. */
  workstreams?: readonly Workstream[];
  onOpenQueue(): void;
}) {
  const asking = waitingAsks(sessions, runs, workstreams);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const oldest = asking[0] ?? null;
  const askId = oldest?.askId ?? null;

  // Dismissal is per ask, not for ever: a *new* prompt is a new interruption
  // and has to be able to say so, or "Later" would quietly turn the feature off.
  useEffect(() => {
    if (askId && dismissed && askId !== dismissed) setDismissed(null);
  }, [askId, dismissed]);

  if (!oldest || dismissed === askId) return null;
  const { request } = oldest;

  return (
    <div className="permission-banner" role="status">
      <span className="permission-banner__edge" aria-hidden="true" />
      <span className="permission-banner__count">
        {asking.length === 1 ? '1 agent needs' : `${asking.length} agents need`} permission
      </span>
      <span className="permission-banner__who">{oldest.who}</span>
      <span className="permission-banner__what">
        {verbLabel(request.verb)} {request.target}
      </span>
      {request.detail && <span className="permission-banner__aside">· {request.detail}</span>}
      <span className="permission-banner__spacer" />
      <button type="button" className="btn--primary permission-banner__open" onClick={onOpenQueue}>
        Review queue <kbd>{PERMISSION_QUEUE_CHORD}</kbd>
      </button>
      <button type="button" className="btn--bare" onClick={() => setDismissed(askId)}>
        Later
      </button>
    </div>
  );
}
