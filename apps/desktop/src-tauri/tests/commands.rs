//! Drives the real command adapters on Tauri's mock runtime: the same wiring
//! `run()` uses, invoked the way the WebView invokes it, with the event channel
//! observed from a Rust listener. This is the automated half of the T010/T020
//! acceptance flows (the other half is a human in front of the window).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    /// The request carries the webview's own URL: the local origin differs per
    /// platform (`tauri://localhost` on macOS/Linux, `http://tauri.localhost` on
    /// Windows) and a request from anywhere else is treated as remote and
    /// blocked by the capability ACL.
    fn invoke(&self, cmd: &str, body: Value) -> Result<Value, Value> {
        get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: self.webview.url().expect("webview url"),
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

    fn last_event(&self, name: &str) -> Value {
        self.events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|e| e["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no {name} event in {:?}", self.event_names()))
    }

    fn open_workspace(&self, path: &std::path::Path) -> Value {
        self.invoke(
            "workspace_open",
            json!({ "request": { "path": path.to_string_lossy() } }),
        )
        .expect("open workspace")
    }

    fn list(&self, workspace_id: &Value, path: &str) -> Result<Value, Value> {
        self.invoke(
            "workspace_list",
            json!({ "request": { "workspaceId": workspace_id, "path": path } }),
        )
    }
}

