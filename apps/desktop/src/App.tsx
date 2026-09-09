import { type SessionTab, toCoreError } from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { Splitter } from './components/splitter';
import { boundsWithin } from './components/splitter-geometry';
import { core } from './core/client';
import { SidebarAgents } from './features/agents/agents-section';
import {
  askingSessions,
  isPermissionQueueChord,
  PermissionBanner,
} from './features/agents/permission-banner';
import { rosterRows } from './features/agents/roster-model';
import { useAgentConsole } from './features/agents/use-agent-console';
import { useAgents } from './features/agents/use-agents';
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
import { ChangesPanel } from './features/git/changes-panel';
import type { SidebarTab } from './features/git/sidebar-tabs';
import { Launcher } from './features/launcher/launcher';
import { useLastSession } from './features/launcher/use-last-session';
import { viewModesReducer } from './features/markdown/view-modes';
import { OperationsPanel } from './features/operations/operations-panel';
import { RunsPanel } from './features/runs/runs-panel';
import { useRuns } from './features/runs/use-runs';
import { DEFAULT_LAYOUT, type Layout, PANE_LIMITS } from './features/session/persisted-layout';
import { SessionNotices } from './features/session/session-notices';
import { useSession } from './features/session/use-session';
import { AppBar, type HealthState } from './features/shell/app-bar';
import { type Accelerator, acceleratorForOs } from './features/shell/open-shortcuts';
import { useOpenShortcuts } from './features/shell/use-open-shortcuts';
import { cwdForDocument } from './features/terminal/terminal-cwd';
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
  // Files and folders dropped onto the window (classified by the core, opened
  // through the ordinary commands). Declared after the explorer because that
  // is where an opened workspace has to land: the drop reports what it opened
  // rather than waiting for an event the core does not always have to emit.
  const drop = useFileDrop(documents, explorer.adoptWorkspace);
  // Only the launcher has a path field, so ⌘⇧O is its shortcut and the shell
  // holds the disclosure it opens.
  const [pathOpen, setPathOpen] = useState(false);
  // Which of the sidebar's two sections is showing (T100-U). Deliberately not
  // persisted: `persisted-layout.ts` is another ticket's file this wave, and a
  // session that reopens on Changes would re-read the repository before the
  // user asked for it.
  const [sidebarTab, setSidebarTab] = useState<SidebarTab>('files');
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
  // Machine-wide, so it takes no workspace and outlives changes of one. One
  // controller for every surface that reads it: the sidebar section, the
  // island's count and the console, and three rosters would drift apart.
  const agents = useAgents();
  // Runs Gritty owns, scoped to the open workspace — the other half of the
  // sidebar's Agents section. `agents` is machine-wide and observed; these are
  // started here and cancellable, and `docs/architecture/agent-runs.md` keeps
  // the two apart everywhere but the row model that draws them side by side.
  const runs = useRuns(workspace);
  // The agent consoles open as editor tabs (T233-C, ADR-026), and the one
  // follower they share between them. Nothing here is persisted: a session id
  // is rediscovered each launch, which is both why restoring a console would
  // be wrong and why none of this needs the migration T231 priced it on.
  const agentConsole = useAgentConsole({
    agents,
    // The runs half of the console. One controller, shared with the sidebar's
    // rows and the editor pane, so a run cannot be in two states at once.
    runs,
    // How a file the agent touched reaches the editor (T232). The console is
    // given the document store rather than its own way in, so opening from the
    // timeline is the same `documents.status` / `documents.open` pair the
    // explorer and the launcher already use.
    documents,
    workspaceId: workspace?.id ?? null,
  });
  // One list for the sidebar's Agents section: the runs Gritty owns and the
  // sessions it merely observes, projected onto a common row and ordered
  // needs-you first. Sessions are filtered to this workspace inside the model;
  // the island above stays machine-wide, which is the split that lets both
  // exist without being two rosters of the same thing.
  const agentRows = useMemo(
    () => rosterRows(runs.runs, agents.roster, workstreams.workstreams),
    [runs.runs, agents.roster, workstreams.workstreams],
  );
  // What the status bar says while a console is showing, for either lane.
  // Read off the same row the sidebar draws, so the bar and the list cannot
  // call one agent two names; `null` whenever a document has the pane.
  const consoleFacts = useMemo(() => {
    const key = agentConsole.activeId;
    if (key === null) return null;
    const row = agentRows.find((item) => item.key === key);
    return row ? { label: row.id, status: row.status.label } : null;
  }, [agentConsole.activeId, agentRows]);
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

  // ⌥⌘A opens the queue on its permissions (artboard 12b). A fourth hand-gated
  // listener, because there is still no shortcut registry — the same apology
  // `agents-section.tsx`, `island.tsx` and `terminal-panel.tsx` each make. It does
  // not collide with the island's ⌥A: every existing matcher rejects `metaKey`.
  useEffect(() => {
    if (!workspace) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isPermissionQueueChord(event)) return;
      event.preventDefault();
      setIslandOpen(false);
      setQueueOpen(true);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [workspace]);

  const active = activeTab(documents.state);
  const activeEol = active ? (editor.eols[active.document.id] ?? null) : null;
  const session = useSession({
    documents,
    // The startup restore is told which workspace is open by its own
    // response; without this the shell would wait for `WorkspaceOpened` and
    // could miss it, leaving the launcher up over an open workspace.
    adoptWorkspace: explorer.adoptWorkspace,
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
    async (root: string, tabs: readonly SessionTab[]) => {
      setPathOpen(false);
      const opened = await explorer.openFolder(root);
      if (tabs.length === 0) return;
      // `openFolder` reports its own failures; without a workspace there is
      // nothing to reopen into.
      if (opened) await session.resumeTabs(opened.id, tabs);
    },
    [explorer.openFolder, session.resumeTabs],
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
              agents={agents}
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
      {/* Above the panes and never over them: a background ask states itself
          and gets out of the way (artboard 12b). Not while the queue is open —
          the queue is where it is going, and a banner pointing at the surface
          you are already looking at is noise. */}
      {workspace && !queueOpen && (
        <PermissionBanner
          sessions={agents.roster.sessions}
          onOpenQueue={() => {
            setIslandOpen(false);
            setQueueOpen(true);
          }}
        />
      )}
      <DropOverlay drop={drop} />
      {/* The panels are hidden until a workspace exists (T049, artboard 3a),
          and the launcher waits for the session to settle rather than
          flashing while the last workspace is being reopened: `workspace` is
          null both before and after that, `phase` is what tells them apart. */}
      {workspace && queueOpen ? (
        <div className="shell__body shell__body--queue">
          <AttentionQueue
            controller={workstreams}
            agents={agents}
            onClose={() => setQueueOpen(false)}
          />
        </div>
      ) : workspace ? (
        <div className="shell__body">
          <aside className="shell__sidebar">
            {/* Above the tree, and pinned there (artboard 12a): an agent that
                stopped to ask you something must not be one tab away. */}
            <SidebarAgents
              rows={agentRows}
              needsYou={agents.roster.needsYou}
              loaded={agents.loaded}
              processEvidence={agents.roster.processEvidence}
              error={agents.error}
              onDismissError={agents.clearError}
              open={layout.agents.open}
              onOpenChange={(open) => setLayout((current) => ({ ...current, agents: { open } }))}
              // A row opens its console as an editor tab, exactly as a file
              // opens as one: a single click is a preview the next click
              // replaces, a double click keeps it (T041's rule, not a second).
              // The row's own key *is* the console key, for both lanes, so
              // nothing here has to know which one a row came from (T310-U).
              selectedKey={agentConsole.activeId}
              onSelect={(row) => agentConsole.show(row.key)}
              onKeep={(row) => agentConsole.keep(row.key)}
              onCancel={(runId) => void runs.cancel(runId)}
              onRescan={() => void agents.refresh()}
              busy={agents.busy}
              // The island's ⌥ chords win while it is open — there is no
              // shortcut registry to arbitrate for them.
              chords={!islandOpen}
            />
            <ExplorerPanel
              explorer={explorer}
              onOpenFile={(workspaceId, path, preview) =>
                void documents.openFile(workspaceId, path, { preview })
              }
              tab={sidebarTab}
              onTabChange={setSidebarTab}
              // An element, not a rendered panel: the explorer mounts it only
              // while its tab is selected, which is what keeps `git.status`
              // from being polled behind the tree.
              changes={<ChangesPanel workspaceId={explorer.state.workspace?.id ?? null} />}
            />
            {import.meta.env.DEV && <OperationsPanel />}
            {import.meta.env.DEV && (
              <RunsPanel workspace={workspace} workstreams={workstreams.workstreams} />
            )}
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
            onRequestCloseMany={closeFlow.requestMany}
            agents={agents}
            runs={runs}
            rows={agentRows}
            console={agentConsole}
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
          onResume={(root, tabs) => {
            lastSession.dismiss();
            void resume(root, tabs);
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
        {/* One 32px row (T096, artboard 8a): the panel owns the label that used
            to be the dock's toggle, so the sessions stay legible while it is
            collapsed. Kept mounted throughout — unmounting would dispose every
            xterm and lose the scrollback of running shells. */}
        <TerminalPanel
          terminals={terminals}
          open={terminalOpen}
          onOpenChange={setTerminalOpen}
          workspace={workspace}
          // A new terminal starts where you are working, not at the root
          // (T098). Only for a document of the *open* workspace: a tab from
          // another one has no path this workspace could resolve.
          cwd={
            active && workspace && active.document.workspaceId === workspace.id
              ? cwdForDocument(active.document.path)
              : undefined
          }
          // The island binds ⌥1…⌥9 to a decision's options while it is open;
          // two live handlers would answer a question and switch a terminal on
          // one keystroke.
          chords={!islandOpen}
          settings={layout.terminal}
          onSettingsChange={(patch) =>
            setLayout((current) => ({ ...current, terminal: { ...current.terminal, ...patch } }))
          }
        />
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
        permissionsWaiting={askingSessions(agents.roster.sessions).length}
        // While a console has the pane, the bar describes the agent rather
        // than the document that is still the active one underneath it.
        agentConsole={consoleFacts}
      />
      {closeFlow.tab && (
        <CloseDialog
          tab={closeFlow.tab}
          busy={closeFlow.busy}
          queued={closeFlow.queued}
          onCancel={closeFlow.cancel}
          onDiscard={closeFlow.discard}
          onSave={closeFlow.save}
        />
      )}
    </main>
  );
}
