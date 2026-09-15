import type { WorkstreamId } from '@gritty/protocol';

/**
 * The launcher's rules, apart from its markup (RUN-LAUNCHER).
 *
 * Pure so they can be tested without a DOM, the way `raise-question-model.ts`
 * holds the island's form rules.
 */

/**
 * `MAX_PROMPT_TEXT` in `crates/gritty-core/src/agent_run/request.rs`. The core
 * refuses a longer prompt rather than clipping it — it is about to be sent to a
 * model and paid for — so the field stops there instead of letting the refusal
 * be the first anyone hears of it.
 */
export const MAX_PROMPT = 16 * 1024;

/**
 * How long a title made from a prompt may be. The core accepts 200
 * (`workstream/wire.rs`), but a title is a label on a row: a paragraph there is
 * a prompt pasted into the wrong field, not a name.
 */
export const DERIVED_TITLE_LENGTH = 80;

/** The workstream select's value for "make one from the prompt". */
export const NEW_WORKSTREAM = '';

/** What the form hands up: the prompt, and where the run should belong. */
export interface LaunchRequest {
  readonly prompt: string;
  /** `null` asks for a workstream to be created from the prompt. */
  readonly workstreamId: WorkstreamId | null;
}

/**
 * A prompt the core would take: not blank, and not longer than it allows.
 * Counted in characters, as the core counts them (`chars().count()`).
 */
export function canLaunch(prompt: string): boolean {
  return prompt.trim().length > 0 && [...prompt].length <= MAX_PROMPT;
}

/**
 * A workstream title from what the user asked for: the first line that says
 * anything, with its whitespace collapsed, clipped with an ellipsis.
 *
 * Clipped by code point rather than by UTF-16 unit, so an emoji at the boundary
 * is dropped whole rather than split into a lone surrogate the core would store.
 */
export function titleFromPrompt(prompt: string): string {
  const line =
    prompt
      .split(/\r?\n/)
      .map((text) => text.replace(/\s+/g, ' ').trim())
      .find((text) => text.length > 0) ?? '';
  const points = [...line];
  if (points.length <= DERIVED_TITLE_LENGTH) return line;
  return `${points
    .slice(0, DERIVED_TITLE_LENGTH - 1)
    .join('')
    .trimEnd()}…`;
}
