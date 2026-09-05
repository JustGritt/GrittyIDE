import type { AgentSession } from '@gritty/protocol';
import { ErrorEnvelope } from '../../components/error-envelope';
import {
  activityLabel,
  activityTone,
  runningCount,
  sessionsElsewhere,
  sessionsHere,
  taskLine,
  toolLine,
} from './agents-reducer';
import type { AgentsController } from './use-agents';

/**
 * The agent roster inside the island (T220).
 *
 * Deliberately read-only. Every row is something another terminal owns, so
 * there is no button here that acts on one — the pid and the tty are shown
 * precisely so the user can go and find it themselves. When Gritty runs agents
 * of its own (T300) those get their own rows, with controls, and this section
 * keeps meaning "everything else".
 */
export function AgentsSection({ controller }: { controller: AgentsController }) {
  const { roster, loaded } = controller;
  const here = sessionsHere(roster);
  const elsewhere = sessionsElsewhere(roster);

  return (
    <section className="agents" aria-label="Agent sessions on this machine">
      <div className="agents__head">
        <span className="agents__title">Agents</span>
        <span className="island__spacer" />
        <button
          type="button"
          className="btn--bare"
          onClick={() => void controller.refresh()}
          disabled={controller.busy}
        >
          {controller.busy ? 'Scanning…' : 'Rescan'}
        </button>
      </div>

      {controller.error && (
        <div className="island__error">
          <ErrorEnvelope error={controller.error} onDismiss={controller.clearError} />
        </div>
      )}

      {/* The process table was unreadable, so every row would read as ended
          whether or not it is. Say that instead of asserting the machine is
          quiet — the roster cannot tell the difference. */}
      {loaded && !roster.processEvidence && (
        <p className="agents__note">
          Cannot read the process list, so nothing can be shown as running. Sessions below come from
          their transcripts alone.
        </p>
      )}

      {!loaded ? (
        <p className="island__empty">Looking for agent sessions…</p>
      ) : roster.sessions.length === 0 ? (
        <p className="island__empty">
          No agent sessions found. Gritty looks for coding agents running in your own terminals — it
          does not start them, and it changes nothing about how they run.
        </p>
      ) : (
        <>
          {here.length > 0 && (
            <div className="agents__group">
              <p className="agents__group-title">In this workspace</p>
              {here.map((session) => (
                <AgentRow key={session.id} session={session} />
              ))}
            </div>
          )}
          {elsewhere.length > 0 && (
            <div className="agents__group">
              <p className="agents__group-title">Elsewhere on this machine</p>
              {elsewhere.map((session) => (
                <AgentRow key={session.id} session={session} showRoot />
              ))}
            </div>
          )}
        </>
      )}
    </section>
  );
}

/**
 * One session: what it is doing, what it is doing it to, and where to find it.
 *
 * Everything textual here came out of a transcript, which is untrusted input.
 * It arrives already bounded and whitespace-collapsed by the core and is
 * rendered as text, never as markup and never as a path anything opens.
 */
function AgentRow({ session, showRoot = false }: { session: AgentSession; showRoot?: boolean }) {
  const task = taskLine(session);
  const tool = toolLine(session);

  return (
    <div className={`agents__row agents__row--${activityTone(session.activity)}`}>
      <span
        className={`island__dot island__dot--${activityTone(session.activity)}`}
        aria-hidden="true"
      />
      <div className="agents__body">
        <div className="agents__line">
          <span className="agents__task">
            {task ?? <em className="agents__unknown">No transcript yet</em>}
          </span>
          <span className="agents__activity">{activityLabel(session.activity)}</span>
        </div>
        <div className="agents__meta">
          {showRoot && session.rootName && <span title={session.root}>{session.rootName}</span>}
          {session.gitBranch && <span>{session.gitBranch}</span>}
          {session.model && <span>{session.model}</span>}
          {session.tty && <span>{session.tty}</span>}
          {session.pid !== undefined && <span>pid {session.pid}</span>}
        </div>
        {tool && (
          <div className="agents__tool" title={tool}>
            {tool}
          </div>
        )}
      </div>
    </div>
  );
}

/** The count the collapsed island pill shows beside the workstream counts. */
export function agentsPillLabel(controller: AgentsController): string | null {
  const running = runningCount(controller.roster);
  if (!controller.loaded || running === 0) return null;
  return `${running} agent${running === 1 ? '' : 's'}`;
}
