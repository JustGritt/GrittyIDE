//! Tauri command adapters: `<domain>.<verb>` in TypeScript maps to
//! `<domain>_<snake_verb>` here. Adapters validate, delegate to the core and
//! map every failure into the `CoreError` envelope; no domain logic lives here.
//!
//! Every command body runs through [`guard`], which attaches a correlation id,
//! a tracing span, and turns a panic into an `Internal` error instead of a
//! promise that never settles.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Instant;

use gritty_core::agent_run::{
    AgentRun, AgentRunEventLog, AgentRunProvider, AgentRunSpec, AnswerRunPermissionRequest,
    CancelRunRequest, ClaudeDriverOptions, ClaudeLaunch, FakeDriverOptions, ListRunsRequest,
    OwnedProviderSession, PromptRunRequest, RunEventsRequest, RunInbox, StartFakeRunRequest,
    StartRunRequest, claude_driver, fake_driver,
};
use gritty_core::agent_session::{
    AgentIntegration, AgentIntegrationChange, AgentIntegrationChangeRequest,
    AgentIntegrationRequest, AgentNotificationSettings, AgentPermissionGrants, AgentRoster,
    AgentTimeline, AgentTimelineRequest, AnswerPermissionRequest, ListAgentSessionsRequest,
    RevokeGrantRequest, SetAgentNotificationsRequest,
};
use gritty_core::document::{
    CloseDocumentRequest, Document, DocumentContent, DocumentStatusRequest, OpenDocumentRequest,
    ReloadDocumentRequest, SaveDocumentRequest, UpdateDocumentRequest,
};
use gritty_core::git::DEFAULT_GIT_BUDGET;
use gritty_core::git::wire::{
    GitCommitRequest, GitDiffRequest, GitDiffResponse, GitDiscardRequest, GitLogRequest,
    GitLogResponse, GitShowRequest, GitShowResponse, GitStageRequest, GitStatusRequest,
    GitStatusResponse, GitUnstageRequest, GitWriteResponse,
};
use gritty_core::health::ShellInfo;
use gritty_core::operation::demo::{StartDemoRequest, start_demo};
use gritty_core::operation::{CancelOperationRequest, Operation};
use gritty_core::ownership::{
    AcquireOwnershipRequest, CheckOwnershipRequest, ListOwnershipRequest, OwnershipAcquisition,
    OwnershipCheck, OwnershipConflict, OwnershipMap, ReleaseOwnershipRequest,
    ResolveConflictRequest,
};
use gritty_core::protocol::actor::Actor;
use gritty_core::protocol::ids::CorrelationId;
use gritty_core::search::wire::SearchQueryRequest;
use gritty_core::session::{
    CandidatesRequest, DiscardSnapshotRequest, LastSession, RecoverSnapshotRequest,
    RecoveryCandidate, SessionRestoreResponse, UpdateSessionRequest,
};
use gritty_core::settings::wire::{
    ResetSettingsRequest, SetSettingsRequest, SettingsSchema, SettingsSnapshot,
};
use gritty_core::shell::{OpenExternalRequest, RevealPathRequest};
use gritty_core::terminal::wire::{
    KillTerminalRequest, ListProfilesRequest, RecolorTerminalRequest, RenameTerminalRequest,
    ResizeTerminalRequest, SpawnTerminalRequest, Terminal, TerminalProfile, WriteTerminalRequest,
};
use gritty_core::workspace::files::{
    CreateEntryRequest, DEFAULT_FILES_BUDGET, DeleteEntriesRequest, Direction,
    FileOperationResponse, FileOperationResult, MoveEntriesRequest, RenameEntryRequest,
    UndoRequest,
};
use gritty_core::workspace::find::{DEFAULT_FIND_BUDGET, FindFilesRequest, FindFilesResponse};
use gritty_core::workspace::watch::{WatchStatusRequest, WorkspaceWatchStatus};
use gritty_core::workspace::{
    CloseWorkspaceRequest, DEFAULT_LIST_BUDGET, ForgetWorkspaceRequest, ListDirectoryRequest,
    ListDirectoryResponse, OpenWorkspaceRequest, RecentWorkspace, RecentWorkspacesRequest,
    UnsavedPolicy, Workspace,
};
use gritty_core::workstream::{
    AgentLinkChange, CreateWorkstreamRequest, Decision, DismissDecisionRequest, LinkAgentRequest,
    ListAgentLinksRequest, ListDecisionsRequest, ListWorkstreamsRequest, RaiseDecisionRequest,
    RemoveWorkstreamRequest, ResolveDecisionRequest, TransitionWorkstreamRequest,
    UnlinkAgentRequest, UpdateWorkstreamRequest, Workstream, WorkstreamAgentLink,
};
use gritty_core::{CoreError, HealthReport};
use serde::de::DeserializeOwned;
use tauri::State;
use tauri_plugin_dialog::DialogExt;
use tracing::Instrument;

use crate::core::Core;

/// `core.health`
#[tauri::command]
pub async fn core_health<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<HealthReport, CoreError> {
    guard("core.health", move |_| async move {
        let shell = ShellInfo {
            app_version: app.package_info().version.to_string(),
            runtime: "tauri".to_owned(),
            runtime_version: tauri::VERSION.to_owned(),
            webview_version: tauri::webview_version().ok(),
        };
        let report = gritty_core::health(shell);
        FIRST_HEALTH.call_once(|| crate::log_startup_milestone("first webview health"));
        tracing::info!(?report, "health requested by webview");
        Ok(report)
    })
    .await
}

/// `operations.list`
#[tauri::command]
pub async fn operations_list(core: State<'_, Core>) -> Result<Vec<Operation>, CoreError> {
    let registry = core.registry.clone();
    guard(
        "operations.list",
        move |_| async move { Ok(registry.list()) },
    )
    .await
}

