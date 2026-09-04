import { toCoreError, type Workspace } from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { Splitter } from './components/splitter';
import { boundsWithin } from './components/splitter-geometry';
import { core } from './core/client';
import { activeTab } from './features/documents/documents-reducer';
import { useDocuments } from './features/documents/use-documents';
import { DropOverlay } from './features/drop/drop-overlay';
import { useFileDrop } from './features/drop/use-file-drop';
import { CloseDialog } from './features/editor/close-dialog';
import { EditorPane } from './features/editor/editor-pane';
import { useEditorPreferences } from './features/editor/editor-preferences';
import { StatusBar } from './features/editor/status-bar';
import { useCloseFlow } from './features/editor/use-close-flow';
import { useEditorBinding } from './features/editor/use-editor-binding';
import { ExplorerPanel } from './features/explorer/explorer-panel';
import { viewModesReducer } from './features/markdown/view-modes';
import { OperationsPanel } from './features/operations/operations-panel';
import { DEFAULT_LAYOUT, type Layout, PANE_LIMITS } from './features/session/persisted-layout';
import { SessionNotices } from './features/session/session-notices';
import { useSession } from './features/session/use-session';
import { AppBar, type HealthState } from './features/shell/app-bar';
import { TerminalPanel } from './features/terminal/terminal-panel';
import { useTerminals } from './features/terminal/use-terminals';

/** What the editor needs beside the sidebar, and above the dock. */
const EDITOR_MIN_WIDTH = 360;
const EDITOR_MIN_HEIGHT = 200;

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
  // Files and folders dropped onto the window (classified by the core, opened
  // through the ordinary commands).
  const drop = useFileDrop(documents);
  const [layout, setLayout] = useState<Layout>(DEFAULT_LAYOUT);
  // The top bar names the open workspace; the explorer owns the subscription
  // and reports it up (T048).
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  // The gutters write their CSS variables straight onto this element while
  // dragging, so a drag never re-renders the shell (T046).
  const shell = useRef<HTMLElement | null>(null);
  // Terminal sessions live beside the editor, not inside it: output never
  // passes through editor state (T090).
  const terminals = useTerminals(layout.terminal.customShells);
  const [terminalOpen, setTerminalOpen] = useState(false);
  // Closing a tab with unsaved edits asks first. The tab strip raises the
  // request and the status bar does too, so the flow belongs to the shell.
  const closeFlow = useCloseFlow(documents);
  const active = activeTab(documents.state);
  const activeEol = active ? (editor.eols[active.document.id] ?? null) : null;
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

  const sidebarBounds = useCallback(
    (root: HTMLElement) =>
      boundsWithin(
        root.clientWidth,
        PANE_LIMITS.sidebar.min,
        EDITOR_MIN_WIDTH,
        PANE_LIMITS.sidebar.max,
      ),
    [],
  );
  const terminalBounds = useCallback(
    (root: HTMLElement) =>
      boundsWithin(
        root.clientHeight,
        PANE_LIMITS.terminal.min,
        EDITOR_MIN_HEIGHT,
        PANE_LIMITS.terminal.max,
      ),
    [],
  );

  return (
    <main
      className="shell"
      ref={shell}
      style={
        {
          '--sidebar-width': `${layout.panes.sidebarWidth}px`,
          '--terminal-height': `${layout.panes.terminalHeight}px`,
        } as React.CSSProperties
      }
    >
      <AppBar health={state} workspace={workspace} />
      <SessionNotices session={session} />
      <DropOverlay drop={drop} />
      <div className="shell__body">
        <aside className="shell__sidebar">
          <ExplorerPanel
            onOpenFile={(workspaceId, path) => void documents.openFile(workspaceId, path)}
            showHidden={layout.explorer.showHidden}
            onShowHiddenChange={(showHidden) =>
              setLayout((current) => ({ ...current, explorer: { showHidden } }))
            }
            onWorkspaceChange={setWorkspace}
          />
          {import.meta.env.DEV && <OperationsPanel />}
        </aside>
        <Splitter
          orientation="vertical"
          label="Resize the sidebar"
          value={layout.panes.sidebarWidth}
          fallback={PANE_LIMITS.sidebar.default}
          direction={1}
          target={shell}
          property="--sidebar-width"
          bounds={sidebarBounds}
          onCommit={(sidebarWidth) =>
            setLayout((current) => ({ ...current, panes: { ...current.panes, sidebarWidth } }))
          }
        />
        <EditorPane
          documents={documents}
          editor={editor}
          prefs={prefs}
          onPrefsChange={updatePrefs}
          modes={modes}
          dispatchMode={dispatchMode}
          session={session}
          onRequestClose={closeFlow.request}
        />
      </div>
      <footer className="shell__dock">
        {/* Only while the dock is open: resizing a collapsed panel means
            nothing, and an invisible tab stop is worse than no gutter. */}
        {terminalOpen && (
          <Splitter
            orientation="horizontal"
            label="Resize the terminal panel"
            value={layout.panes.terminalHeight}
            fallback={PANE_LIMITS.terminal.default}
            // The dock is anchored to the bottom, so it grows as the pointer
            // moves up — against the axis.
            direction={-1}
            target={shell}
            property="--terminal-height"
            bounds={terminalBounds}
            onCommit={(terminalHeight) =>
              setLayout((current) => ({ ...current, panes: { ...current.panes, terminalHeight } }))
            }
          />
        )}
        <button
          type="button"
          className="shell__dock-toggle"
          aria-expanded={terminalOpen}
          onClick={() => setTerminalOpen((open) => !open)}
        >
          Terminal
          {terminals.terminals.length > 0 && (
            <span className="shell__dock-count">{terminals.terminals.length}</span>
          )}
        </button>
        {/* Kept mounted while collapsed: unmounting would dispose every xterm
            and lose the scrollback of running shells. */}
        <div className="shell__dock-body" hidden={!terminalOpen}>
          <TerminalPanel
            terminals={terminals}
            open={terminalOpen}
            settings={layout.terminal}
            onSettingsChange={(patch) =>
              setLayout((current) => ({ ...current, terminal: { ...current.terminal, ...patch } }))
            }
          />
        </div>
      </footer>
      {/* The last 26px of the window, below the dock: what is running, and what
          the contract knows about the active document (T048). */}
      <StatusBar
        health={state}
        tab={active}
        eol={activeEol}
        subscribeCursor={editor.subscribeCursor}
        documents={documents}
        onClose={closeFlow.request}
      />
      {closeFlow.tab && (
        <CloseDialog
          tab={closeFlow.tab}
          busy={closeFlow.busy}
          onCancel={closeFlow.cancel}
          onDiscard={closeFlow.discard}
          onSave={closeFlow.save}
        />
      )}
    </main>
  );
}
