import type { AgentSession } from '@gritty/protocol';
import { useEffect, useId, useRef } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { AGENT_RAIL_CHORD_HINT, isAgentRailChord } from './agent-rail-chords';
import { agentStatus, providerLabel } from './agent-status';
import { AgentStatusBadge } from './agent-status-badge';
import { taskLine, toolLine } from './agents-reducer';
import type { AgentRailController } from './use-agent-rail';
import type { AgentsController } from './use-agents';

/**
 * The agent rail (T231): the third region of the shell, to the right of the
 * editor, listing the coding agents running on this machine.
 *
 * Beside your code rather than instead of it, and deliberately not a tab — an
 * agent session and an open file are different things, and one strip holding
 * both makes both harder to read.
 *
 * Read-only, like the island section it takes over from: every row is a
 * session another terminal owns, so nothing here starts, stops or answers one.
 * The chrome is one row and the body is what collapses, following the dock: a
 * collapsed rail still reports how many sessions need you, because a surface
 * that goes quiet when it is folded away is worse than one that takes space.
 */
export function AgentRail({
  agents,
  rail,
  chords,
}: {
  /** The one roster the island reads too — never a second copy (§7). */
  agents: AgentsController;
  rail: AgentRailController;
  /**
   * Whether ⌥G toggles the rail. Off while the island is open, which binds its
   * own ⌥ chords; there is no central shortcut registry, so each listener is
   * gated by hand.
   */
  chords: boolean;
}) {
  const { roster, loaded } = agents;
  const bodyId = useId();

  // Held in a ref so the listener is bound once per gate change rather than
  // once per render, which is how the dock binds ⌥1…⌥9.
  const toggle = useRef<() => void>(() => {});
  toggle.current = rail.toggle;
  useEffect(() => {
    if (!chords) return;
    function onKeyDown(event: KeyboardEvent) {
      if (!isAgentRailChord(event)) return;
      // A field is a field: ⌥G halfway through typing a name must not fold the
      // window's layout instead.
      if (event.target instanceof HTMLElement && event.target.tagName === 'INPUT') return;
      // Claimed even when it changes nothing, so the character macOS composed
      // (⌥G is `©`) never reaches whatever has focus.
      event.preventDefault();
      toggle.current();
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [chords]);

  const needsYou = roster.needsYou;
  const selected = roster.sessions.find((session) => session.id === rail.selectedId) ?? null;

  return (
    <aside
      className={`agent-rail${rail.open ? '' : ' agent-rail--collapsed'}`}
      aria-label="Agent sessions"
    >
      <header className="agent-rail__bar">
        <button
          type="button"
          className="agent-rail__label"
          aria-expanded={rail.open}
          // Only while there is a body to control: the rail unmounts its own,
          // unlike the dock, because nothing in it holds state worth keeping.
          aria-controls={rail.open ? bodyId : undefined}
          onClick={rail.toggle}
          title={`Agent sessions (${AGENT_RAIL_CHORD_HINT})`}
        >
          <span>Agents</span>
          {/* Visible collapsed as well as expanded: the count is the reason a
              folded rail is still worth the strip it occupies. */}
          {needsYou > 0 && (
            <span className="agent-rail__needs">
              {needsYou}
              <span className="agent-rail__needs-word"> needs you</span>
            </span>
          )}
        </button>
        {rail.open && (
          <>
            <span className="agent-rail__spacer" />
            <button
              type="button"
              className="btn--bare"
              onClick={() => void agents.refresh()}
              disabled={agents.busy}
            >
              {agents.busy ? 'Scanning…' : 'Rescan'}
            </button>
            <kbd className="agent-rail__chord">{AGENT_RAIL_CHORD_HINT}</kbd>
          </>
        )}
      </header>

      {rail.open && (
        <div className="agent-rail__body" id={bodyId}>
          {agents.error && (
            <div className="agent-rail__error">
              <ErrorEnvelope error={agents.error} onDismiss={agents.clearError} />
            </div>
          )}

          {/* The process table was unreadable, so every session would read as
              Ended whether it is or not. Say that, rather than implying the
              machine is quiet — the roster cannot tell the difference. */}
          {loaded && !roster.processEvidence && (
            <p className="agent-rail__note">
              Cannot read the process list, so nothing can be shown as running. The sessions below
              come from their transcripts alone.
            </p>
          )}

          {!loaded ? (
            <p className="agent-rail__empty">Looking for agent sessions…</p>
          ) : roster.sessions.length === 0 ? (
            <p className="agent-rail__empty">
              No agent sessions found. Gritty looks for coding agents running in your own terminals
              — it does not start them, and it changes nothing about how they run.
            </p>
          ) : (
            <ul className="agent-rail__list">
              {/* The core's ranking, unchanged: workspace first, then blocked
                  on you, then running, then recency. Re-sorting here would put
                  two orders in the product and hide a core bug (§5). */}
              {roster.sessions.map((session) => (
                <li key={session.id}>
                  <AgentRow
                    session={session}
                    selected={session.id === rail.selectedId}
                    pending={session.id === rail.pendingId}
                    onSelect={() => rail.select(session.id)}
                  />
                </li>
              ))}
            </ul>
          )}

          {rail.error && (
            <div className="agent-rail__error">
              <ErrorEnvelope error={rail.error} onDismiss={rail.clearError} />
            </div>
          )}

          {selected && <TimelinePlaceholder session={selected} rail={rail} />}
        </div>
      )}
    </aside>
  );
}

/**
 * One session: what it is, what it is doing, and where.
 *
 * Everything textual comes out of a transcript, which is untrusted input. It
 * arrives bounded and whitespace-collapsed from the core and is rendered as
 * text — never as markup, never as a path anything opens.
 */
function AgentRow({
  session,
  selected,
  pending,
  onSelect,
}: {
  session: AgentSession;
  selected: boolean;
  pending: boolean;
  onSelect: () => void;
}) {
  const status = agentStatus(session.activity);
  const task = taskLine(session);
  const tool = toolLine(session);
  const away = !session.inWorkspace;

  return (
    <button
      type="button"
      className={`agent-rail__row${selected ? ' agent-rail__row--selected' : ''}${
        away ? ' agent-rail__row--away' : ''
      }`}
      // Not a tab and not an option in a listbox: it is a toggle that opens
      // one session's timeline, and `aria-pressed` is what says so.
      aria-pressed={selected}
      onClick={onSelect}
      title={status.hint}
    >
      <span className="agent-rail__task">
        {task ?? <em className="agent-rail__unknown">No transcript yet</em>}
      </span>
      <span className="agent-rail__meta">
        <span>{providerLabel(session.provider)}</span>
        {session.root ? (
          <span className="agent-rail__root">{session.root}</span>
        ) : (
          <em className="agent-rail__unknown">directory unknown</em>
        )}
        {/* The roster is machine-wide, so a row from somewhere else says so in
            words as well as by the edge the class draws down its left. */}
        {away && <span className="agent-rail__away">elsewhere</span>}
      </span>
      {tool && <span className="agent-rail__tool">{tool}</span>}
      <span className="agent-rail__foot">
        <AgentStatusBadge activity={session.activity} />
        {pending && <span className="agent-rail__pending">Opening…</span>}
      </span>
    </button>
  );
}

/**
 * Where the timeline goes (T232).
 *
 * The follower is running — `agents.openTimeline` returned, and the core is
 * publishing `AgentTimelineAppended` for this session — but rendering the feed
 * and the files it touched is the next task. Saying so is better than an empty
 * frame that looks like a session with nothing in it.
 */
function TimelinePlaceholder({
  session,
  rail,
}: {
  session: AgentSession;
  rail: AgentRailController;
}) {
  const title = taskLine(session);
  return (
    <section className="agent-rail__timeline" aria-label="Timeline">
      <div className="agent-rail__timeline-head">
        <span className="agent-rail__timeline-title">{title ?? 'Selected session'}</span>
        <span className="agent-rail__spacer" />
        <AgentStatusBadge activity={session.activity} />
      </div>
      {rail.timeline && !rail.timeline.transcriptEvidence ? (
        <p className="agent-rail__note">
          No transcript could be read for this session, so there is nothing to follow. Gritty knows
          it is running, not what it is doing.
        </p>
      ) : (
        <p className="agent-rail__placeholder">
          Following this session. Its timeline is not drawn yet.
        </p>
      )}
    </section>
  );
}
