//! Drives the real command adapters on Tauri's mock runtime: the same wiring
//! `run()` uses, invoked the way the WebView invokes it, with the event channel
//! observed from a Rust listener. This is the automated half of the T010
//! acceptance flow (the other half is a human in front of the window).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gritty_core::{CORE_EVENT_CHANNEL, PROTOCOL_VERSION};
use gritty_desktop_lib::{attach_core, configure};
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{Listener, WebviewWindow, WebviewWindowBuilder};

struct Harness {
    _app: tauri::App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
    events: Arc<Mutex<Vec<Value>>>,
}

impl Harness {
    fn new() -> Self {
        let app = configure(mock_builder())
            .build(mock_context(noop_assets()))
            .expect("build app");
        // The mock runtime never runs `setup`; do what it would do.
        attach_core(app.handle());
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("create webview");
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        app.listen_any(CORE_EVENT_CHANNEL, move |event| {
            let envelope: Value =
                serde_json::from_str(event.payload()).expect("event payload is JSON");
            sink.lock().unwrap().push(envelope);
        });
        Self {
            _app: app,
            webview,
            events,
        }
    }

    /// Invoke a command exactly as the WebView would; `Err` is the serialized error envelope.
    fn invoke(&self, cmd: &str, body: Value) -> Result<Value, Value> {
        get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().expect("response is JSON"))
    }

    fn operation(&self, id: &str) -> Value {
        let list = self.invoke("operations_list", json!({})).expect("list");
        list.as_array()
            .unwrap()
            .iter()
            .find(|op| op["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("operation {id} missing from {list}"))
    }

    fn wait_until(&self, id: &str, timeout: Duration, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let op = self.operation(id);
            if predicate(&op) {
                return op;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {id}: {op}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn event_names(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_owned())
            .collect()
    }
}

#[test]
fn health_reports_the_protocol_version_through_the_command_adapter() {
    let h = Harness::new();
    let report = h.invoke("core_health", json!({})).expect("health");
    assert_eq!(report["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(report["coreVersion"], gritty_core::VERSION);
    assert_eq!(report["shell"]["runtime"], "tauri");
    assert_eq!(report["shell"]["runtimeVersion"], tauri::VERSION);
}

#[test]
fn unknown_operation_rejects_with_a_not_found_envelope() {
    let h = Harness::new();
    let error = h
        .invoke(
            "operations_cancel",
            json!({ "request": { "operationId": "does-not-exist" } }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "NotFound");
    assert_eq!(error["retryable"], false);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("does-not-exist")
    );
    assert!(error["correlationId"].is_string(), "{error}");
}

#[test]
fn invalid_and_malformed_requests_are_invalid_input_without_side_effects() {
    let h = Harness::new();
    let error = h
        .invoke(
            "operations_start_demo",
            json!({ "request": { "steps": 0, "stepMs": 1 } }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "steps");

    let error = h
        .invoke(
            "operations_start_demo",
            json!({ "request": { "steps": "three", "stepMs": 1 } }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "request");

    let list = h.invoke("operations_list", json!({})).expect("list");
    assert_eq!(list, json!([]));
    assert!(h.event_names().is_empty());
}

#[test]
fn unknown_command_is_rejected_rather_than_hanging() {
    let h = Harness::new();
    let error = h.invoke("operations_missing", json!({})).unwrap_err();
    assert!(error.as_str().unwrap().contains("not found"), "{error}");
}

#[test]
fn demo_operation_progresses_and_cancels_without_a_late_completion() {
    let h = Harness::new();
    let created = h
        .invoke(
            "operations_start_demo",
            json!({ "request": { "steps": 200, "stepMs": 20 } }),
        )
        .expect("start demo");
    assert_eq!(created["status"], "CREATED");
    assert_eq!(created["kind"], "operations.demo");
    assert_eq!(created["actor"], json!({ "type": "USER" }));
    let id = created["id"].as_str().unwrap().to_owned();

    let running = h.wait_until(&id, Duration::from_secs(5), |op| {
        op["status"] == "RUNNING" && op["progress"]["current"].as_u64().unwrap_or(0) >= 1
    });
    assert_eq!(running["progress"]["total"], 200);

    let cancelled = h
        .invoke(
            "operations_cancel",
            json!({ "request": { "operationId": id } }),
        )
        .expect("cancel");
    assert_eq!(cancelled["cancellationRequested"], true);
    let bound = Instant::now();
    let final_op = h.wait_until(&id, Duration::from_secs(2), |op| {
        op["status"] == "CANCELLED"
    });
    assert!(
        bound.elapsed() < Duration::from_secs(1),
        "cancellation took {:?}",
        bound.elapsed()
    );
    assert!(final_op["finishedAt"].is_number());

    // Cancelling again is idempotent and nothing completes afterwards.
    let again = h
        .invoke(
            "operations_cancel",
            json!({ "request": { "operationId": id } }),
        )
        .expect("cancel again");
    assert_eq!(again["status"], "CANCELLED");
    std::thread::sleep(Duration::from_millis(300));
    let names = h.event_names();
    assert!(names.contains(&"OperationStarted".to_owned()), "{names:?}");
    assert!(names.contains(&"OperationProgress".to_owned()), "{names:?}");
    assert_eq!(
        names.iter().filter(|n| *n == "OperationCancelled").count(),
        1,
        "{names:?}"
    );
    assert!(
        !names.contains(&"OperationCompleted".to_owned()),
        "{names:?}"
    );
    assert_eq!(names.last().map(String::as_str), Some("OperationCancelled"));

    let seqs: Vec<u64> = h
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[1] == w[0] + 1),
        "seq not contiguous: {seqs:?}"
    );
    let envelope = &h.events.lock().unwrap()[0];
    assert_eq!(envelope["correlationId"], created["correlationId"]);
    assert_eq!(envelope["actor"], json!({ "type": "USER" }));
    assert!(envelope["payload"]["operation"].is_object());
}

#[test]
fn demo_operation_can_fail_and_reports_the_error_envelope() {
    let h = Harness::new();
    let created = h
        .invoke(
            "operations_start_demo",
            json!({ "request": { "steps": 5, "stepMs": 1, "failAtStep": 2 } }),
        )
        .expect("start demo");
    let id = created["id"].as_str().unwrap().to_owned();
    let failed = h.wait_until(&id, Duration::from_secs(5), |op| op["status"] == "FAILED");
    assert_eq!(failed["error"]["code"], "Internal");
    assert_eq!(failed["error"]["details"]["step"], 2);
    assert_eq!(failed["error"]["correlationId"], created["correlationId"]);
    assert!(h.event_names().contains(&"OperationFailed".to_owned()));
}
