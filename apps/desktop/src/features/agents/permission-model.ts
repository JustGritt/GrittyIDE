import type { AgentPermissionRequest, AgentPermissionVerb } from '@gritty/protocol';

/**
 * What a permission card says, in one place (T335, artboards 12a and 12b).
 *
 * Pure, because the inline card and the queue card must never disagree about
 * what an ask *is* — they differ only in how much room they have to say it. The
 * same rule `attention.ts` follows for the island and the queue.
 *
 * Nothing here decides anything: the core already classified the verb, bounded
 * every string and worked out whether the target is outside the session's root.
 * This only phrases it.
 */

/** The verb, as a person reads it. */
const VERB_LABEL: Record<AgentPermissionVerb, string> = {
  WRITE_FILE: 'write file',
  RUN_COMMAND: 'run command',
  INSTALL_DEPENDENCY: 'install dependency',
  OTHER: 'use tool',
};

export function verbLabel(verb: AgentPermissionVerb): string {
  return VERB_LABEL[verb] ?? VERB_LABEL.OTHER;
}

/**
 * The line under the header: where this reaches beyond its target.
 *
 * Assembled from what the core actually knows and nothing else. "Outside its
 * worktree" is the fact that changes how an answer should go, so it comes
 * first; the adapter's own blast-radius line follows it; a redacted target says
 * so out loud, because a card with no diff and no explanation reads as a card
 * with nothing to show.
 */
export function blastRadius(request: AgentPermissionRequest): string[] {
  const parts: string[] = [];
  if (request.outsideRoot) parts.push('outside its worktree');
  if (request.detail) parts.push(request.detail);
  if (request.sensitive) parts.push('contents not shown — this file usually holds secrets');
  return parts;
}

/**
 * `+64 −11 · 1 file`, or nothing when the change has no size to report.
 *
 * The counts describe the whole change even though the diff above them is
 * clipped, which is the point of showing both: the numbers say what is
 * happening, and their disagreement with the visible lines says you are not
 * seeing all of it.
 */
export function changeSummary(request: AgentPermissionRequest): string | null {
  if (request.verb !== 'WRITE_FILE') return null;
  if (request.added === 0 && request.removed === 0) return null;
  return `+${request.added} −${request.removed} · 1 file`;
}

/** What the middle answer offers, or `null` when there is no honest scope. */
export function scopeLabel(request: AgentPermissionRequest): string | null {
  if (!request.suggestedScope) return null;
  return request.verb === 'WRITE_FILE'
    ? `Allow writes in ${request.suggestedScope} for this run`
    : `Allow ${request.suggestedScope} for this run`;
}

/** The sign a diff line carries, for the class that colours it. */
export type DiffSign = 'add' | 'remove' | 'context';

export function diffSign(line: string): DiffSign {
  if (line.startsWith('+')) return 'add';
  if (line.startsWith('-')) return 'remove';
  return 'context';
}

/** The line without its sign. Never trimmed: leading space is indentation. */
export function diffText(line: string): string {
  return line.slice(1);
}

/**
 * How long an ask has been waiting, as the card prints it.
 *
 * Seconds under a minute, because a permission prompt is usually answered in
 * that window and "0m" would read as broken. Above it, `waitedFor` from
 * `features/workstreams/attention` already says what the queue says, and this
 * defers to it rather than inventing a second vocabulary.
 */
export function askedAgo(askedAt: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - askedAt) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
