import type { Decision, Workstream } from '@gritty/protocol';
import { type FormEvent, useEffect, useId, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { AgentsSection, agentsPillLabel } from '../agents/agents-panel';
import { verbLabel } from '../agents/permission-model';
import type { AgentsController } from '../agents/use-agents';
import { AgentLinkSection, linkedRowLabel } from './agent-link-panel';
import {
  type AttentionItem,
  attentionItems,
  rosterOrder,
  stateLabel,
  stateTone,
  transitionLabel,
  waitedFor,
} from './attention';
import { ownsLabel, targetLabel } from './ownership';
import { OwnershipSection } from './ownership-panel';
import { RaiseQuestion } from './raise-question';
import type { WorkstreamsController } from './use-workstreams';

/**
 * The island (T210, artboards 5a/5b, placement A from 6a).
 *
 * It lives in the top bar, where the window's identity already is, so it costs
 * no layout: collapsed it is a pill of status dots and two counts, expanded it
 * is a sheet that hangs under the bar and over the editor. It never pushes the
 * tab strip down, and it closes the moment focus goes back to the editor —
 * which is the whole bargain: always visible, never in the way.
 *
 * What it shows is what the domain can honestly produce: work, its state, the
 * questions blocking it (T200) and who owns which files (T325). There are no
 * approvals or failures here, because there are no agent runs yet (T300).
 */
export function AgentIsland({
  controller,
  agents,
  open,
  onOpenChange,
  onOpenQueue,
}: {
  controller: WorkstreamsController;
  /** Sessions other terminals are running (T220), read-only. */
  agents: AgentsController;
  open: boolean;
  onOpenChange(open: boolean): void;
  /** Show the whole queue instead — more room than the sheet has. */
  onOpenQueue(): void;
}) {
  const { workstreams, decisions, ownership } = controller;
  const items = attentionItems(workstreams, decisions, ownership, agents.roster.sessions);
  const sheet = useRef<HTMLDivElement | null>(null);
  const island = useRef<HTMLDivElement | null>(null);
  const labelId = useId();

  // ⌥A from anywhere, which is the shortcut the artboard prints on the header.
  // Not while a field has focus: on macOS Option composes characters, and a
  // name someone is halfway through typing must not open a sheet instead.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.altKey && event.code === 'KeyA' && !event.metaKey && !event.ctrlKey) {
        if (isTypingInField(event.target)) return;
        event.preventDefault();
        onOpenChange(!open);
      }
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onOpenChange]);

  // Collapses on a click outside and on Escape. Typing in the editor requires
  // focus there, so both together are the artboard's "collapses on keystroke"
  // without stealing keys from the editor to detect it.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: PointerEvent) {
      if (!island.current?.contains(event.target as Node)) onOpenChange(false);
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onOpenChange(false);
    }
    window.addEventListener('pointerdown', onPointerDown);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('pointerdown', onPointerDown);
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [open, onOpenChange]);

  useEffect(() => {
    if (open) sheet.current?.focus();
  }, [open]);

  const waiting = items.length;

  return (
    <div className="island" ref={island}>
      <button
        type="button"
        className={`island__pill${waiting > 0 ? ' island__pill--waiting' : ''}`}
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        title="Workstreams (⌥A)"
      >
        <span className="island__dots" aria-hidden="true">
          {rosterOrder(workstreams)
            .slice(0, 4)
            .map((workstream) => (
              <span
                key={workstream.id}
                className={`island__dot island__dot--${stateTone(workstream.state)}`}
              />
            ))}
        </span>
        <span className="island__count">
          {workstreams.length === 0
            ? 'No workstreams'
            : `${workstreams.length} workstream${workstreams.length === 1 ? '' : 's'}`}
        </span>
        {agentsPillLabel(agents) && (
          <span className="island__agents">{agentsPillLabel(agents)}</span>
        )}
        {waiting > 0 && <span className="island__needs">{waiting} needs you</span>}
      </button>

      {open && (
        <div
          className="island__sheet"
          ref={sheet}
          role="dialog"
          aria-labelledby={labelId}
          tabIndex={-1}
        >
          <div className="island__head">
            <span className="island__title" id={labelId}>
              Workstreams
            </span>
            <span className="island__spacer" />
            <button type="button" className="btn--bare" onClick={onOpenQueue}>
              Open the queue
            </button>
            <kbd className="island__chord">⌥A</kbd>
          </div>

          {controller.error && (
            <div className="island__error">
              <ErrorEnvelope error={controller.error} onDismiss={controller.clearError} />
            </div>
          )}

          {/* Oldest first: the thing that has been waiting longest is the one
              worth answering, and answering it here means not leaving the
              editor at all (5b, "answer without leaving the editor"). */}
          {items[0] && <IslandAttention item={items[0]} controller={controller} />}

          <div className="island__roster">
            {workstreams.length === 0 ? (
              <p className="island__empty">
                Nothing yet. A workstream is a piece of work with a goal, a state and the questions
                that block it — later, an agent runs inside one.
              </p>
            ) : (
              rosterOrder(workstreams).map((workstream) => (
                <RosterRow
                  key={workstream.id}
                  workstream={workstream}
                  controller={controller}
                  agents={agents}
                />
              ))
            )}
          </div>

          <NewWorkstream controller={controller} />

          {/* Who may write where (T325). Under the roster, because a claim is
              made on behalf of a workstream that is already in it. */}
          <OwnershipSection controller={controller} />

          {/* Which observed session is working on which of them (T223). Next
              to ownership because both are statements about a workstream made
              from outside it, and neither is control. */}
          <AgentLinkSection controller={controller} agents={agents} />

          {/* Below the user's own work, because it is not the user's own work:
              these are sessions Gritty observes but does not own (T220). */}
          <AgentsSection controller={agents} />
        </div>
      )}
    </div>
  );
}

