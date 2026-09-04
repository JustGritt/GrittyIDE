import { type CoreError, type HealthReport, toCoreError } from '@gritty/protocol';
import { useEffect, useReducer, useState } from 'react';
import { ErrorEnvelope } from './components/error-envelope';
import { core } from './core/client';
import { describeHealth } from './core/health';
import { useDocuments } from './features/documents/use-documents';
import { EditorPane } from './features/editor/editor-pane';
import { useEditorPreferences } from './features/editor/editor-preferences';
import { useEditorBinding } from './features/editor/use-editor-binding';
import { ExplorerPanel } from './features/explorer/explorer-panel';
import { viewModesReducer } from './features/markdown/view-modes';
import { OperationsPanel } from './features/operations/operations-panel';
import { DEFAULT_LAYOUT, type Layout } from './features/session/persisted-layout';
import { SessionNotices } from './features/session/session-notices';
import { useSession } from './features/session/use-session';

type HealthState =
  | { status: 'loading' }
  | { status: 'ready'; health: HealthReport }
  | { status: 'error'; error: CoreError };

export function App() {
  const [state, setState] = useState<HealthState>({ status: 'loading' });
  const documents = useDocuments();
  // State the session persists lives here (T060): preferences, per-tab view
  // modes, the editor binding (cursor and scroll) and the explorer layout.
  const { prefs, update: updatePrefs, replace: replacePrefs } = useEditorPreferences();
  const [modes, dispatchMode] = useReducer(viewModesReducer, {});
  useEffect(
    () => dispatchMode({ type: 'prune', open: documents.state.order }),
    [documents.state.order],
  );
  const editor = useEditorBinding(documents, prefs);
  const [layout, setLayout] = useState<Layout>(DEFAULT_LAYOUT);
  const session = useSession({
    documents,
    editor,
    prefs,
    setPrefs: replacePrefs,
    viewModes: modes,
    dispatchViewMode: dispatchMode,
    layout,
    setLayout,
  });

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
      <SessionNotices session={session} />
      <div className="shell__body">
        <aside className="shell__sidebar">
          <ExplorerPanel
            onOpenFile={(workspaceId, path) => void documents.openFile(workspaceId, path)}
            showHidden={layout.explorer.showHidden}
            onShowHiddenChange={(showHidden) => setLayout({ explorer: { showHidden } })}
          />
          {import.meta.env.DEV && <OperationsPanel />}
        </aside>
        <EditorPane
          documents={documents}
          editor={editor}
          prefs={prefs}
          onPrefsChange={updatePrefs}
          modes={modes}
          dispatchMode={dispatchMode}
          session={session}
        />
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
