import { COMMANDS, type CommandName } from './commands';
import { type CoreError, toCoreError } from './errors';
import { type Diagnostic, EventHub, type Unsubscribe } from './events';
import type { Transport } from './transport';
import {
  type AgentRoster,
  type CandidatesRequest,
  type CloseDocumentRequest,
  type CloseWorkspaceRequest,
  CORE_EVENT_CHANNEL,
  type CreateWorkstreamRequest,
  type Decision,
  type DiscardSnapshotRequest,
  type DismissDecisionRequest,
  type Document,
  type DocumentContent,
  type DocumentStatusRequest,
  type EventEnvelope,
  type EventName,
  type EventOf,
  type ForgetWorkspaceRequest,
  type HealthReport,
  type KillTerminalRequest,
  type LastSession,
  type ListAgentSessionsRequest,
  type ListDecisionsRequest,
  type ListDirectoryRequest,
  type ListDirectoryResponse,
  type ListProfilesRequest,
  type ListWorkstreamsRequest,
  type OpenDocumentRequest,
  type OpenExternalRequest,
  type OpenWorkspaceRequest,
  type Operation,
  type OperationId,
  PROTOCOL_VERSION,
  type RaiseDecisionRequest,
  type RecentWorkspace,
  type RecentWorkspacesRequest,
  type RecolorTerminalRequest,
  type RecoverSnapshotRequest,
  type RecoveryCandidate,
  type ReloadDocumentRequest,
  type RemoveWorkstreamRequest,
  type RenameTerminalRequest,
  type ResizeTerminalRequest,
  type ResolveDecisionRequest,
  type SaveDocumentRequest,
  type SessionRestoreResponse,
  type SpawnTerminalRequest,
  type StartDemoRequest,
  type Terminal,
  type TerminalProfile,
  type TransitionWorkstreamRequest,
  type UpdateDocumentRequest,
  type UpdateSessionRequest,
  type UpdateWorkstreamRequest,
  type Workspace,
  type WorkspaceId,
  type Workstream,
  type WriteTerminalRequest,
} from './types';
import { assertCompatible } from './version';

export interface CoreClientOptions {
  /** Receives non-fatal observations; defaults to `console.warn`. */
  onDiagnostic?: Diagnostic;
  /** Override the protocol version the client claims (tests only). */
  clientVersion?: string;
}

