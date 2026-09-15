import { COMMANDS, type CommandName } from './commands';
import { type CoreError, toCoreError } from './errors';
import { type Diagnostic, EventHub, type Unsubscribe } from './events';
import type { Transport } from './transport';
import {
  type AcquireOwnershipRequest,
  type AgentIntegration,
  type AgentIntegrationChange,
  type AgentIntegrationChangeRequest,
  type AgentIntegrationRequest,
  type AgentLinkChange,
  type AgentNotificationSettings,
  type AgentPermissionGrants,
  type AgentRoster,
  type AgentRun,
  type AgentRunEventLog,
  type AgentTimeline,
  type AgentTimelineRequest,
  type AnswerPermissionRequest,
  type AnswerRunPermissionRequest,
  type CancelRunRequest,
  type CandidatesRequest,
  type CheckOwnershipRequest,
  type CloseDocumentRequest,
  type CloseWorkspaceRequest,
  CORE_EVENT_CHANNEL,
  type CreateEntryRequest,
  type CreateWorkstreamRequest,
  type Decision,
  type DeleteEntriesRequest,
  type DiscardSnapshotRequest,
  type DismissDecisionRequest,
  type Document,
  type DocumentContent,
  type DocumentStatusRequest,
  type EventEnvelope,
  type EventName,
  type EventOf,
  type FileOperationResponse,
  type FileOperationResult,
  type FindFilesRequest,
  type FindFilesResponse,
  type ForgetWorkspaceRequest,
  type GitCommitRequest,
  type GitDiffRequest,
  type GitDiffResponse,
  type GitDiscardRequest,
  type GitLogRequest,
  type GitLogResponse,
  type GitShowRequest,
  type GitShowResponse,
  type GitStageRequest,
  type GitStatusRequest,
  type GitStatusResponse,
  type GitUnstageRequest,
  type GitWriteResponse,
  type HealthReport,
  type KillTerminalRequest,
  type LastSession,
  type LinkAgentRequest,
  type ListAgentLinksRequest,
  type ListAgentSessionsRequest,
  type ListDecisionsRequest,
  type ListDirectoryRequest,
  type ListDirectoryResponse,
  type ListOwnershipRequest,
  type ListProfilesRequest,
  type ListRunsRequest,
  type ListWorkstreamsRequest,
  type MoveEntriesRequest,
  type OpenDocumentRequest,
  type OpenExternalRequest,
  type OpenWorkspaceRequest,
  type Operation,
  type OperationId,
  type OwnershipAcquisition,
  type OwnershipCheck,
  type OwnershipConflict,
  type OwnershipMap,
  PROTOCOL_VERSION,
  type PromptRunRequest,
  type RaiseDecisionRequest,
  type RecentWorkspace,
  type RecentWorkspacesRequest,
  type RecolorTerminalRequest,
  type RecoverSnapshotRequest,
  type RecoveryCandidate,
  type ReleaseOwnershipRequest,
  type ReloadDocumentRequest,
  type RemoveWorkstreamRequest,
  type RenameEntryRequest,
  type RenameTerminalRequest,
  type ResetSettingsRequest,
  type ResizeTerminalRequest,
  type ResolveConflictRequest,
  type ResolveDecisionRequest,
  type RevealPathRequest,
  type RevokeGrantRequest,
  type RunEventsRequest,
  type SaveDocumentRequest,
  type SearchQueryRequest,
  type SessionRestoreResponse,
  type SetAgentNotificationsRequest,
  type SetSettingsRequest,
  type SettingsSchema,
  type SettingsSnapshot,
  type SpawnTerminalRequest,
  type StartDemoRequest,
  type StartFakeRunRequest,
  type StartRunRequest,
  type Terminal,
  type TerminalProfile,
  type TransitionWorkstreamRequest,
  type UndoRequest,
  type UnlinkAgentRequest,
  type UpdateDocumentRequest,
  type UpdateSessionRequest,
  type UpdateWorkstreamRequest,
  type Workspace,
  type WorkspaceId,
  type WorkspaceWatchStatus,
  type Workstream,
  type WorkstreamAgentLink,
  type WriteTerminalRequest,
} from './types';
import { assertCompatible } from './version';

