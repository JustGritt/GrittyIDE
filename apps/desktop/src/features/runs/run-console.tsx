import type { AgentRun } from '@gritty/protocol';
import { type KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { isPinnedToBottom } from '../agents/agent-timeline-model';
import { runStatus } from '../agents/run-status';
import { promptGate, type RunFeedRow, runFeedRows } from './run-console-model';
import type { RunsController } from './use-runs';

/**
 * One run Gritty **owns**, in the editor area (T310-U).
 *
 * `AgentConsole`'s sibling, and deliberately its twin down to the class names:
 * the two show different sources and there is no reason for them to look
 * different. What separates them is the boundary
 * `docs/architecture/agent-runs.md` draws, and it is the whole reason this is a
 * second component rather than a flag on the first:
 *
 * | | `AgentConsole` | this |
 * |---|---|---|
 * | Who started it | some other terminal | Gritty |
 * | Where the feed comes from | a transcript the core follows | the event ring the core recorded |
 * | What the user may do to it | answer a permission ask (ADR-020) | say something to it |
 *
 * The prompt box is the point of the surface, and it is the one control in the
 * app that speaks *to* an agent. It is drawn only where there is a channel
 * behind it: never on an observed session, and not on a scripted run either
 * (§9, `promptGate`).
 *
 * Everything drawn here came out of an agent and is untrusted (hard rule 13).
 * The core bounded and whitespace-collapsed every field at the ingest seam;
 * this component's whole contribution is to put each string in a text node and
 * nowhere else. There is no `dangerouslySetInnerHTML` in this file, no
 * Markdown, and no path anything opens.
 */
export function RunConsole({
  run,
  title,
  controller,
  pending,
}: {
  run: AgentRun;
  /**
   * The workstream's title. A run brings no task model of its own
   * (`docs/architecture/workstreams.md`): the workstream is the work.
   */
  title: string | null;
  controller: RunsController;
  /** True while `runs.events` is in flight, so "empty" and "not yet" differ. */
  pending: boolean;
}) {
  const status = runStatus(run.state);
  const log = controller.logFor(run.id);
  const rows = useMemo(() => runFeedRows(log), [log]);
  const gate = promptGate(run);

  return (
    <section className="agent-console" aria-label={`Agent run ${run.id}`}>
      <header className="agent-console__bar">
        {/* Identity, then status: which run, and how it is doing. The same
            two-channel encoding the sidebar rows and the terminal tabs use. */}
        <span className="agent-console__identity" aria-hidden="true" />
        <span className={`agent-status agent-status--${status.tone}`} title={status.hint}>
          <span className="agent-status__mark" aria-hidden="true" />
          {status.label}
        </span>
        {/* Where an observed session's console says which of three disagreeing
            sources produced its verdict (T221 §24), this says the one thing a
            run's state cannot be mistaken about: Gritty started it, so the
            state is a transition the core accepted rather than a reading of
            evidence. There is no source to attribute. */}
        <span className="agent-console__source">Gritty started this run</span>
        <span className="agent-console__spacer" />
        <span className="agent-console__meta">{run.provider}</span>
        <code className="agent-console__meta">{run.id.slice(0, 8)}</code>
      </header>

      {/* The workstream is the work, on its own line for the same reason the
          session console gives the task one: it can be long, and it wraps here
          instead of pushing the status off the end. */}
      <p className="agent-console__task">
        {title ?? <em className="agent-console__unknown">No workstream title</em>}
      </p>

      {/* How the run itself broke, which is not a feed row: `RUN_FAILED` may
          have fallen out of the ring, and the snapshot's error is the durable
          answer to "how did it end". */}
      {run.error && (
        <p className="agent-console__note">
          {run.error.code}: {run.error.message}
        </p>
      )}

      {controller.error && (
        <div className="agent-console__error">
          <ErrorEnvelope error={controller.error} onDismiss={controller.clearError} />
        </div>
      )}

      <section className="agent-console__timeline" aria-label="Run feed">
        <div className="agent-console__timeline-head">
          <span className="agent-console__timeline-title">
            {run.activity ? activityText(run) : 'What this run reported'}
          </span>
          <span className="agent-console__spacer" />
          <span className={`agent-status agent-status--${status.tone}`} title={status.hint}>
            <span className="agent-status__mark" aria-hidden="true" />
            {status.label}
          </span>
        </div>

        {rows.length === 0 ? (
          pending ? (
            <p className="agent-console__placeholder">Opening…</p>
          ) : (
            <p className="agent-console__placeholder">
              Nothing reported yet. What this run does appears here as it works.
            </p>
          )
        ) : (
          <RunFeed key={run.id} rows={rows} />
        )}
      </section>

      {/* Below the feed (§9). Nothing optimistic lives above it: what the user
          typed reaches the feed when `PROMPT_STARTED` arrives, which is the run
          confirming it. */}
      <PromptBox run={run} gate={gate} controller={controller} />
    </section>
  );
}

/**
 * The feed itself, oldest event last, in its own scroll container.
 *
 * `AgentTimelinePanel`'s `Feed`, with the permission re-pin removed — a run's
 * ask is a `PERMISSION_REQUESTED` row like any other until T310-ANSWER gives it
 * something to be answered with, and re-pinning to a row nobody can act on
 * would be yanking the view for nothing.
 */
function RunFeed({ rows }: { rows: readonly RunFeedRow[] }) {
  const feed = useRef<HTMLDivElement | null>(null);
  /**
   * Whether new events may scroll the view.
   *
   * A ref, not state: it is read by an effect and written by a scroll handler,
   * and re-rendering on every scroll frame to store a boolean nothing draws
   * would be the opposite of what this is for. It starts pinned and is reset by
   * the `key` the console gives this component, so a different run starts at
   * the bottom of its own feed rather than where the last one was left.
   */
  const pinned = useRef(true);

  // Follow new events down, but only from the bottom. Someone who scrolled up
  // to read something is left exactly where they are. Deliberately on every
  // render rather than keyed to the rows: the pin means "stay at the bottom",
  // and the row array is not the only thing that can change what the bottom is.
  useEffect(() => {
    const element = feed.current;
    if (element && pinned.current) element.scrollTop = element.scrollHeight;
  });

  const onScroll = useCallback(() => {
    const element = feed.current;
    if (element) pinned.current = isPinnedToBottom(element);
  }, []);

  return (
    <div className="agent-feed" ref={feed} onScroll={onScroll}>
      <ol className="agent-feed__list">
        {rows.map((row) => (
          <FeedRow key={row.key} row={row} />
        ))}
      </ol>
    </div>
  );
}

/** One line of the feed. Every string it draws is a text node. */
function FeedRow({ row }: { row: RunFeedRow }) {
  const command = row.tone === 'tool';
  return (
    <li
      className={`agent-feed__row agent-feed__row--${row.tone}${
        row.outcome === null ? '' : ` agent-feed__row--${row.outcome}`
      }`}
    >
      <span className="agent-feed__head">
        <span className="agent-feed__label">{row.label}</span>
        {command && (
          <span className={`agent-feed__badge agent-feed__badge--${row.outcome}`}>
            {row.outcome === 'fail' ? 'failed' : row.outcome}
          </span>
        )}
        <span className="agent-console__spacer" />
        {/* The gap row is not an event and has no time of its own; a clock on
            it would be a claim about when the loss happened. */}
        {row.tone !== 'gap' && <span className="agent-feed__at">{clockTime(row.at)}</span>}
      </span>
      <p className="agent-feed__text">{row.text}</p>
      {row.detail !== null && <p className="agent-feed__detail">{row.detail}</p>}
    </li>
  );
}

/**
 * Say something to a run that is already running (§9).
 *
 * Three rules, and each is the reason a line of this exists:
 *
 * - **⌘↵ sends**, ⌃↵ too, and a bare ↵ is a newline. A prompt is prose and
 *   often more than one line; sending on ↵ would make a paragraph impossible to
 *   type and cost money every time someone reached for one.
 * - **Nothing optimistic.** The textarea clears when the command is accepted —
 *   the text is queued, which is a fact — but no row is drawn. What was typed
 *   reaches the feed when `PROMPT_STARTED` arrives, and that is the run's own
 *   confirmation (`--replay-user-messages`), not the renderer's guess.
 * - **A refusal names the state in the way.** The box is disabled with the
 *   reason beside it, never silently ignored, and never *absent* while a
 *   channel exists — a control that vanishes is a capability the user cannot
 *   tell they have lost.
 *
 * The draft survives a refusal: a run that goes `BLOCKED` while someone is
 * typing must not also take their sentence with it.
 */
function PromptBox({
  run,
  gate,
  controller,
}: {
  run: AgentRun;
  gate: ReturnType<typeof promptGate>;
  controller: RunsController;
}) {
  const [text, setText] = useState('');
  const ready = gate.kind === 'ready';
  const trimmed = text.trim();
  const sendable = ready && trimmed !== '' && !controller.busy;

  const send = useCallback(() => {
    if (!sendable) return;
    // Cleared before the answer, deliberately: the box is a control and has to
    // answer at once. What is *not* faked is the feed — a refusal surfaces as
    // the envelope above, with the state that was in the way.
    setText('');
    void controller.prompt(run.id, trimmed);
  }, [controller, run.id, sendable, trimmed]);

  // There is no channel and none can be opened, so there is nothing to draw —
  // not even disabled (§9).
  if (gate.kind === 'absent') return null;

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key !== 'Enter' || !(event.metaKey || event.ctrlKey)) return;
    event.preventDefault();
    send();
  }

  return (
    <form
      className="agent-prompt"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <label className="agent-prompt__label" htmlFor={`prompt-${run.id}`}>
        Say something to this run
      </label>
      <textarea
        id={`prompt-${run.id}`}
        className="agent-prompt__input"
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
        disabled={!ready}
        rows={3}
        spellCheck={true}
        placeholder={ready ? 'Another turn… (⌘↵ to send)' : ''}
      />
      <div className="agent-prompt__foot">
        {/* The reason travels as text, never as a disabled control alone: a
            greyed-out box that says nothing is indistinguishable from a broken
            one. */}
        {!ready && <p className="agent-prompt__reason">{gate.reason}</p>}
        <span className="agent-console__spacer" />
        {ready && <span className="agent-prompt__hint">⌘↵ to send</span>}
        <button type="submit" disabled={!sendable}>
          Send
        </button>
      </div>
    </form>
  );
}

/**
 * `Editing src/auth/session.ts` — what the run is doing, without its state.
 *
 * The state is already drawn as a word and a coloured mark beside this line, so
 * repeating it here would say the same thing twice. `activityLine` in
 * `roster-model.ts` composes the same two halves for a sidebar row; the two
 * agree on the shape and not on the code, which is the same call that file
 * makes about `describeRun`.
 */
function activityText(run: AgentRun): string {
  const activity = run.activity;
  if (!activity) return 'What this run reported';
  const subject = activity.command ?? activity.path?.display ?? null;
  const kind = activity.kind.toLowerCase().replace(/_/g, ' ');
  const titled = kind.charAt(0).toUpperCase() + kind.slice(1);
  return subject ? `${titled} ${subject}` : titled;
}

/**
 * The wall-clock time of a record, or an empty string when the number cannot be
 * one. A record's `at` is the core's own stamp, so this should never fail;
 * throwing inside a render over a bad timestamp would take the whole feed with
 * it.
 */
function clockTime(millis: number): string {
  const date = new Date(millis);
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleTimeString();
}
