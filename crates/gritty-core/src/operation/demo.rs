//! `operations.startDemo`: a fake long-running operation used by tests and by
//! the development panel to exercise progress, cancellation and failure.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::registry::{OperationContext, OperationRegistry, OperationResult, OperationSpec};
use super::{Operation, OperationKind};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::CorrelationId;

pub const DEMO_KIND: &str = "operations.demo";
pub const MAX_STEPS: u32 = 10_000;
pub const MAX_STEP_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StartDemoRequest {
    /// Number of steps, `1..=10000`.
    pub steps: u32,
    /// Sleep per step in milliseconds, `0..=10000`.
    pub step_ms: u64,
    /// Fail with an `Internal` error when this step is reached (`1..=steps`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fail_at_step: Option<u32>,
}

impl StartDemoRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !(1..=MAX_STEPS).contains(&self.steps) {
            return Err(CoreError::invalid_input(
                "steps",
                format!("must be between 1 and {MAX_STEPS}"),
            ));
        }
        if self.step_ms > MAX_STEP_MS {
            return Err(CoreError::invalid_input(
                "stepMs",
                format!("must be between 0 and {MAX_STEP_MS}"),
            ));
        }
        if let Some(step) = self.fail_at_step
            && !(1..=self.steps).contains(&step)
        {
            return Err(CoreError::invalid_input(
                "failAtStep",
                format!("must be between 1 and {}", self.steps),
            ));
        }
        Ok(())
    }
}

/// Validate and start the demo operation. Invalid input has no side effects.
pub fn start_demo(
    registry: &Arc<OperationRegistry>,
    request: StartDemoRequest,
    actor: Actor,
    correlation_id: CorrelationId,
) -> Result<Operation, CoreError> {
    request.validate()?;
    let spec = OperationSpec {
        kind: OperationKind::new(DEMO_KIND),
        actor,
        workspace_id: None,
        correlation_id,
    };
    Ok(registry.start(spec, move |ctx| run_demo(ctx, request)))
}

async fn run_demo(ctx: OperationContext, request: StartDemoRequest) -> OperationResult {
    let total = u64::from(request.steps);
    ctx.report(0, Some(total), Some("starting".to_owned()));
    for step in 1..=request.steps {
        tokio::select! {
            biased;
            () = ctx.cancelled() => return Err(CoreError::cancelled("demo operation cancelled")),
            () = tokio::time::sleep(Duration::from_millis(request.step_ms)) => {}
        }
        if request.fail_at_step == Some(step) {
            return Err(
                CoreError::internal(format!("demo operation failed at step {step}"))
                    .with_details(serde_json::json!({ "simulated": true, "step": step })),
            );
        }
        ctx.report(
            u64::from(step),
            Some(total),
            Some(format!("step {step} of {}", request.steps)),
        );
    }
    Ok(Some(serde_json::json!({ "steps": request.steps })))
}

#[cfg(test)]
mod tests {
    use tokio::runtime::Handle;

    use super::*;
    use crate::event::testing::RecordingSink;
    use crate::event::{CoreEvent, EventBus};
    use crate::operation::OperationStatus;
    use crate::protocol::error::CoreErrorCode;

    fn request(steps: u32, step_ms: u64) -> StartDemoRequest {
        StartDemoRequest {
            steps,
            step_ms,
            fail_at_step: None,
        }
    }

    #[test]
    fn validation_reports_the_offending_field() {
        let cases = [
            (request(0, 10), "steps"),
            (request(MAX_STEPS + 1, 10), "steps"),
            (request(3, MAX_STEP_MS + 1), "stepMs"),
            (
                StartDemoRequest {
                    fail_at_step: Some(4),
                    ..request(3, 1)
                },
                "failAtStep",
            ),
            (
                StartDemoRequest {
                    fail_at_step: Some(0),
                    ..request(3, 1)
                },
                "failAtStep",
            ),
        ];
        for (req, field) in cases {
            let error = req.validate().unwrap_err();
            assert_eq!(error.code, CoreErrorCode::InvalidInput);
            assert_eq!(error.details.unwrap()["field"], field, "{req:?}");
        }
        assert!(request(1, 0).validate().is_ok());
        assert!(
            StartDemoRequest {
                fail_at_step: Some(3),
                ..request(3, 0)
            }
            .validate()
            .is_ok()
        );
    }

    #[tokio::test]
    async fn invalid_request_creates_no_operation() {
        let (sink, _rx) = RecordingSink::with_channel();
        let registry = OperationRegistry::new(Handle::current(), EventBus::new(sink.clone()));
        let error =
            start_demo(&registry, request(0, 1), Actor::User, CorrelationId::new()).unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(registry.list().is_empty());
        assert!(sink.recorded().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn demo_runs_to_completion_with_progress() {
        let (sink, mut rx) = RecordingSink::with_channel();
        let registry = OperationRegistry::new(Handle::current(), EventBus::new(sink.clone()));
        let op = start_demo(
            &registry,
            request(3, 100),
            Actor::User,
            CorrelationId::new(),
        )
        .unwrap();
        assert_eq!(op.kind.as_str(), DEMO_KIND);
        let completed = loop {
            let envelope = rx.recv().await.unwrap();
            if let CoreEvent::OperationCompleted(payload) = envelope.event {
                break payload;
            }
        };
        assert_eq!(completed.result, Some(serde_json::json!({ "steps": 3 })));
        assert_eq!(completed.operation.progress.unwrap().current, 3);
        let progress_events = sink
            .recorded()
            .iter()
            .filter(|e| e.event.name() == "OperationProgress")
            .count();
        assert!((2..=4).contains(&progress_events), "{progress_events}");
    }

    #[tokio::test(start_paused = true)]
    async fn demo_can_fail_on_demand() {
        let (sink, mut rx) = RecordingSink::with_channel();
        let registry = OperationRegistry::new(Handle::current(), EventBus::new(sink));
        let req = StartDemoRequest {
            fail_at_step: Some(2),
            ..request(5, 10)
        };
        let op = start_demo(&registry, req, Actor::User, CorrelationId::new()).unwrap();
        let failed = loop {
            let envelope = rx.recv().await.unwrap();
            if let CoreEvent::OperationFailed(payload) = envelope.event {
                break payload;
            }
        };
        assert_eq!(failed.error.code, CoreErrorCode::Internal);
        assert_eq!(failed.error.details.unwrap()["step"], 2);
        assert_eq!(
            registry.get(&op.id).unwrap().status,
            OperationStatus::Failed
        );
    }
}
