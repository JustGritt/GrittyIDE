import type { CoreError, HealthReport, Workspace } from '@gritty/protocol';
import type { ReactNode } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { describeHealth } from '../../core/health';

/** What the shell knows about the core while it is starting, and after. */
export type HealthState =
  | { status: 'loading' }
  | { status: 'ready'; health: HealthReport }
  | { status: 'error'; error: CoreError };

/**
 * The window's one top bar (T048): identity on the left, the open workspace
 * beside it, and the core's health as a dot on the right. The health facts are
 * a popover anchored to the bar rather than a panel that pushes the layout
 * down, so opening them never moves the editor.
 *
 * The island (T210) sits here too, for the same reason: the bar is where the
 * window's identity already is, so putting it there costs no layout (artboard
 * 6a, placement A). The bar owns the slot, not the island's state.
 */
export function AppBar({
  health,
  workspace,
  island,
}: {
  health: HealthState;
  workspace: Workspace | null;
  /** The island, when a workspace is open; nothing otherwise. */
  island?: ReactNode;
}) {
  return (
    <header className="appbar">
      <div className="appbar__identity">
        <h1 className="appbar__wordmark">Gritty</h1>
        {workspace ? (
          <span className="appbar__workspace">{workspace.name}</span>
        ) : (
          // Says the state rather than leaving a gap where a name goes (T049).
          <span className="appbar__workspace appbar__workspace--empty">No workspace open</span>
        )}
      </div>
      {workspace && (
        <code className="appbar__path" title={workspace.root}>
          {workspace.root}
        </code>
      )}
      <span className="appbar__spacer" />
      {island}
      <details className="appbar__health">
        <summary aria-live="polite">
          <span className={`appbar__dot appbar__dot--${dotState(health)}`} />
          {summaryLabel(health)}
        </summary>
        {health.status === 'ready' && <HealthFacts health={health.health} />}
        {health.status === 'error' && (
          <div className="appbar__facts">
            <p className="appbar__error">Core unreachable or incompatible.</p>
            <ErrorEnvelope error={health.error} />
          </div>
        )}
      </details>
    </header>
  );
}

function dotState(health: HealthState): string {
  if (health.status === 'ready') return 'ok';
  return health.status === 'error' ? 'error' : 'loading';
}

function summaryLabel(health: HealthState): string {
  if (health.status === 'loading') return 'Contacting core…';
  return health.status === 'ready' ? describeHealth(health.health) : 'Core unreachable';
}

function HealthFacts({ health }: { health: HealthReport }) {
  return (
    <dl className="appbar__facts">
      <dt>App</dt>
      <dd>{health.shell.appVersion}</dd>
      <dt>Core</dt>
      <dd>{health.coreVersion}</dd>
      <dt>Protocol</dt>
      <dd>{health.protocolVersion}</dd>
      <dt>Runtime</dt>
      <dd>
        {health.shell.runtime} {health.shell.runtimeVersion}
      </dd>
      <dt>WebView</dt>
      <dd>{health.shell.webviewVersion ?? 'unknown'}</dd>
      <dt>Platform</dt>
      <dd>
        {health.os}/{health.arch}
      </dd>
      <dt>Build</dt>
      <dd>{health.debug ? 'debug' : 'release'}</dd>
    </dl>
  );
}
