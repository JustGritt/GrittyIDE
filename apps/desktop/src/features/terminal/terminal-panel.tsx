import { ErrorEnvelope } from '../../components/error-envelope';
import { describeTerminal } from './terminal-reducer';
import { TerminalView } from './terminal-view';
import type { TerminalsController } from './use-terminals';

/**
 * The terminal dock: a tab per session and the active session's xterm.
 *
 * Every view stays mounted so switching tabs keeps the scrollback; only the
 * active one is displayed.
 */
export function TerminalPanel({ terminals }: { terminals: TerminalsController }) {
  const { workspaceId, active, error } = terminals;

  return (
    <section className="terminal-panel" aria-label="Terminals">
      <header className="terminal-panel__bar">
        <div className="terminal-panel__tabs" role="tablist" aria-label="Terminal sessions">
          {terminals.terminals.map((session) => {
            const isActive = session.id === active?.id;
            const name = session.cwd === '' ? 'shell' : session.cwd;
            return (
              <div
                key={session.id}
                className={`terminal-tab${isActive ? ' terminal-tab--active' : ''}${
                  session.status === 'EXITED' ? ' terminal-tab--exited' : ''
                }`}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  className="terminal-tab__label"
                  onClick={() => terminals.activate(session.id)}
                  title={`${session.shell} — ${describeTerminal(session)}${
                    session.pid ? ` — pid ${session.pid}` : ''
                  }`}
                >
                  <span className="terminal-tab__name">{name}</span>
                  {session.status === 'EXITED' && (
                    <span className="terminal-tab__status">{describeTerminal(session)}</span>
                  )}
                </button>
                <button
                  type="button"
                  className="terminal-tab__close"
                  // Closing terminates the process tree; the core's kill is
                  // idempotent, so this is safe on an already-ended session.
                  onClick={() => void terminals.close(session.id)}
                  aria-label={`Close terminal ${name}`}
                >
                  ×
                </button>
              </div>
            );
          })}
        </div>
        <div className="terminal-panel__actions">
          {active?.status === 'RUNNING' && (
            <button type="button" onClick={() => void terminals.kill(active.id)}>
              Terminate
            </button>
          )}
          <button
            type="button"
            disabled={!workspaceId}
            title={workspaceId ? 'Start a shell in the workspace' : 'Open a workspace first'}
            onClick={() => {
              if (workspaceId) void terminals.spawn(workspaceId);
            }}
          >
            New terminal
          </button>
        </div>
      </header>

      {error && (
        <div className="terminal-panel__error">
          <ErrorEnvelope error={error} />
          <button type="button" onClick={terminals.clearError}>
            Dismiss
          </button>
        </div>
      )}

      <div className="terminal-panel__surface">
        {terminals.terminals.length === 0 && (
          <p className="terminal-panel__empty">
            {workspaceId
              ? 'No terminal yet. “New terminal” starts your login shell in the workspace.'
              : 'Open a workspace to start a terminal.'}
          </p>
        )}
        {terminals.terminals.map((session) => (
          <TerminalView
            key={session.id}
            session={session}
            active={session.id === active?.id}
            bus={terminals.bus}
            onData={(data) => terminals.write(session.id, data)}
            onResize={(cols, rows) => terminals.resize(session.id, cols, rows)}
          />
        ))}
      </div>
    </section>
  );
}
