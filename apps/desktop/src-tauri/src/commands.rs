//! Tauri command adapters: `<domain>.<verb>` in TypeScript maps to
//! `<domain>_<snake_verb>` here. Adapters validate, delegate to the core and
//! map every failure into the `CoreError` envelope; no domain logic lives here.
//!
//! Every command body runs through [`guard`], which attaches a correlation id,
//! a tracing span, and turns a panic into an `Internal` error instead of a
//! promise that never settles.

use std::future::Future;

use gritty_core::health::ShellInfo;
use gritty_core::operation::demo::{StartDemoRequest, start_demo};
use gritty_core::operation::{CancelOperationRequest, Operation};
use gritty_core::protocol::actor::Actor;
use gritty_core::protocol::ids::CorrelationId;
use gritty_core::{CoreError, HealthReport};
use serde::de::DeserializeOwned;
use tauri::State;
use tracing::Instrument;

use crate::core::Core;

/// `core.health`
#[tauri::command]
pub async fn core_health(app: tauri::AppHandle) -> Result<HealthReport, CoreError> {
    guard("core.health", move |_| async move {
        let shell = ShellInfo {
            app_version: app.package_info().version.to_string(),
            runtime: "tauri".to_owned(),
            runtime_version: tauri::VERSION.to_owned(),
            webview_version: tauri::webview_version().ok(),
        };
        let report = gritty_core::health(shell);
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

/// Deserialize the single `request` argument; a malformed one is `InvalidInput`
/// rather than Tauri's opaque string rejection.
fn parse_request<T: DeserializeOwned>(request: serde_json::Value) -> Result<T, CoreError> {
    serde_json::from_value(request)
        .map_err(|error| CoreError::invalid_input("request", error.to_string()))
}

/// Run a command body on the async runtime with a correlation id and a span.
/// A panic inside the body surfaces as `Internal` carrying that correlation id.
async fn guard<T, F, Fut>(command: &'static str, body: F) -> Result<T, CoreError>
where
    T: Send + 'static,
    F: FnOnce(CorrelationId) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, CoreError>> + Send + 'static,
{
    let correlation = CorrelationId::new();
    let span = tracing::debug_span!("command", command, correlation_id = %correlation);
    let task_correlation = correlation.clone();
    let joined =
        tauri::async_runtime::spawn(async move { body(task_correlation).await }.instrument(span))
            .await;
    match joined {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            tracing::warn!(command, correlation_id = %correlation, code = %error.code, message = %error.message, "command failed");
            Err(error.or_correlation(&correlation))
        }
        Err(join) => {
            tracing::error!(command, correlation_id = %correlation, error = %join, "command panicked");
            Err(CoreError::internal(format!("command {command} panicked"))
                .with_correlation(correlation))
        }
    }
}
