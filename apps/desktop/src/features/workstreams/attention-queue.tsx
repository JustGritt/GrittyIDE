import type { AgentPermissionVerdict, Workstream } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { taskLine } from '../agents/agents-reducer';
import { verbLabel } from '../agents/permission-model';
import { PermissionPrompt } from '../agents/permission-prompt';
import type { AgentsController } from '../agents/use-agents';
import { LinkedSessions } from './agent-link-panel';
import {
  type AttentionFilter,
  type AttentionItem,
  attentionItems,
  filterItems,
  isPermission,
  stateLabel,
  stateTone,
  transitionLabel,
  waitedFor,
} from './attention';
import { DecisionCard } from './island';
import { targetLabel } from './ownership';
import { RaiseQuestion } from './raise-question';
import type { WorkstreamsController } from './use-workstreams';

/**
 * The attention queue (T210, artboard 4b): everything waiting on you, oldest
 * first, with the selected item in full on the right.
 *
 * The artboard's five filters are five here — All, Permissions, Questions,
 * Reviews, Conflicts. Conflicts arrived with ownership (T325) and permissions
 * with T335; failures still belong to agent runs (T300) and are absent, because
 * a tab that can never have anything in it teaches the user to distrust the
 * counts.
 *
 * Permissions are why artboard 12b exists: an agent nobody is watching asks,
 * and the queue answers it *in place* — the verb, the target and the blast
 * radius are on the card, so judging it does not mean opening the console
 * first, though Open console is there when it does.
 *
 * The island and this view show the same items, so answering in one clears the
 * other: both render the core's decisions rather than a copy.
 */
