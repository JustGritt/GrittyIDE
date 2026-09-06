import type { AgentSession } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { taskLine } from './agents-reducer';
import { verbLabel } from './permission-model';

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
  onOpenQueue,
}: {
  sessions: readonly AgentSession[];
  onOpenQueue(): void;
}) {
  const asking = askingSessions(sessions);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const oldest = asking[0] ?? null;
  const askId = oldest?.pending?.askId ?? null;

  // Dismissal is per ask, not for ever: a *new* prompt is a new interruption
  // and has to be able to say so, or "Later" would quietly turn the feature off.
  useEffect(() => {
    if (askId && dismissed && askId !== dismissed) setDismissed(null);
  }, [askId, dismissed]);

  if (!oldest?.pending || dismissed === askId) return null;
  const request = oldest.pending;

  return (
    <div className="permission-banner" role="status">
      <span className="permission-banner__edge" aria-hidden="true" />
      <span className="permission-banner__count">
        {asking.length === 1 ? '1 agent needs' : `${asking.length} agents need`} permission
      </span>
      <span className="permission-banner__who">{taskLine(oldest) ?? oldest.id}</span>
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
