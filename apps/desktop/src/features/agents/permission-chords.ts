import type { AgentPermissionVerdict } from '@gritty/protocol';

/**
 * The three keys artboard 12a puts on the permission card: `⏎` allows once,
 * `⌘⏎` allows for the run, `esc` denies.
 *
 * Matched on `event.code`, never `event.key`, for the reason
 * `agent-rail-chords.ts` and `terminal-chords.ts` both give: on macOS a
 * modifier composes, so `key` is whatever character came out the other side.
 * `code` is the physical key and is what a chord is actually about.
 *
 * Pure and separate from the component so the mapping is a test rather than a
 * click — `apps/desktop`'s vitest project is a node one, with no DOM to press a
 * key in (the `JSDOM-VITEST` ticket is what would change that).
 */
export interface PermissionChord {
  readonly code: string;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
}

/**
 * The verdict a keystroke means, or `null` for one that means nothing here.
 *
 * `scoped` says whether "allow for this run" is on offer: the core only sends a
 * `suggestedScope` when it can name what such a grant would cover, and without
 * one there is no middle answer — so `⌘⏎` must do nothing rather than quietly
 * fall back to allowing once, which is a different decision.
 */
export function permissionVerdictFor(
  event: PermissionChord,
  scoped: boolean,
): AgentPermissionVerdict | null {
  if (event.altKey || event.shiftKey) return null;
  if (event.code === 'Escape') {
    return event.metaKey || event.ctrlKey ? null : 'DENY';
  }
  if (event.code !== 'Enter' && event.code !== 'NumpadEnter') return null;
  if (event.metaKey || event.ctrlKey) return scoped ? 'ALLOW_FOR_RUN' : null;
  return 'ALLOW_ONCE';
}

/** What the card prints beside each answer. */
export const PERMISSION_CHORD_HINT = {
  ALLOW_ONCE: '⏎',
  ALLOW_FOR_RUN: '⌘⏎',
  DENY: 'esc',
} as const satisfies Record<AgentPermissionVerdict, string>;
