import { type CoreError, type HealthReport, toCoreError } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { ErrorEnvelope } from './components/error-envelope';
import { core } from './core/client';
import { describeHealth } from './core/health';
import { useDocuments } from './features/documents/use-documents';
import { EditorPane } from './features/editor/editor-pane';
import { ExplorerPanel } from './features/explorer/explorer-panel';
import { OperationsPanel } from './features/operations/operations-panel';

type HealthState =
  | { status: 'loading' }
  | { status: 'ready'; health: HealthReport }
  | { status: 'error'; error: CoreError };

export function App() {
  const [state, setState] = useState<HealthState>({ status: 'loading' });
  const documents = useDocuments();

  useEffect(() => {
    let cancelled = false;
    core.health().then(
      (health) => {
        if (!cancelled) setState({ status: 'ready', health });
      },
      (cause: unknown) => {
        if (!cancelled) setState({ status: 'error', error: toCoreError(cause) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="shell">
      <header className="shell__header">
        <div>
          <h1>Gritty</h1>
          <p className="shell__tagline">Agent-native development environment. Phase 0 shell.</p>
        </div>
        <section className="shell__status" aria-live="polite">
          {state.status === 'loading' && <p>Contacting core…</p>}
          {state.status === 'ready' && <HealthFacts health={state.health} />}
          {state.status === 'error' && (
            <>
              <p className="shell__error">Core unreachable or incompatible.</p>
              <ErrorEnvelope error={state.error} />
            </>
          )}
        </section>
      </header>
      <div className="shell__body">
        <aside className="shell__sidebar">
          <ExplorerPanel
            onOpenFile={(workspaceId, path) => void documents.openFile(workspaceId, path)}
          />
          {import.meta.env.DEV && <OperationsPanel />}
        </aside>
        <EditorPane documents={documents} />
      </div>
    </main>
  );
}

function HealthFacts({ health }: { health: HealthReport }) {
  return (
    <details>
      <summary className="shell__ok">{describeHealth(health)}</summary>
      <dl className="shell__facts">
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
    </details>
  );
}