export interface CoreClient {
  /**
   * `core.health`. Also the compatibility probe: the first call checks the
   * protocol MAJOR and its result is reused until it fails. A mismatch makes
   * every later command reject with `ProtocolMismatch`.
   */
  health(): Promise<HealthReport>;
  operations: {
    list(): Promise<Operation[]>;
    cancel(operationId: OperationId): Promise<Operation>;
    startDemo(request: StartDemoRequest): Promise<Operation>;
  };
  workspace: {
    /**
     * Open the directory at `path` (absolute, native form). Without a path the
     * shell shows the native directory picker; `null` means it was dismissed.
     * Replacing a workspace that has dirty documents rejects with `Conflict`
     * unless `options.force` is set.
     */
    open(path?: string, options?: { force?: boolean }): Promise<Workspace | null>;
    /** Rejects with `Conflict` while documents are dirty unless `options.force`. */
    close(workspaceId: WorkspaceId, options?: { force?: boolean }): Promise<Workspace>;
    /** The open workspace, for rehydration after a renderer reload. */
    current(): Promise<Workspace | null>;
    /**
     * One directory level. `ready` carries the page; `pending` names the
     * operation whose `OperationCompleted.result` will carry it.
     */
    list(request: ListDirectoryRequest): Promise<ListDirectoryResponse>;
    /**
     * The roots opened before, newest first, for the startup screen (T049).
     * Each entry says whether its root is still a directory and carries a
     * home-shortened `displayRoot`; `root` is what `open` and `forget` take.
     *
     * Never rejects because persistence is unavailable — the history is empty
     * instead, and `session.restore`'s notices say why. `limit` is capped by
     * the core; a snapshot, not a subscription.
     */
    recents(limit?: number): Promise<RecentWorkspace[]>;
    /**
     * Drop one root from the history. Idempotent, and narrow: the tabs and
     * unsaved-buffer snapshots stored under that root are left alone.
     */
    forget(root: string): Promise<void>;
  };
  /**
   * Documents (T030, Gate B). The core owns every buffer: `open` returns the
   * current text (the buffer, not the disk, when the path is already open),
   * `update` replaces it and must quote the `currentVersion` it was derived
   * from (`Conflict` otherwise), `save` writes atomically and refuses with
   * `Conflict` when the file changed or vanished on disk unless `force` is
   * set, `reload` replaces the buffer from disk, `status` re-checks the disk
   * and emits `DocumentExternalChangeDetected` on transitions.
   *
   * Callers must serialize their own `update` calls per document (await one
   * before sending the next), otherwise the second one is stale by construction.
   */
  documents: {
    open(request: OpenDocumentRequest): Promise<DocumentContent>;
    /** Rejects with `Conflict` for a dirty document unless `force` is set. */
    close(request: CloseDocumentRequest): Promise<Document>;
    update(request: UpdateDocumentRequest): Promise<Document>;
    save(request: SaveDocumentRequest): Promise<Document>;
    reload(request: ReloadDocumentRequest): Promise<DocumentContent>;
    /** One document, or every open document when `documentId` is omitted. */
    status(request?: DocumentStatusRequest): Promise<Document[]>;
    /** Every open document (no text), for rehydration after a renderer reload. */
    list(): Promise<Document[]>;
  };
  /**
   * Shell integration (T050). The only way content leaves the app: the core
   * validates the URL (http and https only, bounded length, no credentials)
   * and hands it to the system's default handler. Anything else rejects with
   * `InvalidInput`; a handler failure is `Io`.
   */
  shell: {
    openExternal(url: string): Promise<void>;
  };
  /**
   * Session persistence (T060). `restore` reopens the last workspace once per
   * process and returns the persisted tabs, preferences and layout; it never
   * rejects for persistence reasons (`persistence` and `notices` say what
   * happened). `candidates` compares every recovery snapshot of a workspace
   * with the disk; `recover` puts one back into the document's buffer (never
   * onto the disk) and returns the document like `documents.open`; `discard`
   * drops it; `update` stores what the UI owns.
   */
  session: {
    restore(): Promise<SessionRestoreResponse>;
    candidates(request: CandidatesRequest): Promise<RecoveryCandidate[]>;
    recover(request: RecoverSnapshotRequest): Promise<DocumentContent>;
    discard(request: DiscardSnapshotRequest): Promise<void>;
    update(request: UpdateSessionRequest): Promise<void>;
    /**
     * What the launcher offers to pick up again (T051): the workspace that is
     * not open and the tabs it had. `null` while a workspace is open, without
     * a database, or when the last workspace had no tabs — the recent list
     * already covers that case.
     */
    lastSession(): Promise<LastSession | null>;
  };
  /**
   * Terminal sessions (T090). `spawn` starts the user's login shell in the
   * workspace (`cwd` is workspace-relative, the root by default) and returns
   * the session; its output arrives as `TerminalOutput` events, batched and
   * bounded — a batch's `truncatedBytes` counts output that was dropped
   * because the shell wrote faster than it could be delivered, and must be
   * shown rather than hidden. `write` sends keystrokes verbatim, control
   * characters included. `kill` terminates the whole process tree and is
   * idempotent; `TerminalExited` follows exactly once, after the last output.
   * A session that has ended stays listed, with its exit status, until the
   * workspace closes.
   */
  terminal: {
    spawn(request: SpawnTerminalRequest): Promise<Terminal>;
    write(request: WriteTerminalRequest): Promise<void>;
    resize(request: ResizeTerminalRequest): Promise<Terminal>;
    /**
     * Label a session, or clear the label with a blank name. The label is core
     * state, not renderer state: `list` hands it back after a reload, so a
     * name outlives the window that set it.
     */
    rename(request: RenameTerminalRequest): Promise<Terminal>;
    /**
     * Give a session a colour for grouping, or clear it by omitting `color`.
     * Core state for the same reason the label is, and a closed set of hues
     * rather than a free colour: the palette has a light ramp as well as a
     * dark one (ADR-016).
     */
    recolor(request: RecolorTerminalRequest): Promise<Terminal>;
    kill(request: KillTerminalRequest): Promise<Terminal>;
    /** Every session, running and ended, in creation order. */
    list(): Promise<Terminal[]>;
    /**
     * Shells the user can pick (T095), the default first, followed by any
     * `custom` paths the settings added. A snapshot, not a subscription: a
     * shell installed afterwards appears on the next call.
     *
     * Discovery reports only what `$SHELL` and `/etc/shells` name. A custom
     * path that cannot be started comes back with `available: false` and a
     * reason rather than being dropped.
     */
    profiles(request?: ListProfilesRequest): Promise<TerminalProfile[]>;
  };
  /**
   * Workstreams (T200): the durable model of work. `create` starts a `DRAFT`;
   * `transition` is the only way the state moves and refuses anything the
   * state machine does not allow with `Conflict`, changing nothing. `update`
   * touches title and goal only — an absent field is left alone, a blank goal
   * clears it. `remove` deletes the workstream and its decisions; `ABANDONED`
   * is the reversible version. Every call needs the workspace to be open, and
   * every call needs the session database: without it they fail with `Io`
   * rather than keeping work that would vanish at exit.
   */
  workstreams: {
    list(request: ListWorkstreamsRequest): Promise<Workstream[]>;
    create(request: CreateWorkstreamRequest): Promise<Workstream>;
    update(request: UpdateWorkstreamRequest): Promise<Workstream>;
    transition(request: TransitionWorkstreamRequest): Promise<Workstream>;
    remove(request: RemoveWorkstreamRequest): Promise<void>;
  };
  /**
   * Decisions (T200): the questions that block a workstream, oldest first.
   *
   * `raise` on work that is `IN_PROGRESS` blocks it, and answering the last
   * open question resumes it — that rule lives in the core, so the UI cannot
   * forget to apply it. `resolve` needs an answer: one of the offered option
   * keys, or free text when the question offered none. `dismiss` records no
   * answer at all, because inventing one would be a lie in the run log.
   * Answering a decision twice is a `Conflict`.
   */
  decisions: {
    list(request: ListDecisionsRequest): Promise<Decision[]>;
    raise(request: RaiseDecisionRequest): Promise<Decision>;
    resolve(request: ResolveDecisionRequest): Promise<Decision>;
    dismiss(request: DismissDecisionRequest): Promise<Decision>;
  };
  /**
   * Agent sessions running on this machine (T220).
   *
   * Read-only, and machine-wide: these are sessions other terminals own, so
   * there is nothing here that starts, stops or answers one. `list` returns the
   * roster the core last scanned (scanning once if it never has); `refresh`
   * forces a rescan. The core also polls, and publishes `AgentSessionsChanged`
   * whenever the roster actually differs — subscribe to that rather than
   * calling `refresh` on a timer.
   */
  agents: {
    list(request?: ListAgentSessionsRequest): Promise<AgentRoster>;
    refresh(request?: ListAgentSessionsRequest): Promise<AgentRoster>;
  };
  subscribe<N extends EventName>(name: N, handler: (event: EventOf<N>) => void): Unsubscribe;
  subscribeAll(handler: (event: EventEnvelope) => void): Unsubscribe;
  /** Stop listening for events. Commands still work. */
  dispose(): void;
}

