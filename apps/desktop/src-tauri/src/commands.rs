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

use gritty_core::agent_session::{
    AgentRoster, AgentTimeline, AgentTimelineRequest, ListAgentSessionsRequest,
};
use gritty_core::document::{
    CloseDocumentRequest, Document, DocumentContent, DocumentStatusRequest, OpenDocumentRequest,
    ReloadDocumentRequest, SaveDocumentRequest, UpdateDocumentRequest,
};
use gritty_core::health::ShellInfo;
use gritty_core::operation::demo::{StartDemoRequest, start_demo};
use gritty_core::operation::{CancelOperationRequest, Operation};
use gritty_core::protocol::actor::Actor;
use gritty_core::protocol::ids::CorrelationId;
use gritty_core::session::{
    CandidatesRequest, DiscardSnapshotRequest, LastSession, RecoverSnapshotRequest,
    RecoveryCandidate, SessionRestoreResponse, UpdateSessionRequest,
};
use gritty_core::shell::OpenExternalRequest;
use gritty_core::terminal::wire::{
    KillTerminalRequest, ListProfilesRequest, RecolorTerminalRequest, RenameTerminalRequest,
    ResizeTerminalRequest, SpawnTerminalRequest, Terminal, TerminalProfile, WriteTerminalRequest,
};
use gritty_core::workspace::{
    CloseWorkspaceRequest, DEFAULT_LIST_BUDGET, ForgetWorkspaceRequest, ListDirectoryRequest,
    ListDirectoryResponse, OpenWorkspaceRequest, RecentWorkspace, RecentWorkspacesRequest,
    UnsavedPolicy, Workspace,
};
use gritty_core::workstream::{
    CreateWorkstreamRequest, Decision, DismissDecisionRequest, ListDecisionsRequest,
    ListWorkstreamsRequest, RaiseDecisionRequest, RemoveWorkstreamRequest, ResolveDecisionRequest,
    TransitionWorkstreamRequest, UpdateWorkstreamRequest, Workstream,
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
    guard("workspace.close", move |correlation| async move {
        let request: CloseWorkspaceRequest = parse_request(request)?;
        workspaces.close(
            &request.workspace_id,
            UnsavedPolicy::from_force(request.force),
            Actor::User,
            correlation,
        )
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
    guard("workstreams.remove", move |correlation| async move {
        let request: RemoveWorkstreamRequest = parse_request(request)?;
        workstreams.remove(request, Actor::User, correlation).await
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
