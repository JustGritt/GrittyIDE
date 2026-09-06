import type { DecisionOption } from '@gritty/protocol';

/**
 * Turning what someone typed into a question the core will accept (T210).
 *
 * Pure, so the one rule with any subtlety in it — where an option's `key` comes
 * from — is tested without a DOM. The form above this only collects strings.
 *
 * **This is not validation.** The core is the authority on what a decision may
 * be (`validate_options` in `crates/gritty-core/src/workstream/wire.rs`), and a
 * refusal comes back as the error envelope both surfaces already draw. What
 * happens here is the shaping the core cannot do for us: a person types an
 * answer, and the wire wants an answer *and* a stable identifier for it.
 */

/**
 * Mirrors of the core's bounds, used to stop a field before it is sent rather
 * than to decide whether it is legal. They are `maxLength` on an input and the
 * point at which "add another option" stops — a cheaper refusal than a round
 * trip, and never a second opinion: if these and the core ever disagree, the
 * core wins and its message is what the user sees.
 */
export const MAX_QUESTION = 4_000;
export const MAX_OPTIONS = 12;
export const MAX_OPTION_LABEL = 200;
/** The core's `MAX_OPTION_KEY`; what `slugify` truncates to. */
const MAX_OPTION_KEY = 64;

/**
 * A stable identifier for an option, derived from its label.
 *
 * One field per option rather than two. `key` is what `chosen` records, so it
 * has to be stable and meaningful — but making someone type it beside every
 * label is friction in a form whose whole purpose is to be quick, and the label
 * is already the thing they are thinking about.
 *
 * Lowercase, ASCII-ish, hyphen-joined. A label that reduces to nothing at all —
 * an emoji, punctuation alone — is not an error: it gets a positional key, so
 * the option still works and the person who typed it is not lectured about
 * character classes.
 */
export function slugify(label: string, position: number): string {
  const slug = label
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, MAX_OPTION_KEY);
  return slug === '' ? `option-${position + 1}` : slug;
}

/**
 * The options a raise request carries, from the labels a person typed.
 *
 * Blank rows are dropped rather than refused: an empty row is someone who
 * clicked "add" and changed their mind, and failing the whole submission over
 * it would be the form being pedantic about its own affordance.
 *
 * Keys are de-duplicated here even though the core rejects duplicates, because
 * two options can honestly slug to the same key ("Yes, do it" and "Yes! Do
 * it.") and turning that into a refusal the user has to decode would be worse
 * than making the second one `yes-do-it-2`.
 */
export function optionsFrom(labels: readonly string[]): DecisionOption[] {
  const options: DecisionOption[] = [];
  const taken = new Set<string>();
  for (const raw of labels) {
    const label = raw.trim();
    if (label === '') continue;
    const base = slugify(label, options.length);
    let key = base;
    // Bounded by `labels.length`: each pass takes a key that is then in `taken`.
    for (let suffix = 2; taken.has(key); suffix += 1) {
      key = `${base.slice(0, MAX_OPTION_KEY - 3)}-${suffix}`;
    }
    taken.add(key);
    options.push({ key, label });
  }
  return options;
}

/**
 * Whether the form has enough to send: a question with something in it.
 *
 * Options are optional — a free-form question is a first-class kind of
 * decision, and `DecisionCard` already answers one with a text field.
 */
export function canRaise(question: string): boolean {
  return question.trim() !== '';
}