export function AttentionQueue({
  controller,
  agents,
  onClose,
}: {
  controller: WorkstreamsController;
  /**
   * The roster, so an item can say which agent sessions are working on it
   * (T223 §17). Read-only here: the queue links nothing, the island does.
   */
  agents: AgentsController;
  onClose(): void;
}) {
  const { workstreams, decisions, ownership } = controller;
  const items = attentionItems(workstreams, decisions, ownership, agents.roster.sessions);
  const [filter, setFilter] = useState<AttentionFilter>('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const visible = filterItems(items, filter);
  // Follows the list rather than holding a stale id: answering an item removes
  // it, and the next one is what you want to look at.
  const selected = visible.find((item) => item.id === selectedId) ?? visible[0] ?? null;

  useEffect(() => {
    if (selected && selected.id !== selectedId) setSelectedId(selected.id);
  }, [selected, selectedId]);

  const counts: Record<AttentionFilter, number> = {
    all: items.length,
    permission: items.filter((item) => item.kind === 'permission').length,
    question: items.filter((item) => item.kind === 'question').length,
    review: items.filter((item) => item.kind === 'review').length,
    conflict: items.filter((item) => item.kind === 'conflict').length,
  };
  const answered = agents.roster.recentAnswers?.[0] ?? null;

  return (
    <section className="queue" aria-label="Attention queue">
      <div className="queue__bar">
        {(['all', 'permission', 'question', 'review', 'conflict'] as const).map((key) => (
          <button
            key={key}
            type="button"
            className={`queue__filter${filter === key ? ' queue__filter--on' : ''}`}
            aria-pressed={filter === key}
            onClick={() => setFilter(key)}
          >
            {FILTER_LABEL[key]}
            <span className="queue__filter-count">{counts[key]}</span>
          </button>
        ))}
        <span className="queue__spacer" />
        <span className="queue__hint">oldest first</span>
        <button type="button" className="btn--bare" onClick={onClose}>
          Back to the editor
        </button>
      </div>

      {controller.error && (
        <div className="queue__error">
          <ErrorEnvelope error={controller.error} onDismiss={controller.clearError} />
        </div>
      )}

      <div className="queue__body">
        <div className="queue__list">
          {visible.length === 0 ? (
            <p className="queue__empty">
              {items.length === 0
                ? 'Nothing is waiting on you.'
                : `Nothing under ${FILTER_LABEL[filter].toLowerCase()}.`}
            </p>
          ) : (
            visible.map((item) => (
              <QueueRow
                key={item.id}
                item={item}
                selected={item.id === selected?.id}
                onSelect={() => setSelectedId(item.id)}
              />
            ))
          )}
        </div>

        <aside className="queue__detail">
          {selected ? (
            <QueueDetail item={selected} controller={controller} agents={agents} />
          ) : (
            <div className="queue__detail-empty">
              <p>Everything that needs you shows up here.</p>
              <p className="queue__meta">
                Questions block the work that raised them; answering the last one lets it resume.
              </p>
            </div>
          )}
        </aside>
      </div>

      <div className="queue__foot">
        <span className="queue__meta">
          {countLabel(items.length)} · {blockedLabel(workstreams)}
        </span>
        <span className="queue__spacer" />
        {/* What was just decided, and the only part of it that can still be
            taken back. Undoing is a grant revocation, never a rollback: by the
            time a verdict is read the tool has run, so the label says what the
            button does (T335). */}
        {answered ? (
          <span className="queue__answered">
            <span className="queue__meta">Answered just now:</span>
            <span className="queue__answered-what">
              {verbLabel(answered.verb)} {answered.target}
            </span>
            <span
              className={`queue__answered-verdict queue__answered-verdict--${
                answered.verdict === 'DENY' ? 'deny' : 'allow'
              }`}
            >
              {ANSWER_LABEL[answered.verdict]}
            </span>
            {answered.grantId && (
              <button
                type="button"
                className="btn--bare"
                onClick={() => {
                  if (answered.grantId) void agents.revokeGrant(answered.grantId);
                }}
              >
                Undo the standing rule
              </button>
            )}
          </span>
        ) : (
          <span className="queue__meta">Failures arrive with agent runs</span>
        )}
      </div>
    </section>
  );
}

const FILTER_LABEL: Record<AttentionFilter, string> = {
  all: 'All',
  permission: 'Permissions',
  question: 'Questions',
  review: 'Reviews',
  conflict: 'Conflicts',
};

/** What each kind is called in a row, and which dot it wears. */
const KIND_LABEL: Record<AttentionItem['kind'], string> = {
  permission: 'Permission',
  question: 'Question',
  review: 'Review',
  conflict: 'Ownership',
};

const KIND_TONE: Record<AttentionItem['kind'], string> = {
  permission: 'blocked',
  question: 'blocked',
  review: 'waiting',
  conflict: 'blocked',
};

const ANSWER_LABEL: Record<AgentPermissionVerdict, string> = {
  ALLOW_ONCE: 'allowed once',
  ALLOW_FOR_RUN: 'allowed for this run',
  DENY: 'denied',
};

/** How a row names what is waiting — a workstream, or the session that asked. */
function itemName(item: AttentionItem): string {
  return isPermission(item) ? (taskLine(item.session) ?? item.session.id) : item.workstream.title;
}

function QueueRow({
  item,
  selected,
  onSelect,
}: {
  item: AttentionItem;
  selected: boolean;
  onSelect(): void;
}) {
  const now = Date.now();
  return (
    <button
      type="button"
      className={`queue__row queue__row--${item.kind}${selected ? ' queue__row--on' : ''}`}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="queue__row-head">
        <span className={`island__dot island__dot--${KIND_TONE[item.kind]}`} aria-hidden="true" />
        <span className="label">{KIND_LABEL[item.kind]}</span>
        <span className="queue__row-name">{itemName(item)}</span>
        <span className="queue__spacer" />
        <span className="queue__meta">waiting {waitedFor(item.since, now)}</span>
      </span>
      <span className="queue__row-body">{rowBody(item)}</span>
    </button>
  );
}

function rowBody(item: AttentionItem): string {
  switch (item.kind) {
    case 'permission':
      // Verb, target and blast radius on the row itself: 12b's whole point is
      // that you can judge an ask without opening the console first.
      return [verbLabel(item.request.verb), item.request.target, item.request.detail]
        .filter(Boolean)
        .join(' · ');
    case 'question':
      return item.decision.question;
    case 'review':
      return 'Finished and handed back for your review.';
    case 'conflict':
      return `Wants ${targetLabel(item.conflict.target)}, owned by ${item.owner.title}.`;
  }
}

function QueueDetail({
  item,
  controller,
  agents,
}: {
  item: AttentionItem;
  controller: WorkstreamsController;
  agents: AgentsController;
}) {
  // A permission ask is the one item with no workstream behind it (T335): it
  // belongs to a session Gritty observes, and the link to a workstream is
  // opt-in and usually absent. Everything below this point assumes one.
  if (isPermission(item)) {
    return (
      <div className="queue__detail-body">
        <div className="queue__detail-head">
          <span className="label">Selected · permission</span>
          <span className="queue__spacer" />
          <span className="island__dot island__dot--blocked" aria-hidden="true" />
          <span className="queue__meta">{item.session.root ?? 'this machine'}</span>
        </div>
        <PermissionPrompt
          request={item.request}
          sessionLabel={taskLine(item.session) ?? item.session.id}
          now={Date.now()}
          busy={agents.answering === item.request.askId}
          onAnswer={(verdict) => {
            void agents.answerPermission(item.request.askId, verdict);
          }}
        />
        <p className="queue__meta">
          The same prompt is live in the agent's own terminal. Answering there works too — whichever
          comes first wins.
        </p>
      </div>
    );
  }

  return (
    <div className="queue__detail-body">
      <div className="queue__detail-head">
        <span className="label">Selected · {KIND_LABEL[item.kind].toLowerCase()}</span>
        <span className="queue__spacer" />
        <span
          className={`island__dot island__dot--${stateTone(item.workstream.state)}`}
          aria-hidden="true"
        />
        <span className="queue__meta">{stateLabel(item.workstream.state)}</span>
      </div>

      {item.kind === 'conflict' ? (
        <ConflictCard item={item} controller={controller} />
      ) : item.kind === 'question' ? (
        <DecisionCard
          decision={item.decision}
          workstream={item.workstream}
          controller={controller}
          chords={false}
        />
      ) : (
        <div className="island__card island__card--review">
          <div className="island__card-body">{item.workstream.title}</div>
          {item.workstream.goal && <p className="queue__goal">{item.workstream.goal}</p>}
          <div className="island__card-actions">
            {item.workstream.allowedTransitions
              .filter((to) => to === 'DONE' || to === 'IN_PROGRESS')
              .map((to) => (
                <button
                  key={to}
                  type="button"
                  className={to === 'DONE' ? 'btn--primary' : undefined}
                  disabled={controller.busy}
                  onClick={() => void controller.transition(item.workstream.id, to)}
                >
                  {to === 'DONE' ? 'Accept' : 'Send back'}
                </button>
              ))}
          </div>
        </div>
      )}

      <dl className="queue__facts">
        <dt>Workstream</dt>
        <dd>{item.workstream.title}</dd>
        {item.workstream.goal && (
          <>
            <dt>Goal</dt>
            <dd>{item.workstream.goal}</dd>
          </>
        )}
        <dt>State</dt>
        <dd>{stateLabel(item.workstream.state)}</dd>
        <dt>Open questions</dt>
        <dd>{item.workstream.openDecisions}</dd>
        {item.kind === 'question' && (
          <>
            <dt>Raised by</dt>
            <dd>{describeActor(item.decision.raisedBy.type)}</dd>
          </>
        )}
        {item.kind === 'conflict' && (
          <>
            <dt>Wants</dt>
            <dd>{targetLabel(item.conflict.target)}</dd>
            <dt>Owned by</dt>
            <dd>{item.owner.title}</dd>
          </>
        )}
      </dl>

      {/* What is working on it (T223 §17). Below the facts, because a link is
          a statement about the work rather than one of its own fields — and a
          session that has left the roster still shows, as "not running". */}
      <div className="queue__detail-agents">
        <span className="label">Agent sessions</span>
        <LinkedSessions controller={controller} agents={agents} workstreamId={item.workstream.id} />
        {controller.agentLinks.every((link) => link.workstreamId !== item.workstream.id) && (
          <p className="queue__meta">
            Nothing linked. The island is where a session is attached to a piece of work.
          </p>
        )}
      </div>

      <div className="queue__detail-moves">
        <span className="label">Move it</span>
        <div className="queue__moves">
          {item.workstream.allowedTransitions.map((to) => (
            <button
              key={to}
              type="button"
              disabled={controller.busy}
              onClick={() => void controller.transition(item.workstream.id, to)}
            >
              {transitionLabel(to)}
            </button>
          ))}
        </div>
      </div>

      {/* The same form the island's roster draws (T210). Answering one question
          often raises the next, and this is where the reader already is. */}
      <div className="queue__detail-raise">
        <RaiseQuestion workstreamId={item.workstream.id} controller={controller} />
      </div>
    </div>
  );
}

/**
 * A refusal, and the two ways out of it (T325).
 *
 * There is no "grant it to me" button. Releasing the holder's claim is a
 * separate act from taking the ground, so that an override is never a silent
 * transfer — the requester asks again, and that second ask goes through the
 * same check as the first.
 */
function ConflictCard({
  item,
  controller,
}: {
  item: Extract<AttentionItem, { kind: 'conflict' }>;
  controller: WorkstreamsController;
}) {
  return (
    <div className="island__card island__card--conflict">
      <div className="island__card-body">
        {item.workstream.title} wants to modify {targetLabel(item.conflict.target)}.
      </div>
      <dl className="queue__facts">
        <dt>Currently owned by</dt>
        <dd>{item.owner.title}</dd>
        <dt>State</dt>
        <dd>{stateLabel(item.owner.state)}</dd>
      </dl>
      <div className="island__card-actions">
        <button
          type="button"
          disabled={controller.busy}
          onClick={() => void controller.resolveConflict(item.conflict.id, 'CANCEL')}
        >
          Withdraw the request
        </button>
        <span className="queue__spacer" />
        <button
          type="button"
          className="btn--bare"
          disabled={controller.busy}
          onClick={() => void controller.resolveConflict(item.conflict.id, 'OVERRIDE')}
        >
          Release {item.owner.title}'s claim
        </button>
      </div>
      <p className="queue__meta">
        Releasing it does not hand the ground over: {item.workstream.title} has to claim it again.
      </p>
    </div>
  );
}

/** `AGENT` cannot happen yet, but the wire already allows it (T300). */
function describeActor(type: string): string {
  switch (type) {
    case 'USER':
      return 'You';
    case 'AGENT':
      return 'An agent run';
    case 'SYSTEM':
      return 'Gritty';
    default:
      return 'Something outside Gritty';
  }
}

function countLabel(count: number): string {
  if (count === 0) return 'nothing needs you';
  return `${count} item${count === 1 ? '' : 's'} need${count === 1 ? 's' : ''} you`;
}

function blockedLabel(workstreams: readonly Workstream[]): string {
  const blocked = workstreams.filter((workstream) => workstream.state === 'BLOCKED').length;
  return blocked === 0 ? 'nothing is blocked' : `${blocked} blocked`;
}