export interface CoreClientOptions {
  /** Receives non-fatal observations; defaults to `console.warn`. */
  onDiagnostic?: Diagnostic;
  /** Override the protocol version the client claims (tests only). */
  clientVersion?: string;
  /**
   * How long a command waits for the event listener before going anyway
   * (tests only; the default is a safety valve a user never sees).
   */
  listenBudgetMs?: number;
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
     * The best-ranked paths in the open workspace for `query` — the quick-open
     * search (T130-C). `list` walks one level; this walks the tree and ranks.
     *
     * The query is a case-insensitive **subsequence**, not a substring: `epn`
     * finds `editor-pane.tsx`. Each match carries the spans that matched, so
     * the UI highlights without re-running the matcher — which is why this is
     * not a client-side rank over `search.query`, whose `SearchMatch` carries
     * one contiguous span and cannot express a subsequence hit at all.
     *
     * The same budget hand-over as `list`: `ready` carries the matches,
     * `pending` names the operation whose `OperationCompleted.result` will
     * carry the same `FoundFiles`. On a repository of any size `pending` is
     * the normal answer, not the exceptional one — a walk of 10,429 files was
     * measured at 94 ms against a 100 ms budget — so a caller that only
     * handles `ready` will look like it works on a fixture and not on a
     * real project.
     *
     * `ranges` are **char** indices, so slice by code point (`Array.from`)
     * rather than by `String.prototype.slice`, whose UTF-16 indices disagree
     * past the BMP. Cancel with `operations.cancel`; there is no
     * `findFiles.cancel`. `limit` outside `1..=200` is `InvalidInput`, not a
     * clamp. An empty `query` is not an error: it returns the first `limit`
     * paths in walk order, which is what the field shows before a key is
     * pressed.
     */
    findFiles(request: FindFilesRequest): Promise<FindFilesResponse>;
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
    /**
     * How well the open workspace is watched (WATCH-DEGRADED): every reason the
     * watch cannot cover the whole tree, empty when it can.
     * `WorkspaceWatchStatusChanged` announces each new reason once; this is the
     * same answer for a caller that mounted after it. `NotFound` when the
     * workspace is not the open one.
     */
    watchStatus(workspaceId: WorkspaceId): Promise<WorkspaceWatchStatus>;
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
    /**
     * Show a workspace entry in the system file manager (T042). The path is
     * workspace-relative and resolved in the core, so the WebView neither
     * names nor learns a native path. Only macOS is verified (ADR-002).
     */
    revealPath(request: RevealPathRequest): Promise<void>;
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
    /**
     * Attach an observed agent session to this piece of work (T223).
     *
     * The link is a row of its own, never a field on `Workstream`: a
     * workstream is durable, a discovered session is not, and one workstream
     * legitimately accumulates several sessions over its life. Linking a
     * session that is no longer in the roster is **allowed** — the roster is a
     * snapshot of now and a link records a fact about the past.
     *
     * It grants no control. A linked session is still one Gritty does not own,
     * and a `BLOCKED` session does not move its workstream.
     *
     * These return the workstream's links as they are afterwards rather than
     * emitting an event: `changed` is `false` for a duplicate link and for an
     * unlink of something that was not there, both of which are no-ops that
     * leave the state the caller asked for.
     */
    linkAgent(request: LinkAgentRequest): Promise<AgentLinkChange>;
    unlinkAgent(request: UnlinkAgentRequest): Promise<AgentLinkChange>;
    /**
     * Every link of the workspace, or of one workstream. A link whose session
     * has left the roster is still returned — the UI renders "linked, not
     * running" rather than dropping it or inventing a row.
     */
    agentLinks(request: ListAgentLinksRequest): Promise<WorkstreamAgentLink[]>;
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
   * Workspace ownership (T325): which workstream may write where.
   *
   * Logical leases, not operating-system locks — an agent would not respect
   * one of those, a directory or a glob cannot be locked at all, and a lock is
   * neither visible nor attributable to a piece of work. `acquire` answers
   * `GRANTED` or `CONFLICTED`, never both and never silently: a refusal
   * carries the conflict that explains it, which is the same row the attention
   * queue shows. Overlap is decided in the core, so no glob matcher is written
   * twice.
   *
   * This is **observed** ownership. Nothing here stops a write from happening;
   * it records who claimed what and refuses a second claim. Every mutation
   * publishes `OwnershipChanged` with the whole map.
   */
  ownership: {
    list(request: ListOwnershipRequest): Promise<OwnershipMap>;
    acquire(request: AcquireOwnershipRequest): Promise<OwnershipAcquisition>;
    release(request: ReleaseOwnershipRequest): Promise<void>;
    check(request: CheckOwnershipRequest): Promise<OwnershipCheck>;
    resolve(request: ResolveConflictRequest): Promise<OwnershipConflict>;
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
    /**
     * Start following a session's transcript (T230). The core publishes
     * `AgentTimelineAppended` deltas from here on — subscribe to that rather
     * than polling `timeline`. `NotFound` when the session has left the roster,
     * which it can do between a click and this call.
     */
    openTimeline(request: AgentTimelineRequest): Promise<AgentTimeline>;
    /** Stop following it. Idempotent. */
    closeTimeline(request: AgentTimelineRequest): Promise<void>;
    /** Re-read an open timeline — how a reloaded renderer resynchronises. */
    timeline(request: AgentTimelineRequest): Promise<AgentTimeline>;
    /**
     * Whether Gritty's hook is in the provider's own configuration (T221).
     *
     * This is the one corner of the product that writes into a file Gritty does
     * not own, and ADR-018 is what constrains it: nothing is written without an
     * explicit user action for that provider, entries are appended and never
     * replaced, the prior file is backed up, and a configuration that does not
     * parse is refused rather than repaired.
     *
     * Installing it does not make Gritty able to *do* anything to an agent. A
     * hook is a notification the agent emits; it buys exactly one value —
     * `BLOCKED`, the roster's answer to *which one needs me* — and discovery
     * keeps working with nothing installed, which stays the default.
     */
    integrationStatus(request: AgentIntegrationRequest): Promise<AgentIntegration>;
    /**
     * `apply: false` returns the change without making it, so the UI can show
     * the diff it is about to apply. `apply: true` performs exactly that.
     * `Conflict` when the configuration cannot be understood, carrying the
     * parse error; `PermissionDenied` when it or its backup cannot be written.
     */
    integrationInstall(request: AgentIntegrationChangeRequest): Promise<AgentIntegrationChange>;
    /** The same shape, in reverse. Removes only Gritty's own entries. */
    integrationUninstall(request: AgentIntegrationChangeRequest): Promise<AgentIntegrationChange>;
    /**
     * System notifications when a session stops and needs you (T223).
     *
     * **Off by default, every run**, and reading this never produces a
     * permission prompt — only `setNotifications({ enabled: true })` does, and
     * only because the user asked. A refusal comes back as a value with
     * `enabled: false` and `permission: 'DENIED'`, not as an error to dismiss,
     * and nothing retries.
     *
     * `supported: false` means the setting is **absent** rather than broken:
     * this build cannot raise a notification, so the UI offers no switch.
     *
     * The notification itself is raised from the Rust side, from the same
     * place that already observes the roster diff — edge-triggered on entering
     * `BLOCKED` or `WAITING`, throttled per session, capped globally, and
     * silent while the Gritty window has focus. The renderer neither decides
     * nor sends one.
     */
    notifications(): Promise<AgentNotificationSettings>;
    setNotifications(request: SetAgentNotificationsRequest): Promise<AgentNotificationSettings>;
    /**
     * Answer a permission prompt an agent is stopped at (T335, ADR-020).
     *
     * Returns the fresh roster, because answering removes the ask from the
     * card, the row, the banner, the header count and the status bar at once —
     * and because an ask that has already gone is not an error: the user may
     * have answered in the terminal first, and both paths are live at once.
     */
    answerPermission(request: AnswerPermissionRequest): Promise<AgentRoster>;
    /** Every standing grant, for the queue's standing-rules row. */
    permissionGrants(request?: ListAgentSessionsRequest): Promise<AgentPermissionGrants>;
    /**
     * Take a standing grant back, so the next matching ask returns to the user.
     * What Undo calls — a grant revocation, never a rollback: the tool the
     * grant allowed has already run.
     */
    revokeGrant(request: RevokeGrantRequest): Promise<AgentRoster>;
  };
  /**
   * Agent runs Gritty **owns** (T300, Gate D).
   *
   * The other half of `agents.*`, and deliberately not the same surface. There
   * a session is something another terminal started and Gritty can only watch,
   * which is why nothing there starts or stops one. Here Gritty started the
   * run, can cancel it, and is answerable for how it ended.
   *
   * Two events keep this current and they are not interchangeable:
   * `AgentRunChanged` carries a whole run on every state or activity change,
   * and `AgentRunEventAppended` carries a *batch* of new events. A consumer
   * that missed a change converges on the next one; a consumer that missed a
   * batch calls `events` to resynchronise, and compares `dropped` with what it
   * last saw to know a gap opened.
   */
  runs: {
    /**
     * Start a scripted run against a workstream in the open workspace.
     *
     * The deterministic path, and deliberately still its own command: a
     * fiction and a real process behind one name would make the run every
     * test depends on indistinguishable from the one that spends money. Use
     * {@link start} for a real provider. The workstream must exist
     * (`NotFound`) and the workspace must be open (`NotFound`); past
     * `MAX_CONCURRENT_RUNS` active runs the core refuses with `Conflict`,
     * naming the count. `script` picks one of the adapter's presets and
     * defaults to its first.
     */
    startFake(request: StartFakeRunRequest): Promise<AgentRun>;
    /**
     * Start a run against a real provider (T310-B).
     *
     * `provider: 'FAKE'` is rejected with `InvalidInput` — it has its own
     * command. `prompt` is the first thing to say to the agent and is queued
     * before the run exists, so it and every later prompt travel one path.
     *
     * The run appears in `STARTING` and reaches `RUNNING` only once the
     * provider's session announces itself, which is what that state is for.
     */
    start(request: StartRunRequest): Promise<AgentRun>;
    /**
     * Say something to a run that is already running (T310-B).
     *
     * **Nothing is optimistic.** Resolving means the text was queued, not that
     * the agent has seen it: the user's own words appear in the feed when
     * `PROMPT_STARTED` arrives, which is the run confirming them.
     *
     * Rejects with `Conflict` naming the state when the run is not accepting a
     * prompt (it has ended, is stopping, is `STARTING`, or is stopped on a
     * permission), when the run has no prompt channel at all, and when the
     * caller has already queued the most the core will hold. Never silently
     * dropped.
     */
    prompt(request: PromptRunRequest): Promise<AgentRun>;
    /**
     * Answer the permission ask a run is stopped at (T310-ANSWER): `ALLOW_ONCE`
     * lets that one call run exactly as it was asked, `DENY` tells the agent no
     * and the run carries on.
     *
     * The returned run no longer carries the ask, but may still be `BLOCKED`:
     * the run moves on when its agent acknowledges, and `AgentRunChanged`
     * reports that. An ask that has already gone — answered, withdrawn by a
     * stop, or ended with its run — is not an error; the run comes back
     * unchanged. Rejects with `NotFound` for an unknown run, `InvalidInput`
     * for a blank or oversized ask id, and `Conflict` when the answer could
     * not be delivered because the run's driver has stopped.
     */
    answerPermission(request: AnswerRunPermissionRequest): Promise<AgentRun>;
    /**
     * Ask a run to stop. Cooperative and idempotent, like `operations.cancel`:
     * an unknown id rejects with `NotFound`, a run that already ended returns
     * its snapshot unchanged, and anything else ends `CANCELLED`.
     *
     * The returned run may still be non-terminal with `cancellationRequested`
     * set — the flag moves at once and the state follows when the run notices.
     */
    cancel(request: CancelRunRequest): Promise<AgentRun>;
    /** Every run of one workspace, terminal ones included. For rehydration. */
    list(request: ListRunsRequest): Promise<AgentRun[]>;
    /**
     * One run's events, for rehydration after a renderer reload (T010's
     * convention). `dropped` reports what the core's ring evicted, so a client
     * draws the gap rather than splicing unrelated events together.
     */
    events(request: RunEventsRequest): Promise<AgentRunEventLog>;
  };
  /**
   * Reading the repository the open workspace sits in (T100).
   *
   * Both calls answer one of three ways, and the middle one is the reason they
   * are a tagged union rather than a bare payload:
   *
   * - `ready` — read within the budget, payload attached;
   * - `notARepository` — the workspace root is a directory and not a
   *   repository. A **value, not an error**: opening a folder that is not a
   *   repository is not a mistake, and an error here would put a red envelope
   *   in front of every non-repository workspace;
   * - `pending` — the read outlived its budget and continues as a cancellable
   *   `Operation`, whose `OperationCompleted.result` carries the payload. No
   *   new event family: the `Operation*` events already report it.
   *
   * **Four reads and four writes, on two different mechanisms.** The reads are
   * in-process `gix`; the writes shell out to the user's own `git`. ADR-029 is
   * that decision, and the short reason is that a filterless *write* would put
   * un-cleaned content in the index — plus signing and hooks, which an
   * in-process commit cannot do at all. A commit Gritty makes should be
   * indistinguishable from one the user made by hand.
   *
   * **Still nothing networked.** No fetch, pull, push or clone: that brings
   * credentials and transports, and is a further ADR (030) this surface
   * deliberately does not cover.
   */
  git: {
    /**
     * The state of the repository at the workspace root: the branch, what it is
     * in the middle of, and a page of changed paths.
     *
     * Ignored files are excluded unless `includeIgnored` asks for them — an
     * ignored `target/` is a million entries nobody asked for. Truncated
     * listings carry a `cursor`.
     */
    status(request: GitStatusRequest): Promise<GitStatusResponse>;
    /**
     * What changed between a revision (`HEAD` when absent) and the working
     * tree, with hunks.
     *
     * `path` restricts it to one file, which is the bounded way to ask for the
     * hunks of the file the user selected instead of paging every changed
     * file's hunks to reach one of them.
     */
    diff(request: GitDiffRequest): Promise<GitDiffResponse>;
    /**
     * A page of the history behind a revision (`HEAD` when absent), newest
     * first.
     *
     * `path` restricts the walk to commits that touched one file. The page
     * carries no total: counting every commit means walking the whole history
     * to render a number beside twenty rows, and reporting a count that was
     * never computed would be worse.
     */
    log(request: GitLogRequest): Promise<GitLogResponse>;
    /**
     * The content of one path at one revision — the *original* side of a diff.
     *
     * `git.diff` returns hunks with bounded context, and bounded context cannot
     * reconstruct a whole file, so a side-by-side view needs this. Binary and
     * oversized blobs come back with `noText` saying which, and never as an
     * error.
     */
    show(request: GitShowRequest): Promise<GitShowResponse>;

    /**
     * Move the named paths into the index.
     *
     * `paths` is explicit and must not be empty — **there is no "stage all"**.
     * Gritty ships a terminal, so the user's own `git` may be running beside
     * this one, and a wildcard write would sweep up whatever another process
     * changed a moment ago.
     *
     * The response carries the repository as it now stands, so the caller
     * converges on it rather than issuing a `git.status` behind it.
     */
    stage(request: GitStageRequest): Promise<GitWriteResponse>;
    /** Take the named paths back out of the index. Same rules as `stage`. */
    unstage(request: GitUnstageRequest): Promise<GitWriteResponse>;
    /**
     * Throw away the changes to the named paths — and keep them anyway.
     *
     * Tracked modifications are **stashed** before being reverted, and the
     * response's `stash` says where they went; untracked files go to the
     * **Trash**. Both are recoverable, which is why this is the one destructive
     * verb here that does not have to be feared.
     *
     * Rejects with `Conflict` and `details.dirtyDocuments` when an open buffer
     * under the selection is dirty, unless `force` is set.
     */
    discard(request: GitDiscardRequest): Promise<GitWriteResponse>;
    /**
     * Turn the staged band into a commit, using the user's identity, their
     * signing key and their hooks.
     *
     * The one write that can answer `pending`: a `pre-commit` hook can run a
     * whole test suite, so it continues as a cancellable `Operation`, and
     * cancelling it kills the hook's process group.
     *
     * A hook that refuses is a `Conflict` carrying the hook's own `stderr` in
     * `details`. The message is not lost — resend it.
     */
    commit(request: GitCommitRequest): Promise<GitWriteResponse>;
  };
  /**
   * Find-in-files over the open workspace. One call, and it is always an
   * `Operation`.
   *
   * There is no inline arm and no page: a listing is bounded and can answer as
   * one, a search is open-ended and answers as a stream. So `query` resolves
   * with the operation as soon as it exists, and the caller watches two things:
   *
   * - `SearchMatchesFound` — bounded, throttled batches of matches. Each
   *   carries `truncatedMatches`; a non-zero count means matches were found and
   *   genuinely never delivered, and the UI must say so rather than presenting
   *   a partial list as complete.
   * - the usual `Operation*` events, ending `COMPLETED` with a
   *   `SearchCompletion` result — counts only, never the matches, which
   *   already arrived.
   *
   * Cancel with `operations.cancel`; there is no `search.cancel`. Matches
   * already delivered stay delivered, and none arrive afterwards.
   *
   * A reload loses the batches that already streamed: the operation snapshot
   * carries status, not a result stream, so `operations.list()` rehydrates
   * *that a search is running* and nothing it found. Re-run to see them again.
   *
   * Nothing is indexed and nothing is written: `query` reads the tree as it is
   * now, and there is no replace here.
   */
  search: {
    query(request: SearchQueryRequest): Promise<Operation>;
  };
  /**
   * User settings (T120, ADR-031).
   *
   * Settings live in a `settings.json` the core owns, not in the session
   * database: ADR-004's SQLite rule governs what the application keeps about
   * *itself*, and user configuration is a different category — one you must be
   * able to repair in any editor when a bad value stops Gritty starting.
   *
   * `describe` is a build constant, so fetch it once. `get` and every write
   * answer with the same `SettingsSnapshot`, so a caller converges on the
   * response rather than issuing a `get` behind it.
   *
   * `set` is a **patch**: several keys in one call commit as one atomic file
   * write, and keys you do not name are untouched — including keys this build
   * does not declare, which are preserved rather than dropped. `reset` removes
   * keys so their defaults apply again, rather than writing today's default
   * into the file where a later build could never change it.
   *
   * Both refuse while `storage.writable` is false. The two cases that produce
   * that — an unparseable file, and no application data directory — are
   * reported with a reason meant for the user, and an unparseable file is
   * **never overwritten**: a typo must not cost you the file you were editing.
   */
  settings: {
    /** The schema the settings page renders its form from. */
    describe(): Promise<SettingsSchema>;
    /** Effective values, what is modified, and whether they can be saved. */
    get(): Promise<SettingsSnapshot>;
    /** Apply a patch. Refused whole if any key or value in it is invalid. */
    set(request: SetSettingsRequest): Promise<SettingsSnapshot>;
    /** Return keys to their defaults; omit `keys` to reset every known one. */
    reset(request: ResetSettingsRequest): Promise<SettingsSnapshot>;
    /**
     * Show `settings.json` in the system file manager.
     *
     * Its own command rather than `shell.revealPath`, which takes a
     * `workspaceId` and a `WorkspacePath` and resolves them against the
     * workspace root — the settings file is outside every workspace, so that
     * route is `PermissionDenied` by construction. The path here is
     * core-derived and never renderer-supplied.
     */
    reveal(): Promise<void>;
  };
  /**
   * Changing the tree (T042, ADR-028). The write half of `workspace.list`.
   *
   * Every command answers with a `FileOperationResult` whose `changed` names
   * the directories to re-list. That is the signal, not an event: the T070
   * watcher will report the same change a debounce later, but it is
   * best-effort — a backend that refuses to start still opens the workspace —
   * so convergence must not depend on it. `moved` says where entries ended up,
   * which is how a caller holding one by path follows a rename.
   *
   * `move` and `delete` can run long, so they answer `ready` or `pending` the
   * way `workspace.list` does; cancel with `operations.cancel`. Cancellation
   * stops before the next entry and **leaves what it already wrote**, and
   * nothing is rolled back. The core settles that part itself — journalled,
   * open documents moved — but a cancelled operation carries no result, so it
   * is announced by `OperationCancelled` alone and `cancelledAfter` is not set.
   * A move or delete that *fails* part-way does the same, and its error names
   * how far it got in `details.completed` and `details.requested`; a refusal
   * before any work has no `completed`. Either way, re-list what is loaded.
   *
   * `undo` and `redo` replay a bounded, in-memory journal that lives as long as
   * the workspace. A delete is never in it: on macOS the Trash is the recovery
   * and Gritty cannot restore from it (ADR-028).
   */
  files: {
    /** `path` is the whole path, so `a/b/c.ts` creates `a` and `a/b` too. */
    create(request: CreateEntryRequest): Promise<FileOperationResult>;
    /**
     * Rename in place. `newName` is one segment — a name containing `/` is
     * `InvalidInput` rather than quietly becoming a move.
     *
     * Open documents at or under `path` follow, keeping their buffers and dirty
     * state, and each emits `DocumentChanged { cause: MOVED }`.
     */
    rename(request: RenameEntryRequest): Promise<FileOperationResult>;
    /**
     * Move or copy entries into a directory. A **copy** onto a taken name
     * becomes `name copy.ext`; a **move** onto one is `Conflict`, and a move
     * into the directory an entry already lives in is `InvalidInput` — neither
     * silently renames, and neither overwrites.
     */
    move(request: MoveEntriesRequest): Promise<FileOperationResponse>;
    /**
     * Trash (default) or delete permanently. Rejects with `Conflict` and
     * `details.dirtyDocuments` while an open document under one of the paths
     * has unsaved changes, unless `force` is set. A move is never refused this
     * way, because the buffer goes with the file.
     */
    delete(request: DeleteEntriesRequest): Promise<FileOperationResponse>;
    /** Rejects with `Conflict` when there is nothing to undo, or when the
     *  world moved under the recorded inverse — never an overwrite. */
    undo(workspaceId: WorkspaceId): Promise<FileOperationResult>;
    redo(workspaceId: WorkspaceId): Promise<FileOperationResult>;
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
  const hub = new EventHub(transport, CORE_EVENT_CHANNEL, diagnostic, options.listenBudgetMs);
  // Open the event channel before anything asks for it. Registration is an IPC
  // round trip, and starting it here means it overlaps the compatibility probe
  // below instead of queueing behind whichever hook happens to subscribe first.
  hub.start();

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
    // The ordering guarantee (ADR-009): a command is not sent until the event
    // listener is registered, so the events it causes cannot be emitted into a
    // WebView that is not listening yet. `ready()` is bounded and fails open,
    // and by here it has almost always resolved already — registration started
    // at construction and the probe above is a full round trip.
    await hub.ready();
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
      findFiles: (request) => command<FindFilesResponse>('workspace.findFiles', { request }),
      recents: (limit) => {
        const request: RecentWorkspacesRequest = {};
        if (limit !== undefined) request.limit = limit;
        return command<RecentWorkspace[]>('workspace.recents', { request });
      },
      forget: (root) => {
        const request: ForgetWorkspaceRequest = { root };
        return command<void>('workspace.forget', { request });
      },
      watchStatus: (workspaceId) =>
        command<WorkspaceWatchStatus>('workspace.watchStatus', { request: { workspaceId } }),
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
      revealPath: (request) => command<void>('shell.revealPath', { request }),
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
      linkAgent: (request) => command<AgentLinkChange>('workstreams.linkAgent', { request }),
      unlinkAgent: (request) => command<AgentLinkChange>('workstreams.unlinkAgent', { request }),
      agentLinks: (request) =>
        command<WorkstreamAgentLink[]>('workstreams.agentLinks', { request }),
    },
    decisions: {
      list: (request) => command<Decision[]>('decisions.list', { request }),
      raise: (request) => command<Decision>('decisions.raise', { request }),
      resolve: (request) => command<Decision>('decisions.resolve', { request }),
      dismiss: (request) => command<Decision>('decisions.dismiss', { request }),
    },
    ownership: {
      list: (request) => command<OwnershipMap>('ownership.list', { request }),
      acquire: (request) => command<OwnershipAcquisition>('ownership.acquire', { request }),
      release: (request) => command<void>('ownership.release', { request }),
      check: (request) => command<OwnershipCheck>('ownership.check', { request }),
      resolve: (request) => command<OwnershipConflict>('ownership.resolve', { request }),
    },
    agents: {
      list: (request = {}) => command<AgentRoster>('agents.list', { request }),
      refresh: (request = {}) => command<AgentRoster>('agents.refresh', { request }),
      openTimeline: (request) => command<AgentTimeline>('agents.openTimeline', { request }),
      closeTimeline: (request) => command<void>('agents.closeTimeline', { request }),
      timeline: (request) => command<AgentTimeline>('agents.timeline', { request }),
      integrationStatus: (request) =>
        command<AgentIntegration>('agents.integrationStatus', { request }),
      integrationInstall: (request) =>
        command<AgentIntegrationChange>('agents.integrationInstall', { request }),
      integrationUninstall: (request) =>
        command<AgentIntegrationChange>('agents.integrationUninstall', { request }),
      notifications: () => command<AgentNotificationSettings>('agents.notifications'),
      setNotifications: (request) =>
        command<AgentNotificationSettings>('agents.setNotifications', { request }),
      answerPermission: (request) => command<AgentRoster>('agents.answerPermission', { request }),
      permissionGrants: (request = {}) =>
        command<AgentPermissionGrants>('agents.permissionGrants', { request }),
      revokeGrant: (request) => command<AgentRoster>('agents.revokeGrant', { request }),
    },
    runs: {
      startFake: (request) => command<AgentRun>('runs.startFake', { request }),
      start: (request) => command<AgentRun>('runs.start', { request }),
      prompt: (request) => command<AgentRun>('runs.prompt', { request }),
      answerPermission: (request) => command<AgentRun>('runs.answerPermission', { request }),
      cancel: (request) => command<AgentRun>('runs.cancel', { request }),
      list: (request) => command<AgentRun[]>('runs.list', { request }),
      events: (request) => command<AgentRunEventLog>('runs.events', { request }),
    },
    git: {
      status: (request) => command<GitStatusResponse>('git.status', { request }),
      diff: (request) => command<GitDiffResponse>('git.diff', { request }),
      log: (request) => command<GitLogResponse>('git.log', { request }),
      show: (request) => command<GitShowResponse>('git.show', { request }),
      stage: (request) => command<GitWriteResponse>('git.stage', { request }),
      unstage: (request) => command<GitWriteResponse>('git.unstage', { request }),
      discard: (request) => command<GitWriteResponse>('git.discard', { request }),
      commit: (request) => command<GitWriteResponse>('git.commit', { request }),
    },
    settings: {
      describe: () => command<SettingsSchema>('settings.describe'),
      get: () => command<SettingsSnapshot>('settings.get'),
      set: (request) => command<SettingsSnapshot>('settings.set', { request }),
      reset: (request) => command<SettingsSnapshot>('settings.reset', { request }),
      reveal: () => command<void>('settings.reveal'),
    },
    files: {
      create: (request) => command<FileOperationResult>('files.create', { request }),
      rename: (request) => command<FileOperationResult>('files.rename', { request }),
      move: (request) => command<FileOperationResponse>('files.move', { request }),
      delete: (request) => command<FileOperationResponse>('files.delete', { request }),
      undo: (workspaceId) => {
        const request: UndoRequest = { workspaceId };
        return command<FileOperationResult>('files.undo', { request });
      },
      redo: (workspaceId) => {
        const request: UndoRequest = { workspaceId };
        return command<FileOperationResult>('files.redo', { request });
      },
    },
    search: {
      query: (request) => command<Operation>('search.query', { request }),
    },
    subscribe: (name, handler) => hub.subscribe(name, handler),
    subscribeAll: (handler) => hub.subscribeAll(handler),
    dispose: () => hub.dispose(),
  };
}
