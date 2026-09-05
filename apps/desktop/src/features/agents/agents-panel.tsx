import { AgentIntegrationSettings } from './agent-integration-settings';
import { AGENT_RAIL_CHORD_HINT } from './agent-rail-chords';
import { AgentStatusBadge } from './agent-status-badge';
import { mostUrgent, runningCount, taskLine } from './agents-reducer';
import type { AgentsController } from './use-agents';

/**
 * The agent roster inside the island — reduced to a count and one row (T231 §7).
 *
 * The rail is the primary surface now. The island keeps the summary because it
 * is always on screen, and keeps the single row worth interrupting for so the
 * user does not have to open the rail to learn there is nothing to do. It does
 * *not* keep a roster of its own: it reads the same `AgentsController` the rail
 * does, so the two cannot report different counts.
 *
 * Still deliberately read-only. Every session here is one another terminal
 * owns; nothing in Gritty starts, stops or answers one.
 */
export function AgentsSection({ controller }: { controller: AgentsController }) {
  const { roster, loaded } = controller;
  const urgent = mostUrgent(roster);
  const running = runningCount(roster);

  return (
    <section className="agents" aria-label="Agent sessions on this machine">
      <div className="agents__head">
        <span className="agents__title">Agents</span>
        <span className="island__spacer" />
        {/* Where the rest of them are. No control, just the way in — the rail
            is a read, and so is this. */}
        <span className="agents__hint">
          The rail lists them all <kbd className="island__chord">{AGENT_RAIL_CHORD_HINT}</kbd>
        </span>
        {/* The hook integration (T221) lives here, in settings, and not as a
            control on a session row: it changes a file the user owns, and that
            is a decision about the machine rather than about any one session
            (ADR-018 §1). It reads nothing until it is opened. */}
        <AgentIntegrationSettings />
      </div>

      {/* The envelope itself belongs to the rail, which is where the rescan
          is; saying nothing here would leave the island claiming to still be
          looking when the scan already failed. */}
      {controller.error && !loaded ? (
        <p className="island__empty">The agent roster could not be read. The rail says why.</p>
      ) : !loaded ? (
        <p className="island__empty">Looking for agent sessions…</p>
      ) : roster.sessions.length === 0 ? (
        <p className="island__empty">
          No agent sessions found. Gritty looks for coding agents running in your own terminals.
        </p>
      ) : (
        <>
          <p className="agents__summary">
            {roster.processEvidence ? (
              `${running} running of ${roster.sessions.length}`
            ) : (
              // Without the process table every session reads as ended, so a
              // "running" count here would be a claim the roster cannot make.
              <span className="agents__note">
                {roster.sessions.length} found; the process list is unreadable
              </span>
            )}
            {roster.needsYou > 0 && (
              <span className="island__needs"> · {roster.needsYou} needs you</span>
            )}
          </p>
          {urgent && (
            <div className="agents__row">
              <div className="agents__body">
                <div className="agents__line">
                  <span className="agents__task">
                    {taskLine(urgent) ?? <em className="agents__unknown">No transcript yet</em>}
                  </span>
                </div>
                <div className="agents__meta">
                  <AgentStatusBadge activity={urgent.activity} />
                  {urgent.root && <span>{urgent.root}</span>}
                </div>
              </div>
            </div>
          )}
        </>
      )}
    </section>
  );
}

/** The count the collapsed island pill shows beside the workstream counts. */
export function agentsPillLabel(controller: AgentsController): string | null {
  const running = runningCount(controller.roster);
  if (!controller.loaded || running === 0) return null;
  return `${running} agent${running === 1 ? '' : 's'}`;
}
