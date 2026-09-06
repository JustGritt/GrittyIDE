import { toCoreError } from '@gritty/protocol';
import { useCallback, useReducer } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { core } from '../../core/client';
import {
  agentIntegrationReducer,
  initialAgentIntegrationState,
  installBlockedReason,
  integrationSummary,
} from './agent-integration';

/**
 * The hook integration, offered in settings and nowhere else (T221 §25,
 * ADR-018 §1).
 *
 * This is the only surface in Gritty that writes into a file the user owns, so
 * the whole of it is one deliberate action at a time: nothing happens on first
 * run, nothing happens as a side effect of a scan, and there is no control here
 * that installs into more than one tool. It is a `<details>` in the agents
 * section's own chrome, which is where the terminal and the editor keep their
 * settings too.
 *
 * What it buys is one value in the roster — `BLOCKED`, "Pending validation" —
 * and what it costs is an entry appended to the provider's hook configuration.
 * The change is shown before it is made and the previous file is copied beside
 * it, so the answer to "what did that do to my settings" is always available.
 *
 * It is emphatically not a way to *control* an agent. A hook is a notification
 * the agent emits; installing one adds no way to answer a prompt, cancel a turn
 * or type into somebody else's terminal.
 */
export function AgentIntegrationSettings() {
  const [state, dispatch] = useReducer(agentIntegrationReducer, initialAgentIntegrationState);

  const read = useCallback(async () => {
    dispatch({ type: 'busy' });
    try {
      dispatch({
        type: 'status',
        status: await core.agents.integrationStatus({ provider: 'CLAUDE_CODE' }),
      });
    } catch (cause: unknown) {
      dispatch({ type: 'failed', error: toCoreError(cause) });
    }
  }, []);

  const propose = useCallback(async (direction: 'install' | 'uninstall') => {
    dispatch({ type: 'busy' });
    try {
      const request = { provider: 'CLAUDE_CODE', apply: false } as const;
      const change =
        direction === 'install'
          ? await core.agents.integrationInstall(request)
          : await core.agents.integrationUninstall(request);
      dispatch({ type: 'proposed', direction, change });
    } catch (cause: unknown) {
      dispatch({ type: 'failed', error: toCoreError(cause) });
    }
  }, []);

  const apply = useCallback(async (direction: 'install' | 'uninstall') => {
    dispatch({ type: 'busy' });
    try {
      const request = { provider: 'CLAUDE_CODE', apply: true } as const;
      if (direction === 'install') {
        await core.agents.integrationInstall(request);
      } else {
        await core.agents.integrationUninstall(request);
      }
      // Read it back rather than trusting the change we asked for: what the
      // file says now is the only status worth showing.
      dispatch({
        type: 'applied',
        status: await core.agents.integrationStatus({ provider: 'CLAUDE_CODE' }),
      });
    } catch (cause: unknown) {
      dispatch({ type: 'failed', error: toCoreError(cause) });
    }
  }, []);

  const { status, proposed, busy } = state;
  const blocked = installBlockedReason(status);

  return (
    <details
      className="prefs agents__integration"
      onToggle={(event) => {
        // Read only when it is opened. Discovery must not touch another tool's
        // configuration as a side effect of running (ADR-018 §1).
        if (event.currentTarget.open && !status && !busy) void read();
      }}
    >
      <summary aria-label="Agent integration settings" title="Agent integration settings">
        ⋯
      </summary>
      <div className="prefs__form">
        <p className="agents__hint">
          Gritty can ask Claude Code to tell it when a session stops at a permission prompt, by
          adding one entry to <code>{status?.configPath ?? '~/.claude/settings.json'}</code>. Other
          tools' entries there are left exactly as they are, and the file is copied to a backup
          first.
        </p>

        {state.error && (
          <ErrorEnvelope error={state.error} onDismiss={() => dispatch({ type: 'clearError' })} />
        )}

        {!status ? (
          <p className="island__empty">{busy ? 'Reading the configuration…' : 'Not read yet.'}</p>
        ) : (
          <p className="agents__summary">{integrationSummary(status)}</p>
        )}

        {blocked && <p className="agents__note">{blocked}</p>}

        {proposed ? (
          <div className="agents__integration-diff">
            <p className="agents__summary">
              {proposed.change.events.length === 0
                ? 'Nothing to change: the entry is already exactly as it should be.'
                : proposed.direction === 'install'
                  ? `Add this to ${proposed.change.events.length} event${
                      proposed.change.events.length === 1 ? '' : 's'
                    }:`
                  : `Remove this from ${proposed.change.events.length} event${
                      proposed.change.events.length === 1 ? '' : 's'
                    }:`}
            </p>
            {proposed.change.events.length > 0 && (
              <>
                <pre className="agents__integration-command">{proposed.change.command}</pre>
                <ul className="agents__integration-events">
                  {proposed.change.events.map((event) => (
                    <li key={event.event}>
                      {event.event}
                      {event.foreignEntries > 0 && (
                        <span className="agents__note">
                          {' '}
                          · {event.foreignEntries} other entr
                          {event.foreignEntries === 1 ? 'y' : 'ies'} left untouched
                        </span>
                      )}
                    </li>
                  ))}
                </ul>
                {proposed.change.backupPath ? (
                  <p className="agents__note">
                    The current file is copied to <code>{proposed.change.backupPath}</code> first.
                  </p>
                ) : (
                  <p className="agents__note">
                    The file does not exist yet and will be created carrying only this entry.
                  </p>
                )}
              </>
            )}
            <div className="agents__integration-actions">
              {proposed.change.events.length > 0 && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void apply(proposed.direction)}
                >
                  {proposed.direction === 'install' ? 'Apply' : 'Remove'}
                </button>
              )}
              <button type="button" disabled={busy} onClick={() => dispatch({ type: 'cancelled' })}>
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <div className="agents__integration-actions">
            <button
              type="button"
              disabled={busy || blocked !== null}
              onClick={() => void propose('install')}
            >
              {status?.status === 'BROKEN' ? 'Repair…' : 'Install…'}
            </button>
            {(status?.status === 'INSTALLED' || status?.status === 'BROKEN') && (
              <button type="button" disabled={busy} onClick={() => void propose('uninstall')}>
                Uninstall…
              </button>
            )}
          </div>
        )}
      </div>
    </details>
  );
}
