import { ErrorEnvelope } from '../../components/error-envelope';
import type { TerminalLayout } from '../session/persisted-layout';
import { selectedProfile, shellName, spawnShell } from './terminal-profiles';
import { describeTerminal } from './terminal-reducer';
import { TerminalSettings } from './terminal-settings';
import { TerminalView } from './terminal-view';
import type { TerminalsController } from './use-terminals';

/** Where a session started, for its tooltip. A home shell has no workspace. */
function describeCwd(session: { cwd?: string }): string {
  if (session.cwd === undefined) return 'your home directory';
  return session.cwd === '' ? 'the workspace root' : session.cwd;
}

/**
 * The terminal dock: a tab per session and the active session's xterm.
 *
 * Every view stays mounted so switching tabs keeps the scrollback; only the
 * active one is displayed.
 */
export function TerminalPanel({
  terminals,
  settings,
  onSettingsChange,
}: {
  terminals: TerminalsController;
  /** The persisted terminal settings (T095): shell, custom shells, font. */
  settings: TerminalLayout;
  onSettingsChange: (patch: Partial<TerminalLayout>) => void;
}) {
  const { workspaceId, active, error, profiles } = terminals;
  // A remembered shell that no longer exists falls back to the default rather
  // than leaving the picker showing something that cannot start.
  const profile = selectedProfile(profiles, settings.profilePath);
  // A broken custom entry belongs in the settings, where it can be explained
  // or removed — not in a menu of things to start.
  const startable = profiles.filter((candidate) => candidate.available);

  return (
    <section className="terminal-panel" aria-label="Terminals">
      <header className="terminal-panel__bar">
        <div className="terminal-panel__tabs" role="tablist" aria-label="Terminal sessions">
          {terminals.terminals.map((session) => {
            const isActive = session.id === active?.id;
            // The shell is what tells two tabs apart once profiles exist;
            // the directory is in the tooltip, where a long path belongs.
            const name = shellName(session.shell);
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
                  title={`${session.shell} in ${describeCwd(session)} — ${describeTerminal(
                    session,
                  )}${session.pid ? ` — pid ${session.pid}` : ''}`}
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
          {startable.length > 1 && (
            <select
              className="terminal-panel__profile"
              aria-label="Shell for new terminals"
              value={profile?.path ?? ''}
              title={profile ? profile.path : 'The default login shell'}
              onChange={(event) => onSettingsChange({ profilePath: event.target.value || null })}
            >
              {startable.map((candidate) => (
                <option key={candidate.path} value={candidate.path}>
                  {candidate.name}
                  {candidate.isDefault ? ' (default)' : ''}
                </option>
              ))}
            </select>
          )}
          <button
            type="button"
            title={`Start ${profile ? profile.path : 'your login shell'} in ${
              workspaceId ? 'the workspace' : 'your home directory'
            }`}
            onClick={() => void terminals.spawn(workspaceId, spawnShell(profile))}
          >
            New terminal
          </button>
          <TerminalSettings settings={settings} profiles={profiles} onChange={onSettingsChange} />
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
              ? 'No terminal yet. “New terminal” starts your login shell at the workspace root.'
              : 'No terminal yet. “New terminal” starts your login shell in your home directory.'}
          </p>
        )}
        {terminals.terminals.map((session) => (
          <TerminalView
            key={session.id}
            session={session}
            active={session.id === active?.id}
            bus={terminals.bus}
            fontFamily={settings.fontFamily}
            fontSize={settings.fontSize}
            onData={(data) => terminals.write(session.id, data)}
            onResize={(cols, rows) => terminals.resize(session.id, cols, rows)}
          />
        ))}
      </div>
    </section>
  );
}