const defaultDiagnostic: Diagnostic = (message, context) => {
  console.warn(`[gritty protocol] ${message}`, context);
};

export function createCoreClient(
  transport: Transport,
  options: CoreClientOptions = {},
): CoreClient {
  const diagnostic = options.onDiagnostic ?? defaultDiagnostic;
  const clientVersion = options.clientVersion ?? PROTOCOL_VERSION;
  const hub = new EventHub(transport, CORE_EVENT_CHANNEL, diagnostic);

  let probe: Promise<HealthReport> | null = null;
  let refusal: CoreError | null = null;

  async function invokeRaw<T>(name: CommandName, args?: Record<string, unknown>): Promise<T> {
    try {
      return await transport.invoke<T>(COMMANDS[name], args);
    } catch (error: unknown) {
      throw toCoreError(error);
    }
  }

  function ensureCompatible(): Promise<HealthReport> {
    if (refusal) return Promise.reject(refusal);
    if (!probe) {
      probe = invokeRaw<HealthReport>('core.health')
        .then((report) => {
          assertCompatible(report.protocolVersion, clientVersion);
          return report;
        })
        .catch((error: unknown) => {
          probe = null;
          const coreError = toCoreError(error);
          if (coreError.code === 'ProtocolMismatch') refusal = coreError;
          throw coreError;
        });
    }
    return probe;
  }

  async function command<T>(name: CommandName, args?: Record<string, unknown>): Promise<T> {
    await ensureCompatible();
    return invokeRaw<T>(name, args);
  }

  return {
    health: () => ensureCompatible(),
    operations: {
      list: () => command<Operation[]>('operations.list'),
      cancel: (operationId) =>
        command<Operation>('operations.cancel', { request: { operationId } }),
      startDemo: (request) => command<Operation>('operations.startDemo', { request }),
    },
    workspace: {
      open: (path, options) => {
        const request: OpenWorkspaceRequest = {};
        if (path !== undefined) request.path = path;
        if (options?.force) request.force = true;
        return command<Workspace | null>('workspace.open', { request });
      },
      close: (workspaceId, options) => {
        const request: CloseWorkspaceRequest = { workspaceId };
        if (options?.force) request.force = true;
        return command<Workspace>('workspace.close', { request });
      },
      current: () => command<Workspace | null>('workspace.current'),
      list: (request) => command<ListDirectoryResponse>('workspace.list', { request }),
      recents: (limit) => {
        const request: RecentWorkspacesRequest = {};
        if (limit !== undefined) request.limit = limit;
        return command<RecentWorkspace[]>('workspace.recents', { request });
      },
      forget: (root) => {
        const request: ForgetWorkspaceRequest = { root };
        return command<void>('workspace.forget', { request });
      },
    },
    documents: {
      open: (request) => command<DocumentContent>('documents.open', { request }),
      close: (request) => command<Document>('documents.close', { request }),
      update: (request) => command<Document>('documents.update', { request }),
      save: (request) => command<Document>('documents.save', { request }),
      reload: (request) => command<DocumentContent>('documents.reload', { request }),
      status: (request = {}) => command<Document[]>('documents.status', { request }),
      list: () => command<Document[]>('documents.list'),
    },
    shell: {
      openExternal: (url) => {
        const request: OpenExternalRequest = { url };
        return command<void>('shell.openExternal', { request });
      },
    },
    session: {
      restore: () => command<SessionRestoreResponse>('session.restore'),
      candidates: (request) => command<RecoveryCandidate[]>('session.candidates', { request }),
      recover: (request) => command<DocumentContent>('session.recover', { request }),
      discard: (request) => command<void>('session.discard', { request }),
      update: (request) => command<void>('session.update', { request }),
      lastSession: () => command<LastSession | null>('session.lastSession'),
    },
    terminal: {
      spawn: (request) => command<Terminal>('terminal.spawn', { request }),
      write: (request) => command<void>('terminal.write', { request }),
      resize: (request) => command<Terminal>('terminal.resize', { request }),
      rename: (request) => command<Terminal>('terminal.rename', { request }),
      recolor: (request) => command<Terminal>('terminal.recolor', { request }),
      kill: (request) => command<Terminal>('terminal.kill', { request }),
      list: () => command<Terminal[]>('terminal.list'),
      profiles: (request = { custom: [] }) =>
        command<TerminalProfile[]>('terminal.profiles', { request }),
    },
    workstreams: {
      list: (request) => command<Workstream[]>('workstreams.list', { request }),
      create: (request) => command<Workstream>('workstreams.create', { request }),
      update: (request) => command<Workstream>('workstreams.update', { request }),
      transition: (request) => command<Workstream>('workstreams.transition', { request }),
      remove: (request) => command<void>('workstreams.remove', { request }),
    },
    decisions: {
      list: (request) => command<Decision[]>('decisions.list', { request }),
      raise: (request) => command<Decision>('decisions.raise', { request }),
      resolve: (request) => command<Decision>('decisions.resolve', { request }),
      dismiss: (request) => command<Decision>('decisions.dismiss', { request }),
    },
    agents: {
      list: (request = {}) => command<AgentRoster>('agents.list', { request }),
      refresh: (request = {}) => command<AgentRoster>('agents.refresh', { request }),
    },
    subscribe: (name, handler) => hub.subscribe(name, handler),
    subscribeAll: (handler) => hub.subscribeAll(handler),
    dispose: () => hub.dispose(),
  };
}