/// A scratch workspace root removed on drop.
struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("gritty-desktop-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("src")).unwrap();
        fs::write(path.join("README.md"), "# scratch").unwrap();
        fs::write(path.join(".env"), "SECRET=1").unwrap();
        fs::write(path.join("src/main.rs"), "fn main() {}").unwrap();
        Self(path)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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

#[test]
fn workspace_opens_lists_one_level_and_closes_through_the_adapters() {
    let h = Harness::new();
    let root = TempWorkspace::new("roundtrip");
    let empty = h.invoke("workspace_current", json!({})).expect("current");
    assert_eq!(empty, Value::Null);

    let workspace = h.open_workspace(&root.0);
    assert!(workspace["id"].is_string());
    assert_eq!(
        workspace["name"],
        root.0.file_name().unwrap().to_string_lossy().as_ref()
    );
    let opened = h.last_event("WorkspaceOpened");
    assert_eq!(opened["workspaceId"], workspace["id"]);
    assert_eq!(opened["payload"]["workspace"], workspace);
    let current = h.invoke("workspace_current", json!({})).expect("current");
    assert_eq!(current, workspace);

    let listing = h.list(&workspace["id"], "").expect("list root");
    assert_eq!(listing["status"], "ready");
    let entries = listing["page"]["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["src", ".env", "README.md"]);
    assert_eq!(entries[0]["kind"], "dir");
    assert_eq!(entries[1]["hidden"], true);
    assert_eq!(entries[2]["kind"], "file");
    assert_eq!(entries[2]["size"], 9);
    assert_eq!(listing["page"]["truncated"], false);
    assert_eq!(listing["page"]["total"], 3);

    let nested = h.list(&workspace["id"], "src").expect("list src");
    assert_eq!(nested["page"]["path"], "src");
    assert_eq!(nested["page"]["entries"][0]["name"], "main.rs");
    assert!(
        h.invoke("operations_list", json!({}))
            .expect("operations")
            .as_array()
            .unwrap()
            .is_empty(),
        "small listings never become operations"
    );

    let closed = h
        .invoke(
            "workspace_close",
            json!({ "request": { "workspaceId": workspace["id"] } }),
        )
        .expect("close");
    assert_eq!(closed, workspace);
    let event = h.last_event("WorkspaceClosed");
    assert_eq!(event["payload"]["reason"], "USER");
    assert_eq!(event["payload"]["workspaceId"], workspace["id"]);
    assert_eq!(
        h.invoke("workspace_current", json!({})).expect("current"),
        Value::Null
    );
    let error = h.list(&workspace["id"], "").unwrap_err();
    assert_eq!(error["code"], "NotFound");
}

#[test]
fn workspace_open_failures_carry_the_envelope_and_change_nothing() {
    let h = Harness::new();
    let missing = std::env::temp_dir().join("gritty-desktop-does-not-exist");
    let error = h
        .invoke(
            "workspace_open",
            json!({ "request": { "path": missing.to_string_lossy() } }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "NotFound");
    assert!(error["correlationId"].is_string());

    let root = TempWorkspace::new("file-root");
    let file = root.0.join("README.md");
    let error = h
        .invoke(
            "workspace_open",
            json!({ "request": { "path": file.to_string_lossy() } }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");

    let error = h
        .invoke("workspace_open", json!({ "request": { "path": 42 } }))
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "request");

    assert_eq!(
        h.invoke("workspace_current", json!({})).expect("current"),
        Value::Null
    );
    assert!(h.event_names().is_empty());
}

#[test]
fn workspace_list_rejects_traversal_unknown_ids_and_bad_pages() {
    let h = Harness::new();
    let root = TempWorkspace::new("policy");
    let workspace = h.open_workspace(&root.0);

    for path in ["..", "../etc", "src/../..", "/etc", r"src\main.rs"] {
        let error = h.list(&workspace["id"], path).unwrap_err();
        assert_eq!(error["code"], "InvalidInput", "{path}: {error}");
    }
    let error = h.list(&json!("not-open"), "").unwrap_err();
    assert_eq!(error["code"], "NotFound");
    let error = h.list(&workspace["id"], "missing").unwrap_err();
    assert_eq!(error["code"], "NotFound");
    let error = h.list(&workspace["id"], "README.md").unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    let error = h
        .invoke(
            "workspace_list",
            json!({ "request": { "workspaceId": workspace["id"], "path": "", "pageSize": 0 } }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "pageSize");
    let error = h
        .invoke(
            "workspace_list",
            json!({ "request": { "workspaceId": workspace["id"], "path": "", "cursor": "??" } }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "cursor");
}

#[test]
fn root_removed_while_open_fails_with_not_found_and_closes_the_workspace() {
    let h = Harness::new();
    let root = TempWorkspace::new("removed");
    let workspace = h.open_workspace(&root.0);
    fs::remove_dir_all(&root.0).unwrap();

    let error = h.list(&workspace["id"], "").unwrap_err();
    assert_eq!(error["code"], "NotFound");
    assert_eq!(error["details"]["reason"], "ROOT_MISSING");
    let closed = h.last_event("WorkspaceClosed");
    assert_eq!(closed["payload"]["reason"], "ROOT_MISSING");
    assert_eq!(closed["payload"]["workspaceId"], workspace["id"]);
    assert_eq!(closed["actor"], json!({ "type": "SYSTEM" }));
    assert_eq!(
        h.invoke("workspace_current", json!({})).expect("current"),
        Value::Null
    );
    assert_eq!(
        h.event_names(),
        vec!["WorkspaceOpened", "WorkspaceClosed"],
        "closed exactly once"
    );
}

// ---------------------------------------------------------------------------
// T030 documents
// ---------------------------------------------------------------------------

impl Harness {
    fn doc(&self, cmd: &str, request: Value) -> Result<Value, Value> {
        self.invoke(cmd, json!({ "request": request }))
    }

    fn open_document(&self, workspace_id: &Value, path: &str) -> Value {
        self.doc(
            "documents_open",
            json!({ "workspaceId": workspace_id, "path": path }),
        )
        .expect("open document")
    }
}

#[test]
fn documents_open_update_save_round_trip_through_the_adapters() {
    let h = Harness::new();
    let root = TempWorkspace::new("documents");
    let workspace = h.open_workspace(&root.0);

    let content = h.open_document(&workspace["id"], "src/main.rs");
    let doc = &content["document"];
    assert!(doc["id"].is_string());
    assert_eq!(doc["workspaceId"], workspace["id"]);
    assert_eq!(doc["path"], "src/main.rs");
    assert_eq!(doc["languageId"], "rust");
    assert_eq!(doc["encoding"], "utf-8");
    assert_eq!(doc["currentVersion"], 1);
    assert_eq!(doc["savedRevision"], 1);
    assert_eq!(doc["dirty"], false);
    assert_eq!(doc["externalModificationState"], "UNCHANGED");
    assert_eq!(content["text"], "fn main() {}");
    assert_eq!(h.last_event("DocumentOpened")["payload"]["document"], *doc);
    let id = doc["id"].clone();

    let updated = h
        .doc(
            "documents_update",
            json!({ "documentId": id, "baseVersion": 1, "text": "fn main() { println!(\"hi\"); }\n" }),
        )
        .expect("update");
    assert_eq!(updated["currentVersion"], 2);
    assert_eq!(updated["dirty"], true);
    assert_eq!(h.last_event("DocumentChanged")["payload"]["cause"], "EDIT");
    assert_eq!(
        fs::read_to_string(root.0.join("src/main.rs")).unwrap(),
        "fn main() {}",
        "updates never touch the disk"
    );

    let stale = h
        .doc(
            "documents_update",
            json!({ "documentId": id, "baseVersion": 1, "text": "x" }),
        )
        .unwrap_err();
    assert_eq!(stale["code"], "Conflict");
    assert_eq!(stale["details"]["currentVersion"], 2);

    let saved = h
        .doc("documents_save", json!({ "documentId": id }))
        .expect("save");
    assert_eq!(saved["dirty"], false);
    assert_eq!(saved["savedRevision"], 2);
    assert_eq!(
        fs::read_to_string(root.0.join("src/main.rs")).unwrap(),
        "fn main() { println!(\"hi\"); }\n"
    );
    assert_eq!(h.last_event("DocumentSaved")["payload"]["document"], saved);
    let leftovers: Vec<String> = fs::read_dir(root.0.join("src"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".gritty-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    let listed = h.invoke("documents_list", json!({})).expect("list");
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], id);

    let closed = h
        .doc("documents_close", json!({ "documentId": id }))
        .expect("close");
    assert_eq!(closed["id"], id);
    assert_eq!(h.last_event("DocumentClosed")["payload"]["reason"], "USER");
    assert_eq!(
        h.invoke("documents_list", json!({})).expect("list"),
        json!([])
    );
    let error = h
        .doc("documents_save", json!({ "documentId": id }))
        .unwrap_err();
    assert_eq!(error["code"], "NotFound");
}

#[test]
fn external_edit_while_dirty_is_a_conflict_until_forced_or_reloaded() {
    let h = Harness::new();
    let root = TempWorkspace::new("conflict");
    let workspace = h.open_workspace(&root.0);
    let id = h.open_document(&workspace["id"], "README.md")["document"]["id"].clone();
    h.doc(
        "documents_update",
        json!({ "documentId": id, "baseVersion": 1, "text": "# mine" }),
    )
    .expect("update");

    // `sed -i`-style external edit while the buffer is dirty.
    fs::write(root.0.join("README.md"), "# theirs").unwrap();
    let statuses = h
        .doc("documents_status", json!({ "documentId": id }))
        .expect("status");
    assert_eq!(statuses[0]["externalModificationState"], "CONFLICTED");
    let detected = h.last_event("DocumentExternalChangeDetected");
    assert_eq!(detected["payload"]["previousState"], "UNCHANGED");
    assert_eq!(detected["actor"], json!({ "type": "EXTERNAL" }));

    let refused = h
        .doc("documents_save", json!({ "documentId": id }))
        .unwrap_err();
    assert_eq!(refused["code"], "Conflict");
    assert_eq!(
        refused["details"]["externalModificationState"],
        "CONFLICTED"
    );
    assert_eq!(
        fs::read_to_string(root.0.join("README.md")).unwrap(),
        "# theirs"
    );

    // Closing the workspace would drop the edit: refused without force.
    let refused = h
        .invoke(
            "workspace_close",
            json!({ "request": { "workspaceId": workspace["id"] } }),
        )
        .unwrap_err();
    assert_eq!(refused["code"], "Conflict");
    assert_eq!(refused["details"]["dirtyDocuments"][0]["path"], "README.md");

    let forced = h
        .doc("documents_save", json!({ "documentId": id, "force": true }))
        .expect("forced save");
    assert_eq!(forced["externalModificationState"], "UNCHANGED");
    assert_eq!(
        fs::read_to_string(root.0.join("README.md")).unwrap(),
        "# mine"
    );

    fs::write(root.0.join("README.md"), "# again").unwrap();
    let all = h.doc("documents_status", json!({})).expect("status all");
    assert_eq!(all[0]["externalModificationState"], "MODIFIED_EXTERNALLY");
    let reloaded = h
        .doc("documents_reload", json!({ "documentId": id }))
        .expect("reload");
    assert_eq!(reloaded["text"], "# again");
    assert_eq!(
        reloaded["document"]["externalModificationState"],
        "UNCHANGED"
    );
    assert_eq!(
        h.last_event("DocumentChanged")["payload"]["cause"],
        "RELOAD"
    );

    let closed = h
        .invoke(
            "workspace_close",
            json!({ "request": { "workspaceId": workspace["id"] } }),
        )
        .expect("close workspace");
    assert_eq!(closed["id"], workspace["id"]);
    let event = h.last_event("DocumentClosed");
    assert_eq!(event["payload"]["reason"], "WORKSPACE_CLOSED");
    assert_eq!(event["payload"]["dirty"], false);
}

#[test]
fn document_requests_are_validated_before_touching_the_disk() {
    let h = Harness::new();
    let root = TempWorkspace::new("doc-policy");
    let workspace = h.open_workspace(&root.0);
    for path in ["../secret", "/etc/passwd", "src\\main.rs", ""] {
        let error = h
            .doc(
                "documents_open",
                json!({ "workspaceId": workspace["id"], "path": path }),
            )
            .unwrap_err();
        assert_eq!(error["code"], "InvalidInput", "{path}: {error}");
    }
    let error = h
        .doc(
            "documents_open",
            json!({ "workspaceId": workspace["id"], "path": "src" }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    let error = h
        .doc(
            "documents_open",
            json!({ "workspaceId": workspace["id"], "path": "missing.txt" }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "NotFound");
    let error = h
        .doc(
            "documents_open",
            json!({ "workspaceId": "nope", "path": "README.md" }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "NotFound");
    let error = h
        .doc(
            "documents_update",
            json!({ "documentId": "x", "baseVersion": "one", "text": "" }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "request");
    assert_eq!(
        h.invoke("documents_list", json!({})).expect("list"),
        json!([])
    );
    assert_eq!(h.event_names(), vec!["WorkspaceOpened"]);
}