/** The single most urgent item, rendered in full inside the sheet. */
function IslandAttention({
  item,
  controller,
}: {
  item: AttentionItem;
  controller: WorkstreamsController;
}) {
  if (item.kind === 'conflict') {
    return (
      <div className="island__card island__card--conflict">
        <div className="island__card-head">
          <span className="island__dot island__dot--blocked" aria-hidden="true" />
          <span className="label">Ownership conflict</span>
          <span className="island__name">{item.workstream.title}</span>
          <span className="island__spacer" />
          <span className="island__meta">waiting {waitedFor(item.since, Date.now())}</span>
        </div>
        <div className="island__card-body">
          wants to modify {targetLabel(item.conflict.target)}, owned by {item.owner.title}
        </div>
        <div className="island__card-actions">
          <button
            type="button"
            disabled={controller.busy}
            onClick={() => void controller.resolveConflict(item.conflict.id, 'CANCEL')}
          >
            Withdraw
          </button>
          <span className="island__spacer" />
          {/* Never silent: taking a lease off its holder is a click with a
              name on it, and the event carries who made it. */}
          <button
            type="button"
            className="btn--bare"
            disabled={controller.busy}
            onClick={() => void controller.resolveConflict(item.conflict.id, 'OVERRIDE')}
          >
            Release {item.owner.title}'s claim
          </button>
        </div>
      </div>
    );
  }
  if (item.kind === 'permission') {
    // The island is a strip in the top bar; a permission ask needs a diff, a
    // blast radius and three answers, and artboard 12b gives it the queue for
    // exactly that reason. So this says what is waiting and where it is
    // answered, rather than offering a cramped copy of the card — and the count
    // beside it is what actually earns the strip.
    return (
      <div className="island__card island__card--permission">
        <div className="island__card-head">
          <span className="island__dot island__dot--blocked" aria-hidden="true" />
          <span className="label">Permission</span>
          <span className="island__spacer" />
          <span className="island__meta">{waitedFor(item.since, Date.now())}</span>
        </div>
        <div className="island__card-body">
          {verbLabel(item.request.verb)} · {item.request.target}
        </div>
        <p className="island__meta">
          Answer it in the queue, where the change it would make is shown in full.
        </p>
      </div>
    );
  }
  if (item.kind === 'review') {
    return (
      <div className="island__card island__card--review">
        <div className="island__card-head">
          <span className="island__dot island__dot--waiting" aria-hidden="true" />
          <span className="label">Ready for review</span>
          <span className="island__spacer" />
          <span className="island__meta">{waitedFor(item.since, Date.now())}</span>
        </div>
        <div className="island__card-body">{item.workstream.title}</div>
        <div className="island__card-actions">
          <button
            type="button"
            className="btn--primary"
            disabled={controller.busy}
            onClick={() => void controller.transition(item.workstream.id, 'DONE')}
          >
            Accept
          </button>
          <button
            type="button"
            disabled={controller.busy}
            onClick={() => void controller.transition(item.workstream.id, 'IN_PROGRESS')}
          >
            Send back
          </button>
        </div>
      </div>
    );
  }
  return (
    <DecisionCard decision={item.decision} workstream={item.workstream} controller={controller} />
  );
}

