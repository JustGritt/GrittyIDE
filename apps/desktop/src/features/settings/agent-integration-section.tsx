import { toCoreError } from '@gritty/protocol';
import { useCallback, useReducer } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { core } from '../../core/client';
import {
  agentIntegrationReducer,
  initialAgentIntegrationState,
  installBlockedReason,
  integrationSummary,
} from '../agents/agent-integration';

/**
 * The hook integration, re-parented from the agents popover onto the page
 * (T120-U §4, T221 §25, ADR-018 §1).
 *
 * Not a setting: there is no value in `settings.json` behind it, because what
 * it changes is a file in *another tool's* configuration. It is a section on
 * the Agents category for the reason it was a popover in the agents chrome —
 * this is where a person goes looking for it — and the flow is unchanged.
 *
 * The reducer, the summary and the blocked reason are imported, not copied:
 * `agent-integration.ts` is pure and already tested, and it stays the only
 * description of what this flow does.
 *
 * **The one thing the move could have broken, and did not.** In the popover the
 * read happened on the `<details>` toggle, which is ADR-018 §1 — Gritty must
 * not touch another tool's configuration as a side effect of running. A section
 * on a page is visible by default, so reading on mount would make it exactly
 * that side effect: opening settings would silently open
 * `~/.claude/settings.json`. So the read stays behind a button the user
 * presses, and nothing here runs until they press it.
 */
export function AgentIntegrationSection() {
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
    <section className="settings__panel" aria-label="Agent integration">
      <h3 className="settings__panel-title">Claude Code integration</h3>
      <p className="settings__description">
        Gritty can ask Claude Code to tell it when a session stops at a permission prompt, by adding
        one entry to <code>{status?.configPath ?? '~/.claude/settings.json'}</code>. Other tools'
        entries there are left exactly as they are, and the file is copied to a backup first.
      </p>

      {state.error && (
        <ErrorEnvelope error={state.error} onDismiss={() => dispatch({ type: 'clearError' })} />
      )}

      {!status ? (
        <p className="settings__placeholder">
          {busy
            ? 'Reading the configuration…'
            : 'Not read yet. Gritty does not open another tool’s configuration on its own.'}
        </p>
      ) : (
        <p className="settings__summary">{integrationSummary(status)}</p>
      )}

      {blocked && <p className="settings__note">{blocked}</p>}

      {proposed ? (
        <div className="settings__integration-diff">
          <p className="settings__summary">
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
              <pre className="settings__integration-command">{proposed.change.command}</pre>
              <ul className="settings__integration-events">
                {proposed.change.events.map((event) => (
                  <li key={event.event}>
                    {event.event}
                    {event.foreignEntries > 0 && (
                      <span className="settings__note">
                        {' '}
                        · {event.foreignEntries} other entr
                        {event.foreignEntries === 1 ? 'y' : 'ies'} left untouched
                      </span>
                    )}
                  </li>
                ))}
              </ul>
              {proposed.change.backupPath ? (
                <p className="settings__note">
                  The current file is copied to <code>{proposed.change.backupPath}</code> first.
                </p>
              ) : (
                <p className="settings__note">
                  The file does not exist yet and will be created carrying only this entry.
                </p>
              )}
            </>
          )}
          <div className="settings__actions">
            {proposed.change.events.length > 0 && (
              <button type="button" disabled={busy} onClick={() => void apply(proposed.direction)}>
                {proposed.direction === 'install' ? 'Apply' : 'Remove'}
              </button>
            )}
            <button type="button" disabled={busy} onClick={() => dispatch({ type: 'cancelled' })}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="settings__actions">
          {/* The explicit read. It was the `<details>` toggle in the popover;
              on a page it has to be a button, because the section is already
              open. */}
          <button type="button" disabled={busy} onClick={() => void read()}>
            {status ? 'Check again' : 'Check'}
          </button>
          <button
            type="button"
            disabled={busy || blocked !== null || !status}
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
    </section>
  );
}
