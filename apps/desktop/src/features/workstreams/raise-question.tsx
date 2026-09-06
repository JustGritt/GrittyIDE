import type { WorkstreamId } from '@gritty/protocol';
import { type FormEvent, useId, useState } from 'react';
import {
  canRaise,
  MAX_OPTION_LABEL,
  MAX_OPTIONS,
  MAX_QUESTION,
  optionsFrom,
} from './raise-question-model';
import type { WorkstreamsController } from './use-workstreams';

/**
 * Asking a question against a piece of work (T210).
 *
 * The gap this closes: `decisions.raise` existed on the controller and in the
 * core from the day T200 landed, and no component called it. Decisions could be
 * answered and dismissed, but the only way to *create* one was the command — so
 * the island could not be walked end to end, and manual-pass step 7.7 ("open
 * the island onto a decision with options, press ⌥1") could not be performed at
 * all.
 *
 * Shared by the island's roster and the queue's detail rail, the way
 * `DecisionCard` is, so the two surfaces cannot drift into asking questions
 * differently.
 *
 * **Every field here is an `<input>`, never a `<textarea>`.** `isTypingInField`
 * in `island.tsx` tests `tagName === 'INPUT'`, and deliberately does not exclude
 * Monaco's hidden textarea — ⌥A from the editor is the point of that shortcut.
 * A textarea in this form would therefore let ⌥A and ⌥1…9 fire in the middle of
 * a sentence someone is typing.
 */
export function RaiseQuestion({
  workstreamId,
  controller,
  onDone,
}: {
  workstreamId: WorkstreamId;
  controller: WorkstreamsController;
  /** Called once the core accepted it, so the island can fold the form away. */
  onDone?(): void;
}) {
  const [question, setQuestion] = useState('');
  // One entry per option row. Empty rows are dropped on submit rather than
  // refused: an empty row is someone who clicked "add" and changed their mind.
  const [labels, setLabels] = useState<string[]>([]);
  const questionId = useId();

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canRaise(question)) return;
    const decision = await controller.raise(workstreamId, question.trim(), optionsFrom(labels));
    // Only on success. A refusal — the workstream was removed under us, a
    // question the core would not take — leaves what was typed where it is,
    // beside the envelope that says why.
    if (!decision) return;
    setQuestion('');
    setLabels([]);
    onDone?.();
  }

  return (
    <form className="raise" onSubmit={(event) => void submit(event)}>
      <label className="raise__label" htmlFor={questionId}>
        Ask a question
      </label>
      <div className="raise__question">
        <input
          id={questionId}
          value={question}
          onChange={(event) => setQuestion(event.target.value)}
          placeholder="What do you need decided?"
          maxLength={MAX_QUESTION}
          spellCheck={false}
        />
        <button
          type="submit"
          className="btn--primary"
          disabled={!canRaise(question) || controller.busy}
        >
          Ask
        </button>
      </div>

      {labels.length > 0 && (
        <ul className="raise__options">
          {labels.map((label, index) => (
            // The index is the identity: these rows have no id of their own,
            // and reordering is not offered.
            // biome-ignore lint/suspicious/noArrayIndexKey: the row's position is what identifies it
            <li key={index} className="raise__option">
              <span className="raise__chord">{index + 1}</span>
              <input
                value={label}
                onChange={(event) =>
                  setLabels((current) =>
                    current.map((existing, at) => (at === index ? event.target.value : existing)),
                  )
                }
                placeholder={`Option ${index + 1}`}
                aria-label={`Option ${index + 1}`}
                maxLength={MAX_OPTION_LABEL}
              />
              <button
                type="button"
                className="btn--bare"
                aria-label={`Remove option ${index + 1}`}
                onClick={() => setLabels((current) => current.filter((_, at) => at !== index))}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="raise__foot">
        <button
          type="button"
          className="btn--bare"
          // The core's bound, so the button stops where a refusal would start.
          disabled={labels.length >= MAX_OPTIONS}
          onClick={() => setLabels((current) => [...current, ''])}
        >
          Add an option
        </button>
        <span className="island__spacer" />
        {/* Says what the two shapes of question mean, because the difference is
            a core rule the user did not write: options become the ⌥1…9 answers
            in the island, and no options means a free-text reply. */}
        <span className="raise__hint">
          {labels.length === 0 ? 'No options: answered in words' : 'Answered by picking one'}
        </span>
      </div>
    </form>
  );
}
