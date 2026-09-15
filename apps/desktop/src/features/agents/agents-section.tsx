import type { AgentRunId, CoreError } from '@gritty/protocol';
import { type ReactNode, useEffect, useId, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { AGENTS_SECTION_CHORD_HINT, isAgentsSectionChord } from './agents-section-chords';
import { partitionRows, type RosterRow } from './roster-model';
import type { RunStatus } from './run-status';

/**
 * The Agents section at the top of the left sidebar (T233-S/T233-C, design
 * artboard 12a, collapse behaviour from 11b).
 *
 * Pinned above the file tree rather than made a tab beside it, and that is the
 * whole design: a tab can only be looked at instead of the tree, and an agent
 * that stopped to ask you something must not be invisible because you happened
 * to be reading a file. For the same reason the collapsed form is not empty —
 * it keeps the status marks and the waiting count, so folding the section can
 * never hide an approval.
 *
 * Every row comes from `rosterRows`, which merges the runs Gritty owns with the
 * sessions it merely observes. `row.owned` is what decides whether a row may
 * offer a control: a run Gritty started can be cancelled, and a session another
 * terminal owns cannot be touched at all. That is not a UI preference — it is
 * `docs/architecture/agent-runs.md`'s boundary, drawn where the user can see it.
 *
 * The count is `AgentRoster.needsYou`, taken from the core and never recomputed,
 * so this section and the island cannot report different totals.
 *
 * It also inherits three things that used to live only in the rail (T233-C):
 * the roster's error envelope, the `processEvidence` note, and — on each row's
 * tooltip — where its status came from (T221 §24).
 */
export function SidebarAgents({
  rows,
  needsYou,
  loaded,
  processEvidence,
  error,
  onDismissError,
  open,
  onOpenChange,
  selectedKey,
  onSelect,
  onKeep,
  onCancel,
  onRescan,
  busy,
  chords,
  launcher,
  onLauncherToggle,
}: {
  rows: readonly RosterRow[];
  /** The core's total, for the header. Not derived from `rows`. */
  needsYou: number;
  /** False until the first roster lands, so "none" and "not yet" differ. */
  loaded: boolean;
  /**
   * Whether the process table could be read at all. `false` means every row is
   * transcript-only and so reads as `Ended` whether it is or not.
   */
  processEvidence: boolean;
  /** A refusal from `agents.list` / `agents.refresh`, rendered rather than swallowed. */
  error: CoreError | null;
  onDismissError: () => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  selectedKey: string | null;
  /** A single click: open the console as a preview tab, or show one already open. */
  onSelect: (row: RosterRow) => void;
  /** A double click: keep the tab, so the next single click does not replace it. */
  onKeep: (row: RosterRow) => void;
  /** Only ever called for an owned row; an observed session has no such verb. */
  onCancel: (runId: AgentRunId) => void;
  onRescan: () => void;
  busy: boolean;
  /**
   * Whether ⌥G folds the section. Off while the island is open, which binds its
   * own ⌥ chords; there is still no shortcut registry, so each listener is
   * gated by hand.
   */
  chords: boolean;
  /**
   * The New run form, when it is open (RUN-LAUNCHER). An element rather than
   * props for it, the way the explorer takes its Changes panel: this section
   * draws rows and has no business holding the controllers a launch needs.
   */
  launcher?: ReactNode;
  /** Open or fold the launcher. Absent, the bar offers no New run. */
  onLauncherToggle?: () => void;
}) {
  const bodyId = useId();
  const [showHidden, setShowHidden] = useState(false);

  // Held in a ref so the listener binds once per gate change rather than once
  // per render, which is how the dock binds ⌥1…⌥9.
  const toggle = useRef<() => void>(() => {});
  toggle.current = () => onOpenChange(!open);
  useEffect(() => {
    if (!chords) return;
    function onKeyDown(event: KeyboardEvent) {
      if (!isAgentsSectionChord(event)) return;
      // A field is a field: ⌥G halfway through typing a name must not fold the
      // window's layout instead.
      if (event.target instanceof HTMLElement && event.target.tagName === 'INPUT') return;
      // Nor halfway through a prompt in this section's own launcher, where
      // folding would unmount the form and take the draft with it. Scoped to
      // the section's fields, so ⌥G from Monaco's hidden textarea, or from a
      // row button that kept focus after a click, still folds.
      if (
        event.target instanceof HTMLElement &&
        (event.target.tagName === 'TEXTAREA' || event.target.tagName === 'SELECT') &&
        event.target.closest('.agents-section')
      ) {
        return;
      }
      // Claimed even when it changes nothing, so the character macOS composed
      // (⌥G is `©`) never reaches whatever has focus.
      event.preventDefault();
      toggle.current();
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [chords]);

  const { shown, hidden } = partitionRows(rows);
  const listed = showHidden ? [...shown, ...hidden] : shown;

  return (
    <section className={`agents-section${open ? '' : ' agents-section--collapsed'}`}>
      <div className="agents-section__bar">
        <button
          type="button"
          className="agents-section__label"
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          onClick={() => onOpenChange(!open)}
          title={`Agents (${AGENTS_SECTION_CHORD_HINT})`}
        >
          <span className="agents-section__caret" aria-hidden="true">
            {open ? '▾' : '▸'}
          </span>
          <span>Agents</span>
          {/* The live count, so it means what the top bar's "N agents" means.
              Counting the dead here as well is what made the two disagree. */}
          {shown.length > 0 && <span className="agents-section__count">{shown.length}</span>}
        </button>

        {/* Collapsed, the section still says what it would have shown: one mark
            per agent in its own tone, and the waiting count beside them. The
            two things you would otherwise have to unfold it to learn (11b). */}
        {!open && shown.length > 0 && (
          <span className="agents-section__dots" aria-hidden="true">
            {shown.slice(0, MAX_DOTS).map((row) => (
              // The nested mark is not decoration: the shape rules are written
              // as `.agent-status--tone .agent-status__mark`, so a bare span
              // would come out the same circle in every tone.
              <span
                key={row.key}
                className={`agent-status agent-status--${row.status.tone} agents-section__dot`}
              >
                <span className="agent-status__mark" />
              </span>
            ))}
          </span>
        )}

        <span className="agents-section__spacer" />

        {needsYou > 0 && (
          <span className="agents-section__needs">
            {needsYou}
            <span className="agents-section__needs-word"> waiting</span>
          </span>
        )}

        {open && onLauncherToggle && (
          <button
            type="button"
            className="btn--bare"
            aria-expanded={Boolean(launcher)}
            onClick={onLauncherToggle}
          >
            New run
          </button>
        )}

        {open && (
          <button type="button" className="btn--bare" onClick={onRescan} disabled={busy}>
            {busy ? 'Scanning…' : 'Rescan'}
          </button>
        )}
      </div>

      {open && (
        <div className="agents-section__body" id={bodyId}>
          {launcher}

          {/* A refusal has to land somewhere. It used to land in the rail, and
              deleting the rail without this would have made a failed
              `agents.list` silent. */}
          {error && (
            <div className="agents-section__error">
              <ErrorEnvelope error={error} onDismiss={onDismissError} />
            </div>
          )}

          {/* The process table was unreadable, so every row would read as Ended
              whether it is or not. Say that, rather than implying the machine
              is quiet — the roster cannot tell the difference. */}
          {loaded && !processEvidence && (
            <p className="agents-section__note">
              Cannot read the process list, so nothing can be shown as running. The agents below
              come from their transcripts alone.
            </p>
          )}

          {!loaded ? (
            <p className="agents-section__empty">Looking for agents…</p>
          ) : listed.length === 0 ? (
            // Nothing to list. When the reveal below has something, it is the
            // whole content — an empty `<ul>` above it would be a list element
            // standing for no list.
            // Not while the launcher is open: the form is already the answer to
            // "no agents here", and the sentence under it would repeat the offer.
            hidden.length === 0 &&
            !launcher && (
              <p className="agents-section__empty">
                No agents here. Gritty lists the coding agents running in this workspace — the ones
                in your own terminals, and the runs it started itself.
                {onLauncherToggle && ' Start one with New run.'}
              </p>
            )
          ) : (
            <ul className="agents-section__list">
              {listed.map((row) => (
                <AgentRow
                  key={row.key}
                  row={row}
                  selected={row.key === selectedKey}
                  onSelect={onSelect}
                  onKeep={onKeep}
                  onCancel={onCancel}
                />
              ))}
            </ul>
          )}

          {/* One reveal for both kinds of quiet row: the dead, and the ones
              running somewhere else. They are the rest of the machine, and the
              rail was the only place they appeared — hiding them behind a
              default is fine, losing them is not. */}
          {hidden.length > 0 && (
            <button
              type="button"
              className="agents-section__more btn--bare"
              aria-expanded={showHidden}
              onClick={() => setShowHidden((current) => !current)}
            >
              {showHidden ? `Hide ${hiddenLabel(hidden)}` : `Show ${hiddenLabel(hidden)}`}
            </button>
          )}
        </div>
      )}
    </section>
  );
}

/**
 * Enough dots to read as a cluster, not so many that the strip wraps. Beyond
 * this the count beside them is the honest summary anyway.
 */
const MAX_DOTS = 6;

/** `10 ended`, `2 elsewhere`, or `10 ended and 2 elsewhere` — never a bare number. */
function hiddenLabel(hidden: readonly RosterRow[]): string {
  const ended = hidden.filter((row) => row.retired).length;
  const elsewhere = hidden.length - ended;
  if (ended === 0) return `${elsewhere} elsewhere`;
  if (elsewhere === 0) return `${ended} ended`;
  return `${ended} ended and ${elsewhere} elsewhere`;
}

function AgentRow({
  row,
  selected,
  onSelect,
  onKeep,
  onCancel,
}: {
  row: RosterRow;
  selected: boolean;
  onSelect: (row: RosterRow) => void;
  onKeep: (row: RosterRow) => void;
  onCancel: (runId: AgentRunId) => void;
}) {
  // Where the status came from, on the row that carries it (T221 §24). Three
  // sources can disagree, and `BLOCKED` — the one status the user is meant to
  // act on — is otherwise a verdict with no provenance. The console states it
  // in words; here it rides the tooltip, because the row has no line to spare.
  const hint =
    row.sourceLabel === null ? row.status.hint : `${row.status.hint} (${row.sourceLabel})`;

  // Identity, then status: the bar says *which* agent and the mark says *how
  // it is doing*, which is the two-channel encoding the terminal tabs and the
  // sidebar rows share. Built once because the row draws it either inside a
  // button or inside a plain span, depending on whether it has anywhere to go.
  const face = (
    <>
      <span className="agents-section__identity" aria-hidden="true" />
      <StatusMark status={row.status} />
      <span className="agents-section__text">
        <span className="agents-section__head">
          <span className="agents-section__id">{row.id}</span>
          {row.title !== null && <span className="agents-section__title">{row.title}</span>}
        </span>
        {/* The status word travels with the row even though a mark is drawn
            beside it: colour is never the only channel (T231 §10). */}
        <span className="agents-section__detail">
          {row.status.label}
          {row.detail !== null && ` · ${row.detail}`}
        </span>
      </span>
    </>
  );

  return (
    <li
      className={`agents-section__row${selected ? ' agents-section__row--selected' : ''}${
        row.owned ? ' agents-section__row--owned' : ''
      }${row.elsewhere ? ' agents-section__row--elsewhere' : ''}`}
    >
      {/* Every row opens a console now (T310-U). Both lanes have a feed to
          draw — an observed session a timeline the core follows, a run Gritty
          owns the event ring it recorded — and `row.key` is the console key for
          whichever it is, so this row model is the only thing either side
          needs. A run's console additionally has a prompt box; that difference
          lives in the console, not in the row. */}
      <button
        type="button"
        className="agents-section__pick"
        aria-current={selected ? 'true' : undefined}
        onClick={() => onSelect(row)}
        onDoubleClick={() => onKeep(row)}
        title={hint}
      >
        {face}
      </button>

      {/* The roster is machine-wide, so a row from somewhere else says so in
          words as well as by the edge the class draws down its left. */}
      {row.elsewhere && <span className="agents-section__away">elsewhere</span>}

      {row.badge && (
        <span className={`agents-section__badge agents-section__badge--${row.badge.tone}`}>
          {row.badge.text}
        </span>
      )}

      {/* Sibling of the pick button, never inside it: a button within a button
          is not markup a browser will honour. Owned rows only — Gritty cannot
          stop a session it did not start, and offering to would be a lie about
          the boundary the row model exists to keep. */}
      {row.owned && row.runId !== null && (
        <button
          type="button"
          className="agents-section__cancel btn--bare"
          onClick={() => onCancel(row.runId as AgentRunId)}
          title={`Cancel ${row.id}`}
        >
          Cancel
        </button>
      )}
    </li>
  );
}

/**
 * The status mark alone — the word for it is on the row's second line.
 *
 * `agent-status-badge.tsx` is the same markup entered through an
 * `AgentActivity`; this one is entered through a `RunStatus`, because a row
 * here may be either a run or a session and the two have different state
 * vocabularies. The classes are deliberately identical, so both keep drawing
 * from the one set of ADR-016 tokens and marker shapes — the shape rules are
 * written as `.agent-status--tone .agent-status__mark`, which is why the mark
 * stays a nested element rather than becoming one span.
 */
function StatusMark({ status }: { status: RunStatus }) {
  return (
    <span
      className={`agent-status agent-status--${status.tone} agents-section__status`}
      aria-hidden="true"
    >
      <span className="agent-status__mark" />
    </span>
  );
}