/**
 * A question and its answers. Shared by the island and the queue, because
 * artboard 5b's rule is that they are the same item: resolving one clears the
 * other. Here that is free — both render the core's decision.
 */
export function DecisionCard({
  decision,
  workstream,
  controller,
  chords = true,
}: {
  decision: Decision;
  workstream: Workstream;
  controller: WorkstreamsController;
  /** ⌥1…⌥9 answer the options. Off in the queue, where a list is in focus. */
  chords?: boolean;
}) {
  const [answer, setAnswer] = useState('');
  const freeForm = decision.options.length === 0;

  useEffect(() => {
    if (!chords || freeForm) return;
    function onKeyDown(event: KeyboardEvent) {
      if (!event.altKey || event.metaKey || event.ctrlKey) return;
      // An answer is a decision, not a keystroke escaping a field.
      if (isTypingInField(event.target)) return;
      const index = Number.parseInt(event.key, 10) - 1;
      const option = decision.options[index];
      if (!option) return;
      event.preventDefault();
      void controller.resolve(decision.id, option.key);
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [chords, freeForm, decision, controller]);

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (answer.trim()) void controller.resolve(decision.id, answer.trim());
  }

  return (
    <div className="island__card island__card--question">
      <div className="island__card-head">
        <span className="island__dot island__dot--blocked" aria-hidden="true" />
        <span className="label">Question</span>
        <span className="island__name">{workstream.title}</span>
        <span className="island__spacer" />
        <span className="island__meta">waiting {waitedFor(decision.raisedAt, Date.now())}</span>
      </div>
      <div className="island__card-body">{decision.question}</div>
      {freeForm ? (
        <form className="island__answer" onSubmit={submit}>
          <input
            value={answer}
            onChange={(event) => setAnswer(event.target.value)}
            placeholder="Your answer"
            aria-label={`Answer: ${decision.question}`}
            spellCheck={false}
          />
          <button
            type="submit"
            className="btn--primary"
            disabled={!answer.trim() || controller.busy}
          >
            Answer
          </button>
        </form>
      ) : (
        <ul className="island__options">
          {decision.options.map((option, index) => (
            <li key={option.key}>
              <button
                type="button"
                className="island__option"
                disabled={controller.busy}
                onClick={() => void controller.resolve(decision.id, option.key)}
              >
                {chords && index < 9 && <kbd className="island__chord">⌥{index + 1}</kbd>}
                <span>{option.label}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="island__card-actions">
        <button
          type="button"
          className="btn--bare"
          disabled={controller.busy}
          onClick={() => void controller.dismiss(decision.id)}
        >
          Dismiss the question
        </button>
        <span className="island__spacer" />
        {/* Says what happens next, because unblocking is a core rule the user
            did not write and would otherwise have to infer. */}
        <span className="island__meta">
          {workstream.openDecisions > 1
            ? `${workstream.openDecisions - 1} more question${
                workstream.openDecisions === 2 ? '' : 's'
              } after this`
            : 'answering resumes the work'}
        </span>
      </div>
    </div>
  );
}

/**
 * Whether a keystroke belongs to something the user is typing into. Monaco's
 * hidden textarea counts as the editor, not as a field: ⌥A from the editor is
 * exactly what the shortcut is for, so only plain inputs are excluded.
 */
function isTypingInField(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && target.tagName === 'INPUT';
}

function RosterRow({
  workstream,
  controller,
  agents,
}: {
  workstream: Workstream;
  controller: WorkstreamsController;
  /** The same roster the rail reads, so the two cannot disagree (T223 §17). */
  agents: AgentsController;
}) {
  const linked = linkedRowLabel(controller, agents, workstream.id);
  // Per row, and closed by default. The roster is the only surface that lists
  // every workstream — the queue lists what already needs you — so this is
  // where a question gets raised, but a form open on every row would bury the
  // roster it belongs to.
  const [asking, setAsking] = useState(false);
  return (
    <>
      <div className="island__row">
        <span
          className={`island__dot island__dot--${stateTone(workstream.state)}`}
          aria-hidden="true"
        />
        <span className="island__row-name" title={workstream.goal ?? undefined}>
          {workstream.title}
        </span>
        <span className="island__row-state">{stateLabel(workstream.state)}</span>
        {workstream.openDecisions > 0 && (
          <span className="island__row-open">
            {workstream.openDecisions} open{workstream.openDecisions === 1 ? ' question' : ''}
          </span>
        )}
        {ownsLabel(controller.ownership, workstream.id) && (
          <span className="island__row-owns">{ownsLabel(controller.ownership, workstream.id)}</span>
        )}
        {/* What is working on it, live when it is in the roster and "not
          running" when it is not — never dropped, because the link is what
          happened (T223 §12). */}
        {linked && <span className="island__row-agents">{linked}</span>}
        <span className="island__spacer" />
        {/* Exactly the moves the core allows, straight from the wire: the
          renderer keeps no copy of the state machine (T200). */}
        {/* Before the moves, because asking is what *stops* the work: raising a
          question on running work blocks it, which the core does itself. */}
        <button
          type="button"
          className={`island__move${asking ? ' island__move--on' : ''}`}
          aria-expanded={asking}
          disabled={controller.busy}
          onClick={() => setAsking((open) => !open)}
        >
          {asking ? 'Cancel' : 'Ask'}
        </button>
        {workstream.allowedTransitions
          .filter((to) => to !== 'ABANDONED' && to !== 'BLOCKED')
          .map((to) => (
            <button
              key={to}
              type="button"
              className="island__move"
              disabled={controller.busy}
              onClick={() => void controller.transition(workstream.id, to)}
            >
              {transitionLabel(to)}
            </button>
          ))}
      </div>
      {asking && (
        <RaiseQuestion
          workstreamId={workstream.id}
          controller={controller}
          onDone={() => setAsking(false)}
        />
      )}
    </>
  );
}

function NewWorkstream({ controller }: { controller: WorkstreamsController }) {
  const [title, setTitle] = useState('');

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = title.trim();
    if (!trimmed) return;
    void controller.create(trimmed);
    setTitle('');
  }

  return (
    <form className="island__new" onSubmit={submit}>
      <input
        value={title}
        onChange={(event) => setTitle(event.target.value)}
        placeholder="New workstream"
        aria-label="New workstream title"
        maxLength={200}
      />
      <button type="submit" disabled={!title.trim() || controller.busy}>
        Add
      </button>
    </form>
  );
}