/// `operations.cancel`
#[tauri::command]
pub async fn operations_cancel(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Operation, CoreError> {
    let registry = core.registry.clone();
    guard("operations.cancel", move |_| async move {
        let request: CancelOperationRequest = parse_request(request)?;
        registry.cancel(&request.operation_id)
    })
    .await
}

/// `operations.startDemo`
#[tauri::command]
pub async fn operations_start_demo(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Operation, CoreError> {
    let registry = core.registry.clone();
    guard("operations.startDemo", move |correlation| async move {
        let request: StartDemoRequest = parse_request(request)?;
        start_demo(&registry, request, Actor::User, correlation)
    })
    .await
}

/// `workspace.open`. Without a path the native directory picker is shown;
/// `null` means the user dismissed it.
#[tauri::command]
pub async fn workspace_open<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Option<Workspace>, CoreError> {
    let workspaces = core.workspaces.clone();
    guard("workspace.open", move |correlation| async move {
        let request: OpenWorkspaceRequest = parse_request(request)?;
        let path = match request.path {
            Some(path) => PathBuf::from(path),
            None => match pick_folder(&app).await? {
                Some(path) => path,
                None => {
                    tracing::info!("directory picker dismissed");
                    return Ok(None);
                }
            },
        };
        workspaces
            .open(
                &path,
                UnsavedPolicy::from_force(request.force),
                Actor::User,
                correlation,
            )
            .map(Some)
    })
    .await
}

/// `workspace.close`
#[tauri::command]
pub async fn workspace_close(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Workspace, CoreError> {
    let workspaces = core.workspaces.clone();
    let runs = core.runs.clone();
    guard("workspace.close", move |correlation| async move {
        let request: CloseWorkspaceRequest = parse_request(request)?;
        let closed = workspaces.close(
            &request.workspace_id,
            UnsavedPolicy::from_force(request.force),
            Actor::User,
            correlation,
        )?;
        // `INTERRUPTED`, not `CANCELLED`: nobody asked these runs to stop, we
        // simply stopped being able to watch them. Done after the close
        // succeeded — an unsaved buffer can refuse it, and interrupting a run
        // for a close that did not happen would be a lie.
        for interrupted in runs.interrupt_for_workspace(&request.workspace_id) {
            tracing::info!(
                run = %interrupted.id,
                workspace = %request.workspace_id,
                "interrupted a run whose workspace closed"
            );
        }
        Ok(closed)
    })
    .await
}

/// `workspace.current`
#[tauri::command]
pub async fn workspace_current(core: State<'_, Core>) -> Result<Option<Workspace>, CoreError> {
    let workspaces = core.workspaces.clone();
    guard("workspace.current", move |correlation| async move {
        Ok(workspaces.current(&correlation))
    })
    .await
}

/// `workspace.watchStatus`: how well the open workspace is watched
/// (WATCH-DEGRADED). The answer a renderer reads when it mounts after the
/// `WorkspaceWatchStatusChanged` that first said so.
#[tauri::command]
pub async fn workspace_watch_status(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<WorkspaceWatchStatus, CoreError> {
    let workspaces = core.workspaces.clone();
    guard("workspace.watchStatus", move |correlation| async move {
        let request: WatchStatusRequest = parse_request(request)?;
        workspaces.watch_status(request, &correlation)
    })
    .await
}

/// `workspace.list`
#[tauri::command]
pub async fn workspace_list(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<ListDirectoryResponse, CoreError> {
    let workspaces = core.workspaces.clone();
    guard("workspace.list", move |correlation| async move {
        let request: ListDirectoryRequest = parse_request(request)?;
        workspaces
            .list(request, Actor::User, correlation, DEFAULT_LIST_BUDGET)
            .await
    })
    .await
}

/// `workspace.findFiles`
///
/// The quick-open search (T130-C). Same budget hand-over as `workspace.list`:
/// the walk runs on the blocking pool, and a tree that outlives
/// `DEFAULT_FIND_BUDGET` comes back as an operation rather than making the
/// command wait. Cancel it with `operations.cancel`.
#[tauri::command]
pub async fn workspace_find_files(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<FindFilesResponse, CoreError> {
    let workspaces = core.workspaces.clone();
    guard("workspace.findFiles", move |correlation| async move {
        let request: FindFilesRequest = parse_request(request)?;
        workspaces
            .find_files(request, Actor::User, correlation, DEFAULT_FIND_BUDGET)
            .await
    })
    .await
}

/// `workspace.recents`: the workspace history behind the startup screen
/// (T049). Served by the session service, which owns the database handle.
#[tauri::command]
pub async fn workspace_recents(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<RecentWorkspace>, CoreError> {
    let session = core.session.clone();
    guard("workspace.recents", move |correlation| async move {
        let request: RecentWorkspacesRequest = parse_request(request)?;
        session.recents(request, correlation).await
    })
    .await
}

/// `workspace.forget`: drop one root from the history. Idempotent.
#[tauri::command]
pub async fn workspace_forget(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let session = core.session.clone();
    guard("workspace.forget", move |correlation| async move {
        let request: ForgetWorkspaceRequest = parse_request(request)?;
        session.forget(request, correlation).await
    })
    .await
}

/// `documents.open`
#[tauri::command]
pub async fn documents_open(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<DocumentContent, CoreError> {
    let documents = core.documents.clone();
    guard("documents.open", move |correlation| async move {
        let request: OpenDocumentRequest = parse_request(request)?;
        documents.open(request, Actor::User, correlation).await
    })
    .await
}

/// `documents.close`
#[tauri::command]
pub async fn documents_close(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Document, CoreError> {
    let documents = core.documents.clone();
    guard("documents.close", move |correlation| async move {
        let request: CloseDocumentRequest = parse_request(request)?;
        documents.close(request, Actor::User, correlation)
    })
    .await
}

/// `documents.update`
#[tauri::command]
pub async fn documents_update(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Document, CoreError> {
    let documents = core.documents.clone();
    guard("documents.update", move |correlation| async move {
        let request: UpdateDocumentRequest = parse_request(request)?;
        documents.update(request, Actor::User, correlation).await
    })
    .await
}

/// `documents.save`
#[tauri::command]
pub async fn documents_save(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Document, CoreError> {
    let documents = core.documents.clone();
    guard("documents.save", move |correlation| async move {
        let request: SaveDocumentRequest = parse_request(request)?;
        documents.save(request, Actor::User, correlation).await
    })
    .await
}

/// `documents.reload`
#[tauri::command]
pub async fn documents_reload(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<DocumentContent, CoreError> {
    let documents = core.documents.clone();
    guard("documents.reload", move |correlation| async move {
        let request: ReloadDocumentRequest = parse_request(request)?;
        documents.reload(request, Actor::User, correlation).await
    })
    .await
}

/// `documents.status`
#[tauri::command]
pub async fn documents_status(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<Document>, CoreError> {
    let documents = core.documents.clone();
    guard("documents.status", move |correlation| async move {
        let request: DocumentStatusRequest = parse_request(request)?;
        documents.status(request, Actor::User, correlation).await
    })
    .await
}

/// `documents.list`
#[tauri::command]
pub async fn documents_list(core: State<'_, Core>) -> Result<Vec<Document>, CoreError> {
    let documents = core.documents.clone();
    guard(
        "documents.list",
        move |_| async move { Ok(documents.list()) },
    )
    .await
}

/// `shell.openExternal`: the core validates the URL (http/https only) and the
/// system handler opens it; the WebView never navigates or spawns anything.
#[tauri::command]
pub async fn shell_open_external(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let shell = core.shell.clone();
    guard("shell.openExternal", move |correlation| async move {
        let request: OpenExternalRequest = parse_request(request)?;
        shell.open_external(request, Actor::User, correlation).await
    })
    .await
}

/// `shell.revealPath`
///
/// Show a workspace entry in the system file manager (T042). The path is
/// workspace-relative and resolved through the root in the core: the WebView
/// never names a native path, and never learns one.
#[tauri::command]
pub async fn shell_reveal_path(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let shell = core.shell.clone();
    guard("shell.revealPath", move |correlation| async move {
        let request: RevealPathRequest = parse_request(request)?;
        shell.reveal_path(request, correlation).await
    })
    .await
}

/// `session.restore`: the last workspace (reopened once per process), its
/// persisted tabs, the UI blobs and the persistence status. Never fails for
/// persistence reasons.
#[tauri::command]
pub async fn session_restore(core: State<'_, Core>) -> Result<SessionRestoreResponse, CoreError> {
    let session = core.session.clone();
    guard("session.restore", move |correlation| async move {
        let response = session.restore(correlation).await;
        tracing::info!(
            workspace = ?response.workspace.as_ref().map(|w| w.id.to_string()),
            tabs = response.tabs.len(),
            available = response.persistence.available,
            notices = response.notices.len(),
            "session restored"
        );
        Ok(response)
    })
    .await
}

/// `session.candidates`
#[tauri::command]
pub async fn session_candidates(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<RecoveryCandidate>, CoreError> {
    let session = core.session.clone();
    guard("session.candidates", move |correlation| async move {
        let request: CandidatesRequest = parse_request(request)?;
        session.candidates(request, correlation).await
    })
    .await
}

/// `session.recover`
#[tauri::command]
pub async fn session_recover(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<DocumentContent, CoreError> {
    let session = core.session.clone();
    guard("session.recover", move |correlation| async move {
        let request: RecoverSnapshotRequest = parse_request(request)?;
        session.recover(request, Actor::User, correlation).await
    })
    .await
}

/// `session.discard`
#[tauri::command]
pub async fn session_discard(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let session = core.session.clone();
    guard("session.discard", move |correlation| async move {
        let request: DiscardSnapshotRequest = parse_request(request)?;
        session.discard(request, correlation).await
    })
    .await
}

/// `session.update`
#[tauri::command]
pub async fn session_update(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let session = core.session.clone();
    guard("session.update", move |correlation| async move {
        let request: UpdateSessionRequest = parse_request(request)?;
        session.update(request, correlation).await
    })
    .await
}

/// `terminal.spawn`
#[tauri::command]
pub async fn terminal_spawn(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Terminal, CoreError> {
    let terminals = core.terminals.clone();
    guard("terminal.spawn", move |correlation| async move {
        let request: SpawnTerminalRequest = parse_request(request)?;
        terminals.spawn(request, Actor::User, correlation)
    })
    .await
}

/// `terminal.write`
///
/// The payload is what the user typed; it is never logged, only its size is.
#[tauri::command]
pub async fn terminal_write(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let terminals = core.terminals.clone();
    guard("terminal.write", move |_| async move {
        let request: WriteTerminalRequest = parse_request(request)?;
        terminals.write(request)
    })
    .await
}

/// `terminal.resize`
#[tauri::command]
pub async fn terminal_resize(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Terminal, CoreError> {
    let terminals = core.terminals.clone();
    guard("terminal.resize", move |_| async move {
        let request: ResizeTerminalRequest = parse_request(request)?;
        terminals.resize(request)
    })
    .await
}

/// `terminal.rename`
#[tauri::command]
pub async fn terminal_rename(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Terminal, CoreError> {
    let terminals = core.terminals.clone();
    guard("terminal.rename", move |_| async move {
        let request: RenameTerminalRequest = parse_request(request)?;
        terminals.rename(request)
    })
    .await
}

/// `terminal.recolor`
#[tauri::command]
pub async fn terminal_recolor(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Terminal, CoreError> {
    let terminals = core.terminals.clone();
    guard("terminal.recolor", move |_| async move {
        let request: RecolorTerminalRequest = parse_request(request)?;
        terminals.recolor(request)
    })
    .await
}

/// `terminal.kill`
#[tauri::command]
pub async fn terminal_kill(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Terminal, CoreError> {
    let terminals = core.terminals.clone();
    guard("terminal.kill", move |_| async move {
        let request: KillTerminalRequest = parse_request(request)?;
        terminals.kill(request)
    })
    .await
}

/// `terminal.profiles`
///
/// Filesystem inspection only, but still driven on the command runtime like
/// every other command: it stats a few dozen paths and the renderer must not
/// wait on a slow or unresponsive filesystem.
#[tauri::command]
pub async fn terminal_profiles(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<TerminalProfile>, CoreError> {
    let terminals = core.terminals.clone();
    guard("terminal.profiles", move |_| async move {
        let request: ListProfilesRequest = parse_request(request)?;
        terminals.profiles(request)
    })
    .await
}

/// `terminal.list`
#[tauri::command]
pub async fn terminal_list(core: State<'_, Core>) -> Result<Vec<Terminal>, CoreError> {
    let terminals = core.terminals.clone();
    guard(
        "terminal.list",
        move |_| async move { Ok(terminals.list()) },
    )
    .await
}

/// `session.lastSession`
#[tauri::command]
pub async fn session_last_session(core: State<'_, Core>) -> Result<Option<LastSession>, CoreError> {
    let session = core.session.clone();
    guard("session.lastSession", move |correlation| async move {
        session.last_session(correlation).await
    })
    .await
}

/// `agents.list`
///
/// The roster of agent sessions running on this machine (T220). Read-only:
/// there is no command that acts on one, because Gritty does not own them.
#[tauri::command]
pub async fn agents_list(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRoster, CoreError> {
    let agents = core.agents.clone();
    guard("agents.list", move |_| async move {
        let _request: ListAgentSessionsRequest = parse_request(request)?;
        Ok(agents.list().await)
    })
    .await
}

/// `agents.refresh`
#[tauri::command]
pub async fn agents_refresh(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRoster, CoreError> {
    let agents = core.agents.clone();
    guard("agents.refresh", move |_| async move {
        let _request: ListAgentSessionsRequest = parse_request(request)?;
        Ok(agents.refresh().await)
    })
    .await
}

/// `agents.openTimeline`
///
/// Starts following a session's transcript and returns what is known so far
/// (T230). Still read-only: following is a read, and nothing here can write to
/// the session or answer it.
#[tauri::command]
pub async fn agents_open_timeline(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentTimeline, CoreError> {
    let agents = core.agents.clone();
    guard("agents.openTimeline", move |_| async move {
        let request: AgentTimelineRequest = parse_request(request)?;
        agents.open_timeline(&request.session_id).await
    })
    .await
}

/// `agents.closeTimeline`
///
/// Idempotent: a renderer that reloaded cannot know which timelines it left
/// open, so closing one that is not open is not an error.
#[tauri::command]
pub async fn agents_close_timeline(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let agents = core.agents.clone();
    guard("agents.closeTimeline", move |_| async move {
        let request: AgentTimelineRequest = parse_request(request)?;
        agents.close_timeline(&request.session_id).await
    })
    .await
}

/// `agents.timeline`
///
/// Re-reads an open timeline without opening one — how the renderer
/// resynchronises after a reload or a missed event, rather than replaying.
#[tauri::command]
pub async fn agents_timeline(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentTimeline, CoreError> {
    let agents = core.agents.clone();
    guard("agents.timeline", move |_| async move {
        let request: AgentTimelineRequest = parse_request(request)?;
        agents.timeline(&request.session_id).await
    })
    .await
}

/// `agents.integrationStatus`
///
/// Reads a provider's configuration and says whether Gritty's hook is in it
/// (T221). Read-only; the two commands below are the only ones in the whole
/// `agents.*` surface that write anything at all, and only into the one file
/// ADR-018 allows.
#[tauri::command]
pub async fn agents_integration_status(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentIntegration, CoreError> {
    let agents = core.agents.clone();
    guard("agents.integrationStatus", move |_| async move {
        let request: AgentIntegrationRequest = parse_request(request)?;
        agents.integration_status(request.provider).await
    })
    .await
}

/// `agents.integrationInstall`
///
/// `apply: false` returns the change without making it — the file, the events,
/// the entry and the backup path — so the UI shows the diff it is about to
/// apply rather than a description of one (ADR-018 §3).
#[tauri::command]
pub async fn agents_integration_install(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentIntegrationChange, CoreError> {
    let agents = core.agents.clone();
    guard("agents.integrationInstall", move |_| async move {
        let request: AgentIntegrationChangeRequest = parse_request(request)?;
        agents
            .integration_install(request.provider, request.apply)
            .await
    })
    .await
}

/// `agents.integrationUninstall`
///
/// Removes exactly the entries whose command runs Gritty's binary, backing the
/// file up first and leaving every other tool's entries alone.
#[tauri::command]
pub async fn agents_integration_uninstall(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentIntegrationChange, CoreError> {
    let agents = core.agents.clone();
    guard("agents.integrationUninstall", move |_| async move {
        let request: AgentIntegrationChangeRequest = parse_request(request)?;
        agents
            .integration_uninstall(request.provider, request.apply)
            .await
    })
    .await
}

/// `agents.notifications`
///
/// What the settings surface shows (T223): whether this build can raise a
/// notification, whether the user has turned it on, and what the platform says
/// about permission. Reading it never produces a permission prompt.
#[tauri::command]
pub async fn agents_notifications(
    core: State<'_, Core>,
) -> Result<AgentNotificationSettings, CoreError> {
    let agents = core.agents.clone();
    guard("agents.notifications", move |_| async move {
        Ok(agents.notification_settings())
    })
    .await
}

/// `agents.setNotifications`
///
/// Turning it **on** is the one thing in Gritty that asks the platform for
/// notification permission — never application startup. A refusal comes back
/// as a value with the setting still off, not as an error to dismiss, and
/// nothing retries.
#[tauri::command]
pub async fn agents_set_notifications(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentNotificationSettings, CoreError> {
    let agents = core.agents.clone();
    guard("agents.setNotifications", move |_| async move {
        let request: SetAgentNotificationsRequest = parse_request(request)?;
        Ok(agents.set_notifications(request.enabled))
    })
    .await
}

/// `agents.answerPermission`
///
/// The one command in the agent surface that acts on a session rather than
/// reporting on it (T335, ADR-020), and it carries a decision the user just
/// took rather than one the core derived.
///
/// Answering an ask that has already gone is deliberately **not** an error: the
/// user may have answered in the terminal first, and ADR-020 says both paths
/// are live at once. The fresh roster comes back either way, which is what the
/// caller needs in order to stop drawing a prompt that is no longer there.
#[tauri::command]
pub async fn agents_answer_permission(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRoster, CoreError> {
    let agents = core.agents.clone();
    guard("agents.answerPermission", move |_| async move {
        let request: AnswerPermissionRequest = parse_request(request)?;
        Ok(agents
            .answer_permission(&request.ask_id, request.verdict)
            .await)
    })
    .await
}

/// `agents.permissionGrants`
#[tauri::command]
pub async fn agents_permission_grants(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentPermissionGrants, CoreError> {
    let agents = core.agents.clone();
    guard("agents.permissionGrants", move |_| async move {
        let _: ListAgentSessionsRequest = parse_request(request)?;
        Ok(agents.permission_grants())
    })
    .await
}

/// `agents.revokeGrant`
///
/// What Undo calls, and the standing-rules row. Not a rollback: the tool the
/// grant allowed has already run, and what is taken back is only the standing
/// part, so the next matching ask returns to the user.
#[tauri::command]
pub async fn agents_revoke_grant(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRoster, CoreError> {
    let agents = core.agents.clone();
    guard("agents.revokeGrant", move |_| async move {
        let request: RevokeGrantRequest = parse_request(request)?;
        Ok(agents.revoke_grant(&request.grant_id).await)
    })
    .await
}

/// `workstreams.list`
#[tauri::command]
pub async fn workstreams_list(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<Workstream>, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("workstreams.list", move |correlation| async move {
        let request: ListWorkstreamsRequest = parse_request(request)?;
        workstreams.list(request, correlation).await
    })
    .await
}

/// `workstreams.create`
#[tauri::command]
pub async fn workstreams_create(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Workstream, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("workstreams.create", move |correlation| async move {
        let request: CreateWorkstreamRequest = parse_request(request)?;
        workstreams.create(request, Actor::User, correlation).await
    })
    .await
}

/// `workstreams.update`
#[tauri::command]
pub async fn workstreams_update(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Workstream, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("workstreams.update", move |correlation| async move {
        let request: UpdateWorkstreamRequest = parse_request(request)?;
        workstreams.update(request, Actor::User, correlation).await
    })
    .await
}

/// `workstreams.transition`
#[tauri::command]
pub async fn workstreams_transition(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Workstream, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("workstreams.transition", move |correlation| async move {
        let request: TransitionWorkstreamRequest = parse_request(request)?;
        workstreams
            .transition(request, Actor::User, correlation)
            .await
    })
    .await
}

/// `workstreams.remove`
#[tauri::command]
pub async fn workstreams_remove(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let workstreams = core.workstreams.clone();
    let runs = core.runs.clone();
    guard("workstreams.remove", move |correlation| async move {
        let request: RemoveWorkstreamRequest = parse_request(request)?;
        let workstream_id = request.workstream_id.clone();
        workstreams
            .remove(request, Actor::User, correlation)
            .await?;
        // A run outlives the work it was doing otherwise: its workstream is
        // gone and nothing would ever end it. Cancelled after the removal
        // succeeded, so a refused removal leaves its runs alone.
        for cancelled in runs.cancel_for_workstream(&workstream_id) {
            tracing::info!(
                run = %cancelled.id,
                workstream = %workstream_id,
                "cancelled a run whose workstream was removed"
            );
        }
        Ok(())
    })
    .await
}

/// `workstreams.linkAgent`
///
/// Attaches an observed agent session to the work it is doing (T223). A
/// session that is no longer in the roster is allowed: a link records a fact
/// about the past. Returns the workstream's links as they are afterwards, read
/// back from the database, rather than emitting an event.
#[tauri::command]
pub async fn workstreams_link_agent(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentLinkChange, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("workstreams.linkAgent", move |correlation| async move {
        let request: LinkAgentRequest = parse_request(request)?;
        workstreams.link_agent(request, correlation).await
    })
    .await
}

/// `workstreams.unlinkAgent`. Removes the link and nothing else: the session
/// is not Gritty's to touch.
#[tauri::command]
pub async fn workstreams_unlink_agent(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentLinkChange, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("workstreams.unlinkAgent", move |correlation| async move {
        let request: UnlinkAgentRequest = parse_request(request)?;
        workstreams.unlink_agent(request, correlation).await
    })
    .await
}

/// `workstreams.agentLinks`. Without `workstreamId`, every link of the
/// workspace. A link whose session is not running is still returned; joining
/// it against the roster is the caller's job.
#[tauri::command]
pub async fn workstreams_agent_links(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<WorkstreamAgentLink>, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("workstreams.agentLinks", move |correlation| async move {
        let request: ListAgentLinksRequest = parse_request(request)?;
        workstreams.agent_links(request, correlation).await
    })
    .await
}

/// `decisions.list`
#[tauri::command]
pub async fn decisions_list(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<Decision>, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("decisions.list", move |correlation| async move {
        let request: ListDecisionsRequest = parse_request(request)?;
        workstreams.list_decisions(request, correlation).await
    })
    .await
}

/// `decisions.raise`
#[tauri::command]
pub async fn decisions_raise(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Decision, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("decisions.raise", move |correlation| async move {
        let request: RaiseDecisionRequest = parse_request(request)?;
        workstreams
            .raise_decision(request, Actor::User, correlation)
            .await
    })
    .await
}

/// `decisions.resolve`
#[tauri::command]
pub async fn decisions_resolve(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Decision, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("decisions.resolve", move |correlation| async move {
        let request: ResolveDecisionRequest = parse_request(request)?;
        workstreams
            .resolve_decision(request, Actor::User, correlation)
            .await
    })
    .await
}

/// `decisions.dismiss`
#[tauri::command]
pub async fn decisions_dismiss(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Decision, CoreError> {
    let workstreams = core.workstreams.clone();
    guard("decisions.dismiss", move |correlation| async move {
        let request: DismissDecisionRequest = parse_request(request)?;
        workstreams
            .dismiss_decision(request, Actor::User, correlation)
            .await
    })
    .await
}

/// `ownership.list`
#[tauri::command]
pub async fn ownership_list(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<OwnershipMap, CoreError> {
    let ownership = core.ownership.clone();
    guard("ownership.list", move |correlation| async move {
        let request: ListOwnershipRequest = parse_request(request)?;
        ownership.list(request, correlation).await
    })
    .await
}

/// `ownership.acquire`
#[tauri::command]
pub async fn ownership_acquire(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<OwnershipAcquisition, CoreError> {
    let ownership = core.ownership.clone();
    guard("ownership.acquire", move |correlation| async move {
        let request: AcquireOwnershipRequest = parse_request(request)?;
        ownership.acquire(request, Actor::User, correlation).await
    })
    .await
}

/// `ownership.release`
#[tauri::command]
pub async fn ownership_release(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<(), CoreError> {
    let ownership = core.ownership.clone();
    guard("ownership.release", move |correlation| async move {
        let request: ReleaseOwnershipRequest = parse_request(request)?;
        ownership.release(request, Actor::User, correlation).await
    })
    .await
}

/// `ownership.check`
#[tauri::command]
pub async fn ownership_check(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<OwnershipCheck, CoreError> {
    let ownership = core.ownership.clone();
    guard("ownership.check", move |correlation| async move {
        let request: CheckOwnershipRequest = parse_request(request)?;
        ownership.check(request, correlation).await
    })
    .await
}

/// `ownership.resolve`
#[tauri::command]
pub async fn ownership_resolve(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<OwnershipConflict, CoreError> {
    let ownership = core.ownership.clone();
    guard("ownership.resolve", move |correlation| async move {
        let request: ResolveConflictRequest = parse_request(request)?;
        ownership
            .resolve_conflict(request, Actor::User, correlation)
            .await
    })
    .await
}

/// `git.status`
///
/// A read, with the budget hand-over `workspace.list` established: a repository
/// that answers quickly answers inline, and one that does not becomes a
/// cancellable `Operation` the caller already knows how to watch. A workspace
/// that is not a repository is `NotARepository` — a value, not an error, because
/// opening a folder that is not a repository is not a mistake.
#[tauri::command]
pub async fn git_status(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitStatusResponse, CoreError> {
    let git = core.git.clone();
    guard("git.status", move |correlation| async move {
        let request: GitStatusRequest = parse_request(request)?;
        git.status(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `git.diff`
#[tauri::command]
pub async fn git_diff(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitDiffResponse, CoreError> {
    let git = core.git.clone();
    guard("git.diff", move |correlation| async move {
        let request: GitDiffRequest = parse_request(request)?;
        git.diff(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `git.log`
///
/// A page of history behind a revision, newest first, with the same budget
/// hand-over as the other reads. `path` narrows the walk to the commits that
/// changed one file.
#[tauri::command]
pub async fn git_log(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitLogResponse, CoreError> {
    let git = core.git.clone();
    guard("git.log", move |correlation| async move {
        let request: GitLogRequest = parse_request(request)?;
        git.log(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `git.show`
///
/// One path at one revision — the original side of a diff. A binary or
/// oversized blob answers with `noText` saying which, rather than an error; a
/// path that is absent at that revision is `NotFound`, because "did not exist
/// yet" and "was empty" are different answers.
#[tauri::command]
pub async fn git_show(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitShowResponse, CoreError> {
    let git = core.git.clone();
    guard("git.show", move |correlation| async move {
        let request: GitShowRequest = parse_request(request)?;
        git.show(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `git.stage`
///
/// The pathspec is explicit and never empty: there is no "stage everything" on
/// this wire, because Gritty ships a terminal and a wildcard write could sweep
/// up what another process changed a moment ago (ADR-029 decision 3).
#[tauri::command]
pub async fn git_stage(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitWriteResponse, CoreError> {
    let git = core.git.clone();
    guard("git.stage", move |correlation| async move {
        let request: GitStageRequest = parse_request(request)?;
        git.stage(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `git.unstage`
#[tauri::command]
pub async fn git_unstage(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitWriteResponse, CoreError> {
    let git = core.git.clone();
    guard("git.unstage", move |correlation| async move {
        let request: GitUnstageRequest = parse_request(request)?;
        git.unstage(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `git.discard`
///
/// The one destructive write, and the one that is nonetheless recoverable:
/// tracked work is stashed before it is reverted and untracked files go to the
/// Trash. A dirty open buffer under the selection is `Conflict` with
/// `details.dirtyDocuments` unless `force` is set (hard rule 7).
#[tauri::command]
pub async fn git_discard(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitWriteResponse, CoreError> {
    let git = core.git.clone();
    guard("git.discard", move |correlation| async move {
        let request: GitDiscardRequest = parse_request(request)?;
        git.discard(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `git.commit`
///
/// The write that can outlive its budget, because a `pre-commit` hook can run a
/// test suite. It then continues as a cancellable operation whose cancellation
/// kills the hook's process group (ADR-029 decision 6, hard rule 10).
#[tauri::command]
pub async fn git_commit(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<GitWriteResponse, CoreError> {
    let git = core.git.clone();
    guard("git.commit", move |correlation| async move {
        let request: GitCommitRequest = parse_request(request)?;
        git.commit(request, Actor::User, correlation, DEFAULT_GIT_BUDGET)
            .await
    })
    .await
}

/// `settings.describe`
///
/// The schema the settings page renders its form from. A build constant: it
/// reads no file, so it cannot fail and never touches the blocking pool.
#[tauri::command]
pub async fn settings_describe(core: State<'_, Core>) -> Result<SettingsSchema, CoreError> {
    let settings = core.settings.clone();
    guard("settings.describe", move |_correlation| async move {
        Ok(settings.describe())
    })
    .await
}

/// `settings.get`
#[tauri::command]
pub async fn settings_get(core: State<'_, Core>) -> Result<SettingsSnapshot, CoreError> {
    let settings = core.settings.clone();
    guard("settings.get", move |_correlation| async move {
        settings.get().await
    })
    .await
}

/// `settings.set`
///
/// A patch. Every key in it is validated before any of it is applied, so a
/// refusal leaves the file exactly as it was rather than half-written.
#[tauri::command]
pub async fn settings_set(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<SettingsSnapshot, CoreError> {
    let settings = core.settings.clone();
    guard("settings.set", move |correlation| async move {
        let request: SetSettingsRequest = parse_request(request)?;
        settings.set(request, correlation).await
    })
    .await
}

/// `settings.reset`
#[tauri::command]
pub async fn settings_reset(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<SettingsSnapshot, CoreError> {
    let settings = core.settings.clone();
    guard("settings.reset", move |correlation| async move {
        let request: ResetSettingsRequest = parse_request(request)?;
        settings.reset(request, correlation).await
    })
    .await
}

/// `settings.reveal`
///
/// Takes no request, deliberately: the path shown is the core's own, and a
/// renderer that could name one would be a renderer that can reveal anything.
#[tauri::command]
pub async fn settings_reveal(core: State<'_, Core>) -> Result<(), CoreError> {
    let settings = core.settings.clone();
    guard("settings.reveal", move |_correlation| async move {
        settings.reveal().await
    })
    .await
}

/// `search.query`
///
/// Unlike `workspace.list` and `git.status` there is no budget and no inline
/// arm: a listing is bounded and can answer as a page, a search is open-ended
/// and its output is a stream. So this always returns the `Operation`, and the
/// matches arrive as `SearchMatchesFound` batches until it ends. Cancelling is
/// `operations.cancel`; there is deliberately no `search.cancel`.
///
/// An empty query or a pattern that does not compile is `InvalidInput` with no
/// operation created and no event emitted, which is why the validation lives in
/// the core rather than here: it is a fact about the query, not about the
/// adapter.
#[tauri::command]
pub async fn search_query(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Operation, CoreError> {
    let searches = core.searches.clone();
    guard("search.query", move |correlation| async move {
        let request: SearchQueryRequest = parse_request(request)?;
        searches.query(request, Actor::User, correlation)
    })
    .await
}

/// `files.create`
///
/// The write half of `workspace.*` (T042, ADR-028). Like every adapter here it
/// parses, delegates and correlates; the path policy, the conflict rules and
/// the undo journal are all core-side.
#[tauri::command]
pub async fn files_create(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<FileOperationResult, CoreError> {
    let files = core.files.clone();
    guard("files.create", move |correlation| async move {
        let request: CreateEntryRequest = parse_request(request)?;
        files.create(request, correlation)
    })
    .await
}

/// `files.rename`
#[tauri::command]
pub async fn files_rename(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<FileOperationResult, CoreError> {
    let files = core.files.clone();
    guard("files.rename", move |correlation| async move {
        let request: RenameEntryRequest = parse_request(request)?;
        files.rename(request, Actor::User, correlation)
    })
    .await
}

/// `files.move`
///
/// Answers `ready`, or the operation a transfer that outlived its budget
/// continues as — the same hand-over `workspace.list` makes.
#[tauri::command]
pub async fn files_move(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<FileOperationResponse, CoreError> {
    let files = core.files.clone();
    guard("files.move", move |correlation| async move {
        let request: MoveEntriesRequest = parse_request(request)?;
        files
            .transfer(request, Actor::User, correlation, DEFAULT_FILES_BUDGET)
            .await
    })
    .await
}

/// `files.delete`
#[tauri::command]
pub async fn files_delete(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<FileOperationResponse, CoreError> {
    let files = core.files.clone();
    guard("files.delete", move |correlation| async move {
        let request: DeleteEntriesRequest = parse_request(request)?;
        files
            .delete(request, Actor::User, correlation, DEFAULT_FILES_BUDGET)
            .await
    })
    .await
}

/// `files.undo`
#[tauri::command]
pub async fn files_undo(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<FileOperationResult, CoreError> {
    let files = core.files.clone();
    guard("files.undo", move |correlation| async move {
        let request: UndoRequest = parse_request(request)?;
        files
            .step(request, Direction::Undo, Actor::User, correlation)
            .await
    })
    .await
}

/// `files.redo`
#[tauri::command]
pub async fn files_redo(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<FileOperationResult, CoreError> {
    let files = core.files.clone();
    guard("files.redo", move |correlation| async move {
        let request: UndoRequest = parse_request(request)?;
        files
            .step(request, Direction::Redo, Actor::User, correlation)
            .await
    })
    .await
}

/// `runs.startFake`
///
/// Where the three Gate D halves meet (T300-WIRE). Each was correct alone and
/// none could reach the others: `AgentRunManager::start` takes a *generic*
/// driver closure, `gritty-agent-fake` cannot name a `AgentRunEvent` without a
/// Cargo cycle, and this file had no `runs.*` in it at all.
///
/// Two checks happen **here**, before anything is created, because
/// `AgentRunSpec` says they are the adapter's:
///
/// 1. the script name resolves, or `InvalidInput` on `script`;
/// 2. the workstream exists in that workspace, or `NotFound`. The manager holds
///    no database and cannot answer this; `workstreams.list` refuses a
///    workspace that is not open, so one call settles both halves of the
///    request rather than trusting either.
///
/// The two API shapes are reconciled here rather than by changing either side:
/// T300-CMD recorded `start_fake(request, correlation)` and T300-CORE shipped
/// `start(spec, drive)`, so this builds the spec and supplies the driver.
#[tauri::command]
pub async fn runs_start_fake(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRun, CoreError> {
    let runs = core.runs.clone();
    let workstreams = core.workstreams.clone();
    guard("runs.startFake", move |correlation| async move {
        let request: StartFakeRunRequest = parse_request(request)?;
        request.validate()?;
        let script = fake_driver::resolve_script(request.script.as_deref())?;

        let known = workstreams
            .list(
                ListWorkstreamsRequest {
                    workspace_id: request.workspace_id.clone(),
                },
                correlation.clone(),
            )
            .await?;
        if !known
            .iter()
            .any(|workstream| workstream.id == request.workstream_id)
        {
            return Err(CoreError::not_found(format!(
                "no workstream {} in this workspace",
                request.workstream_id
            )));
        }

        let spec = AgentRunSpec {
            workspace_id: request.workspace_id,
            workstream_id: request.workstream_id,
            provider: AgentRunProvider::Fake,
            actor: Actor::User,
            correlation_id: correlation,
        };
        // Paced for a human, not for a test: the fake is instantaneous, and a
        // run that reaches FINISHED before the panel paints shows nothing.
        runs.start(spec, fake_driver::drive(script, FakeDriverOptions::demo()))
    })
    .await
}

/// `runs.start`
///
/// The real one (T310-B). `runs.startFake` is untouched beside it, because a
/// fiction and a process behind one command name would make the deterministic
/// path every other test depends on indistinguishable from the one that spends
/// money — so `StartRunRequest::validate` refuses `FAKE` outright.
///
/// The same two checks `runs.startFake` makes here rather than in the manager,
/// and for the same reason — the manager holds no database — plus one this
/// command needs and that one does not: the workspace's **root**, which is the
/// child's working directory.
///
/// Gritty mints the provider session id. That is what lets §8 exclude its own
/// child from the observed roster without parsing anything, and it is why the
/// id is handed to `start_with_inbox` rather than discovered afterwards.
#[tauri::command]
pub async fn runs_start(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRun, CoreError> {
    let runs = core.runs.clone();
    let workstreams = core.workstreams.clone();
    let workspaces = core.workspaces.clone();
    let binary = core.claude_binary.clone();
    guard("runs.start", move |correlation| async move {
        let request: StartRunRequest = parse_request(request)?;
        request.validate()?;

        let known = workstreams
            .list(
                ListWorkstreamsRequest {
                    workspace_id: request.workspace_id.clone(),
                },
                correlation.clone(),
            )
            .await?;
        if !known
            .iter()
            .any(|workstream| workstream.id == request.workstream_id)
        {
            return Err(CoreError::not_found(format!(
                "no workstream {} in this workspace",
                request.workstream_id
            )));
        }

        let Some((open_id, root)) = workspaces.current_open() else {
            return Err(CoreError::not_found("no workspace is open"));
        };
        if open_id != request.workspace_id {
            return Err(CoreError::not_found(format!(
                "workspace {} is not open",
                request.workspace_id
            )));
        }

        // Minted here, so Gritty knows exactly which observed sessions are its
        // own children. Not the `AgentRunId`: welding Gritty's identifier space
        // to a provider's would break the moment `T310-RESUME` has to adopt an
        // id the provider already owns.
        let session_id = uuid::Uuid::now_v7().hyphenated().to_string();
        let mut launch = ClaudeLaunch::new(&session_id, root.path())
            .map_err(|error| CoreError::invalid_input("provider", error.to_string()))?;
        if let Some(binary) = binary {
            launch = launch.with_program(binary);
        }

        let (inbox, receiver) = RunInbox::pair();
        if let Some(prompt) = request.prompt {
            // Queued before the run exists, so the first prompt and every later
            // one travel the same path.
            inbox.seed(prompt);
        }

        let spec = AgentRunSpec {
            workspace_id: request.workspace_id,
            workstream_id: request.workstream_id,
            provider: request.provider,
            actor: Actor::User,
            correlation_id: correlation,
        };
        runs.start_with_inbox(
            spec,
            inbox,
            OwnedProviderSession {
                session_id,
                pid: None,
            },
            claude_driver::drive(launch, receiver, ClaudeDriverOptions::default()),
        )
    })
    .await
}

/// `runs.prompt`
///
/// Say something to a run that is already running. Refuses rather than queues
/// at every edge, and every refusal names what is in the way: the state, the
/// provider, or the bound.
///
/// **Nothing is optimistic.** An `Ok` here means the text was queued, not that
/// the agent has seen it — `PROMPT_STARTED` is the run's own confirmation and
/// it arrives later, when the CLI replays the message back.
#[tauri::command]
pub async fn runs_prompt(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRun, CoreError> {
    let runs = core.runs.clone();
    guard("runs.prompt", move |_| async move {
        let request: PromptRunRequest = parse_request(request)?;
        request.validate()?;
        runs.prompt(&request.run_id, request.text, Actor::User)
    })
    .await
}

/// `runs.answerPermission`
///
/// The user's verdict on the permission ask a run is stopped at (T310-ANSWER).
/// The run leaves `BLOCKED` when its child acknowledges, not on this call's
/// return; an ask that has already gone comes back as the run, unchanged, and
/// is not an error.
#[tauri::command]
pub async fn runs_answer_permission(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRun, CoreError> {
    let runs = core.runs.clone();
    guard("runs.answerPermission", move |_| async move {
        let request: AnswerRunPermissionRequest = parse_request(request)?;
        request.validate()?;
        runs.answer_permission(
            &request.run_id,
            &request.ask_id,
            request.verdict,
            Actor::User,
        )
    })
    .await
}

/// `runs.cancel`
///
/// Cooperative and idempotent, like `operations.cancel`: an unknown id is
/// `NotFound`, an already-ended run comes back unchanged.
#[tauri::command]
pub async fn runs_cancel(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRun, CoreError> {
    let runs = core.runs.clone();
    guard("runs.cancel", move |_| async move {
        let request: CancelRunRequest = parse_request(request)?;
        runs.cancel(&request.run_id, Actor::User)
    })
    .await
}

/// `runs.list`
#[tauri::command]
pub async fn runs_list(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<Vec<AgentRun>, CoreError> {
    let runs = core.runs.clone();
    guard("runs.list", move |_| async move {
        let request: ListRunsRequest = parse_request(request)?;
        Ok(runs.list(&request.workspace_id))
    })
    .await
}

/// `runs.events`
///
/// The bounded ring, replayed. What a client that missed a batch — or attached
/// late — reads to catch up.
#[tauri::command]
pub async fn runs_events(
    core: State<'_, Core>,
    request: serde_json::Value,
) -> Result<AgentRunEventLog, CoreError> {
    let runs = core.runs.clone();
    guard("runs.events", move |_| async move {
        let request: RunEventsRequest = parse_request(request)?;
        runs.events(&request.run_id)
    })
    .await
}

/// Show the native directory picker from the Rust side. The plugin runs the
/// dialog on the main thread and calls back from a helper thread, so awaiting
/// it here never blocks the event loop or the command runtime.
async fn pick_folder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Option<PathBuf>, CoreError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Open workspace")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    let folder = rx
        .await
        .map_err(|_| CoreError::internal("directory picker closed without a result"))?;
    match folder {
        None => Ok(None),
        Some(location) => location.into_path().map(Some).map_err(|error| {
            CoreError::invalid_input(
                "path",
                format!("picker returned an unsupported location: {error}"),
            )
        }),
    }
}

/// Deserialize the single `request` argument; a malformed one is `InvalidInput`
/// rather than Tauri's opaque string rejection.
fn parse_request<T: DeserializeOwned>(request: serde_json::Value) -> Result<T, CoreError> {
    serde_json::from_value(request)
        .map_err(|error| CoreError::invalid_input("request", error.to_string()))
}

/// Run a command body on the async runtime with a correlation id and a span.
/// A panic inside the body surfaces as `Internal` carrying that correlation id.
/// Marks the first `core.health` call, which is the WebView's first round trip
/// through the protocol and therefore the closest proxy for "the shell is
/// usable" (§C, cold and warm launch).
static FIRST_HEALTH: Once = Once::new();

async fn guard<T, F, Fut>(command: &'static str, body: F) -> Result<T, CoreError>
where
    T: Send + 'static,
    F: FnOnce(CorrelationId) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, CoreError>> + Send + 'static,
{
    let started = Instant::now();
    let correlation = CorrelationId::new();
    let span = tracing::debug_span!("command", command, correlation_id = %correlation);
    let task_correlation = correlation.clone();
    let joined =
        tauri::async_runtime::spawn(async move { body(task_correlation).await }.instrument(span))
            .await;
    let elapsed_us = started.elapsed().as_micros() as u64;
    match joined {
        Ok(Ok(value)) => {
            tracing::debug!(
                command,
                correlation_id = %correlation,
                elapsed_us,
                "command completed"
            );
            Ok(value)
        }
        Ok(Err(error)) => {
            tracing::warn!(command, correlation_id = %correlation, elapsed_us, code = %error.code, message = %error.message, "command failed");
            Err(error.or_correlation(&correlation))
        }
        Err(join) => {
            tracing::error!(command, correlation_id = %correlation, elapsed_us, error = %join, "command panicked");
            Err(CoreError::internal(format!("command {command} panicked"))
                .with_correlation(correlation))
        }
    }
}

#[cfg(test)]
mod tests {
    use gritty_core::CoreErrorCode;

    use super::*;

    #[test]
    fn guard_reports_how_long_a_command_took() {
        let (result, log) = crate::log_capture::capture(|| {
            tauri::async_runtime::block_on(guard("test.timed", |_| async { Ok(7u8) }))
        });
        assert_eq!(result.expect("command"), 7);
        let line = crate::log_capture::line_with(&log, "command=\"test.timed\"");
        assert!(line.contains("command completed"), "{line}");
        assert!(line.contains("elapsed_us="), "{line}");
    }

    #[test]
    fn a_failing_command_still_reports_how_long_it_took() {
        let (result, log) = crate::log_capture::capture(|| {
            tauri::async_runtime::block_on(guard("test.slow-failure", |_| async {
                Err::<(), _>(CoreError::not_found("nothing"))
            }))
        });
        assert!(result.is_err());
        let line = crate::log_capture::line_with(&log, "command=\"test.slow-failure\"");
        assert!(line.contains("command failed"), "{line}");
        assert!(line.contains("elapsed_us="), "{line}");
    }

    #[test]
    fn guard_turns_a_panic_into_an_internal_error_with_correlation() {
        let result: Result<(), CoreError> =
            tauri::async_runtime::block_on(guard("test.panic", |_| async {
                panic!("adapter bug");
            }));
        let error = result.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::Internal);
        assert!(error.message.contains("test.panic"));
        assert!(error.correlation_id.is_some());
    }

    #[test]
    fn guard_attaches_the_correlation_id_to_core_errors() {
        let result: Result<(), CoreError> =
            tauri::async_runtime::block_on(guard("test.fail", |_| async {
                Err(CoreError::not_found("nothing"))
            }));
        let error = result.unwrap_err();
        assert_eq!(error.code, CoreErrorCode::NotFound);
        assert!(error.correlation_id.is_some());
    }

    #[test]
    fn guard_hands_the_same_correlation_id_to_the_body() {
        let result = tauri::async_runtime::block_on(guard("test.ok", |correlation| async move {
            Err::<(), _>(CoreError::internal("x").with_correlation(correlation))
        }));
        let error = result.unwrap_err();
        assert!(error.correlation_id.unwrap().is_well_formed());
    }

    #[test]
    fn malformed_request_is_invalid_input() {
        let error =
            parse_request::<StartDemoRequest>(serde_json::json!({ "steps": "x" })).unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert_eq!(error.details.unwrap()["field"], "request");
    }

    #[test]
    fn traversal_in_a_list_request_is_rejected_while_parsing() {
        let error = parse_request::<ListDirectoryRequest>(
            serde_json::json!({ "workspaceId": "ws", "path": "../secrets" }),
        )
        .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(error.message.contains("'..'"), "{error}");
    }
}
