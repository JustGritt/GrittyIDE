import { toCoreError } from '@gritty/protocol';
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
import { useExplorer } from './features/explorer/use-explorer';
import { Launcher } from './features/launcher/launcher';
import { useLastSession } from './features/launcher/use-last-session';
import { viewModesReducer } from './features/markdown/view-modes';
import { OperationsPanel } from './features/operations/operations-panel';
import { DEFAULT_LAYOUT, type Layout, PANE_LIMITS } from './features/session/persisted-layout';
import { SessionNotices } from './features/session/session-notices';
import { useSession } from './features/session/use-session';
import { AppBar, type HealthState } from './features/shell/app-bar';
import { type Accelerator, acceleratorForOs } from './features/shell/open-shortcuts';
import { useOpenShortcuts } from './features/shell/use-open-shortcuts';
import { TerminalPanel } from './features/terminal/terminal-panel';
import { useTerminals } from './features/terminal/use-terminals';
import { AttentionQueue } from './features/workstreams/attention-queue';
import { AgentIsland } from './features/workstreams/island';
import { useWorkstreams } from './features/workstreams/use-workstreams';

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
  // The workspace lives here (T049), not inside the explorer: the launcher
  // opens folders through the same controller and the top bar names the same
  // workspace, so there is one subscription and one tree.
  const explorer = useExplorer({
    showHidden: layout.explorer.showHidden,
    onShowHiddenChange: (showHidden) =>
      setLayout((current) => ({ ...current, explorer: { showHidden } })),
  });
  const workspace = explorer.state.workspace;
  // Only the launcher has a path field, so ⌘⇧O is its shortcut and the shell
  // holds the disclosure it opens.
  const [pathOpen, setPathOpen] = useState(false);
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
  // Workstreams and their decisions (T210). The island lives in the top bar
  // and the queue replaces the editor body; both read this one controller, so
  // answering a question in either clears it in the other.
  const workstreams = useWorkstreams(workspace);
  // What the previous window had open, for the launcher's resume rail (T051).
  // Asked for whenever no workspace is open, which is exactly when the
  // launcher is on screen — including after the user closes one mid-run.
  const lastSession = useLastSession(workspace);
  const [islandOpen, setIslandOpen] = useState(false);
  const [queueOpen, setQueueOpen] = useState(false);
  // Closing the workspace ends the queue with it: reopening should land in the
  // editor, not in a triage view of work you have not looked at yet.
  useEffect(() => {
    if (!workspace) {
      setQueueOpen(false);
      setIslandOpen(false);
    }
  }, [workspace]);
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

  // The core knows the platform; `navigator.platform` is deprecated and lies
  // under emulation. Null until health lands, which leaves the shortcuts inert
  // rather than guessing which modifier is "the command key".
  const accelerator: Accelerator | null =
    state.status === 'ready' ? acceleratorForOs(state.health.os) : null;
  const openFolder = useCallback(
    (path?: string) => {
      setPathOpen(false);
      void explorer.openFolder(path);
    },
    [explorer.openFolder],
  );
  useOpenShortcuts(accelerator, {
    onOpenFolder: () => openFolder(),
    // Only the launcher has a field to focus; with a workspace open the chord
    // is left to whatever else wants it.
    onOpenPath: workspace ? undefined : () => setPathOpen(true),
  });

  /**
   * Open a workspace and, when asked, put its persisted tabs back. The startup
   * restore runs once per WebView, so a workspace opened from the rail cannot
   * go through it; `session.resumeTabs` is the same code path on demand.
   */
  const resume = useCallback(
    async (root: string, withTabs: boolean) => {
      const tabs = lastSession.value?.tabs ?? [];
      setPathOpen(false);
      const opened = await explorer.openFolder(root);
      if (!withTabs || tabs.length === 0) return;
      // `openFolder` reports its own failures; without a workspace there is
      // nothing to reopen into.
      const workspaceId = opened?.id;
      if (workspaceId) await session.resumeTabs(workspaceId, tabs);
    },
    [explorer.openFolder, lastSession.value, session.resumeTabs],
  );

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
      <AppBar
        health={state}
        workspace={workspace}
        island={
          workspace ? (
            <AgentIsland
              controller={workstreams}
              open={islandOpen}
              onOpenChange={setIslandOpen}
              onOpenQueue={() => {
                setIslandOpen(false);
                setQueueOpen(true);
              }}
            />
          ) : null
        }
      />
      <SessionNotices session={session} />
      <DropOverlay drop={drop} />
      {/* The panels are hidden until a workspace exists (T049, artboard 3a),
          and the launcher waits for the session to settle rather than
          flashing while the last workspace is being reopened: `workspace` is
          null both before and after that, `phase` is what tells them apart. */}
      {workspace && queueOpen ? (
        <div className="shell__body shell__body--queue">
          <AttentionQueue controller={workstreams} onClose={() => setQueueOpen(false)} />
        </div>
      ) : workspace ? (
        <div className="shell__body">
          <aside className="shell__sidebar">
            <ExplorerPanel
              explorer={explorer}
              onOpenFile={(workspaceId, path) => void documents.openFile(workspaceId, path)}
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
      ) : session.state.phase === 'ready' ? (
        <Launcher
          accelerator={accelerator}
          pathOpen={pathOpen}
          onPathOpenChange={setPathOpen}
          onOpen={openFolder}
          onPickFolder={() => openFolder()}
          lastSession={lastSession.value}
          onResume={(root, withTabs) => {
            lastSession.dismiss();
            void resume(root, withTabs);
          }}
          onDismissLastSession={lastSession.dismiss}
        />
      ) : null}
      {/* The dock stays mounted with no workspace open: T095 roots a shell in
          the home directory when there is none, and unmounting would dispose
          every xterm and its scrollback. */}
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
        workspace={workspace}
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
