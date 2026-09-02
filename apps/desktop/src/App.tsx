import { useEffect, useState } from 'react';
import { fetchHealth } from './core/bridge';
import { type AppHealth, describeHealth } from './core/health';

type HealthState =
  | { status: 'loading' }
  | { status: 'ready'; health: AppHealth }
  | { status: 'error'; message: string };

export function App() {
  const [state, setState] = useState<HealthState>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    fetchHealth().then(
      (health) => {
        if (!cancelled) setState({ status: 'ready', health });
      },
      (error: unknown) => {
        if (!cancelled) setState({ status: 'error', message: errorMessage(error) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="shell">
      <header className="shell__header">
        <h1>Gritty</h1>
        <p className="shell__tagline">Agent-native development environment. Phase 0 shell.</p>
      </header>
      <section className="shell__status" aria-live="polite">
        {state.status === 'loading' && <p>Contacting core…</p>}
        {state.status === 'ready' && <HealthFacts health={state.health} />}
        {state.status === 'error' && (
          <p className="shell__error" role="alert">
            Core unreachable: {state.message}
          </p>
        )}
      </section>
    </main>
  );
}

function HealthFacts({ health }: { health: AppHealth }) {
  return (
    <>
      <p className="shell__ok">{describeHealth(health)}</p>
      <dl className="shell__facts">
        <dt>App</dt>
        <dd>{health.appVersion}</dd>
        <dt>Core</dt>
        <dd>{health.core.coreVersion}</dd>
        <dt>Tauri</dt>
        <dd>{health.tauriVersion}</dd>
        <dt>WebView</dt>
        <dd>{health.webviewVersion ?? 'unknown'}</dd>
        <dt>Platform</dt>
        <dd>
          {health.core.os}/{health.core.arch}
        </dd>
        <dt>Build</dt>
        <dd>{health.core.debug ? 'debug' : 'release'}</dd>
      </dl>
    </>
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === 'string' ? error : JSON.stringify(error);
}
