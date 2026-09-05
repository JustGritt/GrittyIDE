import type { Workstream } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import {
  type AttentionFilter,
  type AttentionItem,
  attentionItems,
  filterItems,
  stateLabel,
  stateTone,
  transitionLabel,
  waitedFor,
} from './attention';
import { DecisionCard } from './island';
import type { WorkstreamsController } from './use-workstreams';

/**
 * The attention queue (T210, artboard 4b): everything waiting on you, oldest
 * first, with the selected item in full on the right.
 *
 * The artboard's five filters are three here — All, Questions, Reviews.
 * Approvals, conflicts and failures belong to agent runs and worktrees
 * (T300/T400); a tab that can never have anything in it teaches the user to
 * distrust the counts, so they arrive with the things they count.
 *
 * The island and this view show the same items, so answering in one clears the
 * other: both render the core's decisions rather than a copy.
 */
export function AttentionQueue({
  controller,
  onClose,
}: {
  controller: WorkstreamsController;
  onClose(): void;
}) {
  const { workstreams, decisions } = controller;
  const items = attentionItems(workstreams, decisions);
  const [filter, setFilter] = useState<AttentionFilter>('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const visible = filterItems(items, filter);
  // Follows the list rather than holding a stale id: answering an item removes
  // it, and the next one is what you want to look at.
  const selected = visible.find((item) => item.id === selectedId) ?? visible[0] ?? null;

  useEffect(() => {
    if (selected && selected.id !== selectedId) setSelectedId(selected.id);
  }, [selected, selectedId]);

  const counts = {
    all: items.length,
    question: items.filter((item) => item.kind === 'question').length,
    review: items.filter((item) => item.kind === 'review').length,
  };

  return (
    <section className="queue" aria-label="Attention queue">
      <div className="queue__bar">
        {(['all', 'question', 'review'] as const).map((key) => (
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
            <QueueDetail item={selected} controller={controller} />
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
        <span className="queue__meta">
          Approvals, conflicts and failures arrive with agent runs
        </span>
      </div>
    </section>
  );
}

const FILTER_LABEL: Record<AttentionFilter, string> = {
  all: 'All',
  question: 'Questions',
  review: 'Reviews',
};

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
        <span
          className={`island__dot island__dot--${item.kind === 'question' ? 'blocked' : 'waiting'}`}
          aria-hidden="true"
        />
        <span className="label">{item.kind === 'question' ? 'Question' : 'Review'}</span>
        <span className="queue__row-name">{item.workstream.title}</span>
        <span className="queue__spacer" />
        <span className="queue__meta">waiting {waitedFor(item.since, now)}</span>
      </span>
      <span className="queue__row-body">
        {item.kind === 'question'
          ? item.decision.question
          : 'Finished and handed back for your review.'}
      </span>
    </button>
  );
}

function QueueDetail({
  item,
  controller,
}: {
  item: AttentionItem;
  controller: WorkstreamsController;
}) {
  return (
    <div className="queue__detail-body">
      <div className="queue__detail-head">
        <span className="label">Selected · {item.kind === 'question' ? 'question' : 'review'}</span>
        <span className="queue__spacer" />
        <span
          className={`island__dot island__dot--${stateTone(item.workstream.state)}`}
          aria-hidden="true"
        />
        <span className="queue__meta">{stateLabel(item.workstream.state)}</span>
      </div>

      {item.kind === 'question' ? (
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
      </dl>

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
