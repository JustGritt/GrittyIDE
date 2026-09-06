//! Drives the real command adapters on Tauri's mock runtime: the same wiring
//! `run()` uses, invoked the way the WebView invokes it, with the event channel
//! observed from a Rust listener. This is the automated half of the T010/T020
//! acceptance flows (the other half is a human in front of the window).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gritty_core::shell::ExternalOpener;
use gritty_core::{CORE_EVENT_CHANNEL, CoreError, PROTOCOL_VERSION};
use gritty_desktop_lib::core::Core;
use gritty_desktop_lib::{CoreOptions, attach_core_with, configure};
use gritty_terminal::testing::FakePtyHost;
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{Listener, Manager, WebviewWindow, WebviewWindowBuilder};

/// Stands in for the system browser: records what the core asked to open.
#[derive(Default)]
struct RecordingOpener {
    opened: Mutex<Vec<String>>,
    fail: bool,
}

impl ExternalOpener for RecordingOpener {
    fn open(&self, url: &str) -> Result<(), CoreError> {
        if self.fail {
            return Err(std::io::Error::other("no handler registered").into());
        }
        self.opened.lock().unwrap().push(url.to_owned());
        Ok(())
    }
}

struct Harness {
    app: tauri::App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
    events: Arc<Mutex<Vec<Value>>>,
    opener: Arc<RecordingOpener>,
    /// No test spawns a real shell: the PTY tests that do live in
    /// `gritty-terminal`, and driving one from the mock runtime would make
    /// these adapter tests depend on a login shell's timing.
    pty: Arc<FakePtyHost>,
    /// Owned when the harness created it; a shared directory outlives the harness.
    _data_dir: Option<TempData>,
}

impl Harness {
    fn new() -> Self {
        Self::with_opener(Arc::new(RecordingOpener::default()))
    }

    fn with_opener(opener: Arc<RecordingOpener>) -> Self {
        let data = TempData::new("harness");
        let path = data.0.clone();
        let mut harness = Self::build(opener, path);
        harness._data_dir = Some(data);
        harness
    }

    /// The "next process": the same data directory, everything else fresh.
    fn with_data_dir(data_dir: &std::path::Path) -> Self {
        Self::build(Arc::new(RecordingOpener::default()), data_dir.to_path_buf())
    }

    fn build(opener: Arc<RecordingOpener>, data_dir: PathBuf) -> Self {
        let pty = FakePtyHost::new();
        let app = configure(mock_builder())
            .build(mock_context(noop_assets()))
            .expect("build app");
        // The mock runtime never runs `setup`; do what it would do. The
        // recording opener means no test can ever open a real browser, and
        // the temporary data directory means no test touches the real
        // session database.
        attach_core_with(
            app.handle(),
            CoreOptions {
                opener: opener.clone(),
                data_dir: Some(data_dir),
                pty_host: pty.clone(),
            },
        );
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
            app,
            webview,
            events,
            opener,
            pty,
            _data_dir: None,
        }
    }

    /// Make the session writer persist everything pending (what a clean exit does).
    fn flush(&self) {
        let core = self.app.state::<Core>();
        assert!(
            tauri::async_runtime::block_on(core.session.flush(Duration::from_secs(5))),
            "session writer flushed"
        );
    }

    fn session(&self, cmd: &str, request: Value) -> Result<Value, Value> {
        self.invoke(cmd, json!({ "request": request }))
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

    fn terminal(&self, cmd: &str, request: Value) -> Result<Value, Value> {
        self.invoke(cmd, json!({ "request": request }))
    }

    fn events_named(&self, name: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["name"] == name)
            .cloned()
            .collect()
    }
}

/// A scratch application data directory removed on drop.
struct TempData(PathBuf);

impl TempData {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "gritty-desktop-data-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempData {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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

// --- shell.openExternal (T050) -------------------------------------------------

#[test]
fn shell_open_external_hands_validated_urls_to_the_opener() {
    let h = Harness::new();
    let result = h
        .invoke(
            "shell_open_external",
            json!({ "request": { "url": "HTTPS://Example.com/docs?x=1" } }),
        )
        .expect("open");
    assert_eq!(result, Value::Null);
    assert_eq!(
        *h.opener.opened.lock().unwrap(),
        vec!["https://example.com/docs?x=1".to_owned()]
    );
}

#[test]
fn shell_open_external_rejects_everything_but_http_and_https_without_side_effects() {
    let h = Harness::new();
    for url in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "ftp://example.com/",
        "mailto:someone@example.com",
        "https://user:secret@example.com/",
        "https://exa\nmple.com/",
        "",
    ] {
        let error = h
            .invoke("shell_open_external", json!({ "request": { "url": url } }))
            .expect_err(url);
        assert_eq!(error["code"], "InvalidInput", "{url}");
        assert_eq!(error["details"]["field"], "url", "{url}");
        assert!(error["correlationId"].is_string(), "{url}");
    }
    let long = format!("https://example.com/{}", "a".repeat(3000));
    let error = h
        .invoke("shell_open_external", json!({ "request": { "url": long } }))
        .expect_err("too long");
    assert_eq!(error["code"], "InvalidInput");
    // A request that is not even the right shape is refused while parsing.
    let error = h
        .invoke("shell_open_external", json!({ "request": { "url": 42 } }))
        .expect_err("wrong type");
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "request");
    assert!(h.opener.opened.lock().unwrap().is_empty());
}

#[test]
fn shell_open_external_reports_a_handler_failure_as_io() {
    let h = Harness::with_opener(Arc::new(RecordingOpener {
        fail: true,
        ..Default::default()
    }));
    let error = h
        .invoke(
            "shell_open_external",
            json!({ "request": { "url": "https://example.com/" } }),
        )
        .expect_err("handler failure");
    assert_eq!(error["code"], "Io");
    assert_eq!(error["retryable"], true);
    assert!(error["correlationId"].is_string());
}

#[test]
fn the_workspace_history_survives_a_restart_and_forget_removes_one_entry() {
    let data = TempData::new("recents");
    let kept = TempWorkspace::new("recents-kept");
    let dropped = TempWorkspace::new("recents-dropped");
    let kept_root = fs::canonicalize(&kept.0).unwrap();
    let dropped_root = fs::canonicalize(&dropped.0).unwrap();

    {
        let h = Harness::with_data_dir(&data.0);
        assert_eq!(
            h.invoke("workspace_recents", json!({ "request": {} }))
                .expect("recents"),
            json!([]),
            "a first run has no history"
        );
        h.open_workspace(&kept.0);
        h.open_workspace(&dropped.0);
        h.flush();
    }

    // The next process reads what the previous one recorded.
    let h = Harness::with_data_dir(&data.0);
    let entries = h
        .invoke("workspace_recents", json!({ "request": {} }))
        .expect("recents");
    let roots = |value: &Value| {
        let mut roots = value
            .as_array()
            .expect("array")
            .iter()
            .map(|entry| entry["root"].as_str().expect("root").to_owned())
            .collect::<Vec<_>>();
        roots.sort();
        roots
    };
    let mut expected = vec![
        kept_root.to_string_lossy().into_owned(),
        dropped_root.to_string_lossy().into_owned(),
    ];
    expected.sort();
    assert_eq!(roots(&entries), expected);
    let entry = entries.as_array().unwrap().iter().next().unwrap();
    assert_eq!(entry["exists"], true);
    assert!(entry["displayRoot"].is_string());
    assert!(entry["name"].is_string());
    assert!(entry["lastOpenedAt"].is_number());

    h.invoke(
        "workspace_forget",
        json!({ "request": { "root": dropped_root.to_string_lossy() } }),
    )
    .expect("forget");
    assert_eq!(
        roots(
            &h.invoke("workspace_recents", json!({ "request": {} }))
                .expect("recents")
        ),
        vec![kept_root.to_string_lossy().into_owned()]
    );
    // Idempotent: pressing Forget twice is not an error.
    h.invoke(
        "workspace_forget",
        json!({ "request": { "root": dropped_root.to_string_lossy() } }),
    )
    .expect("forget again");

    let error = h
        .invoke("workspace_forget", json!({ "request": { "root": 42 } }))
        .unwrap_err();
    assert_eq!(error["code"], "InvalidInput");
    assert_eq!(error["details"]["field"], "request");
    assert!(error["correlationId"].is_string());
}

#[test]
fn session_restore_reopens_the_last_workspace_with_its_tabs_after_a_restart() {
    let data = TempData::new("session-restore");
    let root = TempWorkspace::new("session-restore");
    let canonical = fs::canonicalize(&root.0).unwrap();
    let workspace_id = {
        let h = Harness::with_data_dir(&data.0);
        let fresh = h.invoke("session_restore", json!({})).expect("restore");
        assert!(
            fresh["workspace"].is_null(),
            "nothing persisted yet: {fresh}"
        );
        assert_eq!(fresh["tabs"], json!([]));
        assert_eq!(fresh["persistence"]["available"], true);
        assert_eq!(fresh["persistence"]["healthy"], true);
        assert!(
            fresh["persistence"]["databasePath"]
                .as_str()
                .unwrap()
                .starts_with(data.0.to_str().unwrap()),
            "{fresh}"
        );
        assert_eq!(fresh["notices"], json!([]));

        let workspace = h.open_workspace(&root.0);
        h.session(
            "session_update",
            json!({
                "workspaceId": workspace["id"],
                "tabs": [
                    { "path": "README.md", "active": true, "viewMode": "preview",
                      "cursor": { "line": 1, "column": 3 }, "anchor": { "line": 1, "column": 1 },
                      "scrollLine": 1 },
                    { "path": "src/main.rs", "active": false }
                ],
                "preferences": { "version": 1, "fontSize": 15 },
                "layout": { "version": 1, "explorer": { "showHidden": true } }
            }),
        )
        .expect("update");
        h.flush();
        workspace["id"].clone()
    };

    let h = Harness::with_data_dir(&data.0);
    let restored = h.invoke("session_restore", json!({})).expect("restore");
    assert_eq!(
        restored["workspace"]["root"],
        canonical.to_string_lossy().as_ref()
    );
    assert_ne!(
        restored["workspace"]["id"], workspace_id,
        "ids are per process"
    );
    let tabs = restored["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0]["path"], "README.md");
    assert_eq!(tabs[0]["active"], true);
    assert_eq!(tabs[0]["viewMode"], "preview");
    assert_eq!(tabs[0]["cursor"], json!({ "line": 1, "column": 3 }));
    assert_eq!(tabs[0]["scrollLine"], 1);
    assert_eq!(tabs[1]["path"], "src/main.rs");
    assert_eq!(tabs[1]["active"], false);
    assert!(tabs[1].get("cursor").is_none());
    assert_eq!(restored["preferences"]["fontSize"], 15);
    assert_eq!(restored["layout"]["explorer"]["showHidden"], true);
    assert_eq!(restored["notices"], json!([]));
    assert_eq!(
        h.last_event("WorkspaceOpened")["payload"]["workspace"],
        restored["workspace"],
        "the explorer learns about the reopened workspace from the event"
    );
    assert_eq!(
        h.invoke("workspace_current", json!({})).expect("current"),
        restored["workspace"]
    );

    // A renderer reload: the same answer, no second reopen.
    let again = h
        .invoke("session_restore", json!({}))
        .expect("restore again");
    assert_eq!(again["workspace"], restored["workspace"]);
    assert_eq!(again["tabs"], restored["tabs"]);
    assert_eq!(
        h.event_names()
            .iter()
            .filter(|n| *n == "WorkspaceOpened")
            .count(),
        1
    );
}

#[test]
fn session_recovery_round_trip_through_the_adapters() {
    let data = TempData::new("session-recover");
    let root = TempWorkspace::new("session-recover");
    {
        let h = Harness::with_data_dir(&data.0);
        let workspace = h.open_workspace(&root.0);
        for (path, text) in [
            ("src/main.rs", "fn main() { unsaved() }\n"),
            ("README.md", "# unsaved\n"),
        ] {
            let content = h.open_document(&workspace["id"], path);
            let id = content["document"]["id"].clone();
            h.doc(
                "documents_update",
                json!({ "documentId": id, "baseVersion": 1, "text": text }),
            )
            .expect("update");
        }
        h.flush();
        // Death without a save: the disk still has the originals.
        assert_eq!(
            fs::read_to_string(root.0.join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
    }
    // README changed on disk meanwhile; main.rs did not.
    fs::write(root.0.join("README.md"), "# changed by someone else\n").unwrap();

    let h = Harness::with_data_dir(&data.0);
    let restored = h.invoke("session_restore", json!({})).expect("restore");
    let workspace_id = restored["workspace"]["id"].clone();
    let candidates = h
        .session("session_candidates", json!({ "workspaceId": workspace_id }))
        .expect("candidates");
    let candidates = candidates.as_array().unwrap();
    assert_eq!(candidates.len(), 2, "{candidates:?}");
    assert_eq!(candidates[0]["path"], "README.md");
    assert_eq!(candidates[0]["state"], "DISK_CHANGED");
    assert_eq!(candidates[1]["path"], "src/main.rs");
    assert_eq!(candidates[1]["state"], "SAME_AS_DISK");
    assert_eq!(candidates[1]["size"], "fn main() { unsaved() }\n".len());
    assert!(candidates[1]["updatedAt"].as_u64().unwrap() > 0);

    let recovered = h
        .session(
            "session_recover",
            json!({ "workspaceId": workspace_id, "path": "src/main.rs" }),
        )
        .expect("recover");
    assert_eq!(recovered["text"], "fn main() { unsaved() }\n");
    assert_eq!(recovered["document"]["dirty"], true);
    assert_eq!(
        recovered["document"]["externalModificationState"],
        "UNCHANGED"
    );
    assert_eq!(
        h.last_event("DocumentChanged")["payload"]["cause"],
        "RECOVERED"
    );
    assert_eq!(
        fs::read_to_string(root.0.join("src/main.rs")).unwrap(),
        "fn main() {}",
        "recovery never writes the disk"
    );

    let conflicted = h
        .session(
            "session_recover",
            json!({ "workspaceId": workspace_id, "path": "README.md" }),
        )
        .expect("recover changed");
    assert_eq!(conflicted["text"], "# unsaved\n");
    assert_eq!(
        conflicted["document"]["externalModificationState"],
        "CONFLICTED"
    );
    assert_eq!(
        h.last_event("DocumentExternalChangeDetected")["payload"]["previousState"],
        "UNCHANGED"
    );

    // Both are open and dirty now: nothing left to offer; discarding is idempotent.
    let none = h
        .session("session_candidates", json!({ "workspaceId": workspace_id }))
        .expect("candidates");
    assert_eq!(none, json!([]));
    h.session(
        "session_discard",
        json!({ "workspaceId": workspace_id, "path": "README.md" }),
    )
    .expect("discard");
    let gone = h
        .session(
            "session_recover",
            json!({ "workspaceId": workspace_id, "path": "README.md" }),
        )
        .unwrap_err();
    assert!(
        gone["code"] == "NotFound" || gone["code"] == "Conflict",
        "{gone}"
    );
}

#[test]
fn session_requests_are_validated_before_touching_anything() {
    let h = Harness::new();
    let root = TempWorkspace::new("session-validate");
    let workspace = h.open_workspace(&root.0);

    let malformed = h.session("session_candidates", json!({})).unwrap_err();
    assert_eq!(malformed["code"], "InvalidInput");
    let unknown = h
        .session("session_candidates", json!({ "workspaceId": "nope" }))
        .unwrap_err();
    assert_eq!(unknown["code"], "NotFound");
    let traversal = h
        .session(
            "session_recover",
            json!({ "workspaceId": workspace["id"], "path": "../secrets" }),
        )
        .unwrap_err();
    assert_eq!(traversal["code"], "InvalidInput");
    let missing = h
        .session(
            "session_recover",
            json!({ "workspaceId": workspace["id"], "path": "README.md" }),
        )
        .unwrap_err();
    assert_eq!(missing["code"], "NotFound");
    let tabs_without_workspace = h
        .session(
            "session_update",
            json!({ "tabs": [{ "path": "README.md" }] }),
        )
        .unwrap_err();
    assert_eq!(tabs_without_workspace["code"], "InvalidInput");
    let bad_tab_path = h
        .session(
            "session_update",
            json!({ "workspaceId": workspace["id"], "tabs": [{ "path": "/etc/passwd" }] }),
        )
        .unwrap_err();
    assert_eq!(bad_tab_path["code"], "InvalidInput");
    h.session(
        "session_discard",
        json!({ "workspaceId": workspace["id"], "path": "README.md" }),
    )
    .expect("discarding nothing is fine");
    let restored = h.invoke("session_restore", json!({})).expect("restore");
    assert_eq!(
        restored["tabs"],
        json!([]),
        "nothing was persisted by the refused requests"
    );
    assert_eq!(restored["workspace"]["id"], workspace["id"]);
}

// ---------------------------------------------------------------------------
// T090 terminal
// ---------------------------------------------------------------------------

/// Open a workspace and start a terminal in it.
fn terminal_harness() -> (Harness, TempData, Value, Value) {
    let harness = Harness::new();
    let dir = TempData::new("terminal-ws");
    fs::create_dir_all(dir.0.join("src")).unwrap();
    let workspace = harness.open_workspace(&dir.0);
    let terminal = harness
        .terminal("terminal_spawn", json!({ "workspaceId": workspace["id"] }))
        .expect("spawn a terminal");
    (harness, dir, workspace, terminal)
}

#[test]
fn a_terminal_starts_without_a_workspace() {
    // The whole point: no workspace has been opened on this harness.
    let harness = Harness::new();

    let terminal = harness
        .terminal("terminal_spawn", json!({}))
        .expect("a terminal needs no workspace");

    assert_eq!(terminal["status"], "RUNNING");
    assert!(
        terminal.get("workspaceId").is_none(),
        "a home session belongs to no workspace: {terminal}"
    );
    assert!(
        terminal.get("cwd").is_none(),
        "a home directory has no workspace-relative path: {terminal}"
    );

    let listed = harness.invoke("terminal_list", json!({})).expect("list");
    assert_eq!(listed.as_array().expect("an array").len(), 1);
}

#[test]
fn a_relative_cwd_without_a_workspace_is_invalid_input() {
    // The renderer may say which workspace, or say nothing. It may never name
    // a directory to root a shell in (ADR-010).
    let harness = Harness::new();

    let error = harness
        .terminal("terminal_spawn", json!({ "cwd": "src" }))
        .expect_err("a cwd needs a workspace");

    assert_eq!(error["code"], "InvalidInput");
}

#[test]
fn a_terminal_can_be_named_and_the_name_survives_a_relist() {
    let (harness, _dir, _workspace, terminal) = terminal_harness();

    let named = harness
        .terminal(
            "terminal_rename",
            json!({ "terminalId": terminal["id"], "name": "  claude  " }),
        )
        .expect("rename");
    assert_eq!(named["name"], "claude");

    // The name is core state, so a renderer that reloads and re-lists gets it
    // back with the session rather than losing it.
    let listed = harness.invoke("terminal_list", json!({})).expect("list");
    assert_eq!(listed[0]["name"], "claude");

    let cleared = harness
        .terminal("terminal_rename", json!({ "terminalId": terminal["id"] }))
        .expect("clear");
    assert!(cleared.get("name").is_none(), "{cleared}");

    let error = harness
        .terminal(
            "terminal_rename",
            json!({ "terminalId": terminal["id"], "name": "two\nlines" }),
        )
        .expect_err("a tab cannot draw a newline");
    assert_eq!(error["code"], "InvalidInput");
}

#[test]
fn a_terminal_can_be_coloured_and_the_colour_survives_a_relist() {
    let (harness, _dir, _workspace, terminal) = terminal_harness();

    let coloured = harness
        .terminal(
            "terminal_recolor",
            json!({ "terminalId": terminal["id"], "color": "BLUE" }),
        )
        .expect("recolor");
    assert_eq!(coloured["color"], "BLUE");

    // Core state, like the name and for the same reason: a renderer that
    // reloads and re-lists gets the grouping back rather than losing it.
    let listed = harness.invoke("terminal_list", json!({})).expect("list");
    assert_eq!(listed[0]["color"], "BLUE");

    let cleared = harness
        .terminal("terminal_recolor", json!({ "terminalId": terminal["id"] }))
        .expect("clearing is a recolor like any other");
    assert!(cleared.get("color").is_none(), "{cleared}");

    let error = harness
        .terminal(
            "terminal_recolor",
            json!({ "terminalId": terminal["id"], "color": "TEAL" }),
        )
        .expect_err("the palette is closed");
    assert_eq!(error["code"], "InvalidInput");
}

#[test]
fn terminal_profiles_reports_what_the_host_can_start() {
    let harness = Harness::new();

    // The fake host reports no shells, which must be an empty list rather than
    // an error: a machine Gritty cannot inspect still has a working default.
    let profiles = harness
        .invoke("terminal_profiles", json!({ "request": {} }))
        .expect("profiles");
    assert!(profiles.as_array().expect("an array").is_empty());

    // A configured shell that does not exist is reported with the reason, not
    // dropped: the user typed it and is owed an explanation.
    let profiles = harness
        .invoke(
            "terminal_profiles",
            json!({ "request": { "custom": ["/nope/fish"] } }),
        )
        .expect("profiles");
    let profiles = profiles.as_array().expect("an array");
    assert_eq!(profiles.len(), 1, "{profiles:?}");
    assert_eq!(profiles[0]["path"], "/nope/fish");
    assert_eq!(profiles[0]["isCustom"], true);
    assert_eq!(profiles[0]["available"], false);
    assert!(profiles[0]["unavailableReason"].is_string());

    let error = harness
        .invoke(
            "terminal_profiles",
            json!({ "request": { "custom": ["relative/fish"] } }),
        )
        .expect("a relative path is reported, not refused");
    assert_eq!(error.as_array().expect("an array")[0]["available"], false);
}

#[test]
fn terminal_spawn_returns_a_running_session_and_lists_it() {
    let (harness, _dir, workspace, terminal) = terminal_harness();

    assert_eq!(terminal["status"], "RUNNING");
    assert_eq!(terminal["workspaceId"], workspace["id"]);
    assert_eq!(terminal["cwd"], "");
    assert_eq!(terminal["cols"], 80);
    assert_eq!(terminal["rows"], 24);
    assert!(terminal["pid"].is_number());

    let listed = harness.invoke("terminal_list", json!({})).expect("list");
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], terminal["id"]);
}

#[test]
fn terminal_write_and_resize_reach_the_session() {
    let (harness, _dir, _workspace, terminal) = terminal_harness();

    harness
        .terminal(
            "terminal_write",
            json!({ "terminalId": terminal["id"], "data": "echo hi\n" }),
        )
        .expect("write");
    // A control character travels verbatim: the shell interprets Ctrl+C, not Gritty.
    harness
        .terminal(
            "terminal_write",
            json!({ "terminalId": terminal["id"], "data": "\u{3}" }),
        )
        .expect("write");
    assert_eq!(harness.pty.last().written(), "echo hi\n\u{3}");

    let resized = harness
        .terminal(
            "terminal_resize",
            json!({ "terminalId": terminal["id"], "cols": 120, "rows": 40 }),
        )
        .expect("resize");
    assert_eq!(resized["cols"], 120);
    assert_eq!(resized["rows"], 40);
}

#[test]
fn terminal_output_and_exit_reach_the_webview_in_order() {
    let (harness, _dir, _workspace, terminal) = terminal_harness();
    let session = harness.pty.last();

    session.emit("hello\r\n");
    session.emit_batch(gritty_terminal::OutputBatch {
        text: "after a flood".into(),
        truncated_bytes: 2048,
    });
    session.finish(gritty_terminal::TerminalExit {
        code: Some(0),
        signal: None,
    });
    // Anything the host produces after the ending is dropped.
    session.emit("too late");

    let outputs = harness.events_named("TerminalOutput");
    assert_eq!(outputs.len(), 2, "only the batches before the exit");
    assert_eq!(outputs[0]["payload"]["data"], "hello\r\n");
    assert_eq!(outputs[0]["payload"]["terminalId"], terminal["id"]);
    assert_eq!(outputs[0]["payload"]["truncatedBytes"], 0);
    assert_eq!(
        outputs[1]["payload"]["truncatedBytes"], 2048,
        "dropped output is reported on the wire"
    );

    let exits = harness.events_named("TerminalExited");
    assert_eq!(exits.len(), 1, "TerminalExited lands exactly once");
    assert_eq!(exits[0]["payload"]["code"], 0);
    assert_eq!(exits[0]["payload"]["terminal"]["status"], "EXITED");

    // Ordering: no output after the ending, in the sequence the WebView sees.
    let names = harness.event_names();
    let exit_at = names
        .iter()
        .position(|name| name == "TerminalExited")
        .unwrap();
    assert!(
        !names[exit_at..].contains(&"TerminalOutput".to_owned()),
        "output followed the exit: {names:?}"
    );

    let listed = harness.invoke("terminal_list", json!({})).expect("list");
    assert_eq!(listed[0]["status"], "EXITED");
    assert_eq!(listed[0]["exitCode"], 0);
}

#[test]
fn terminal_kill_is_idempotent_and_unknown_ids_are_not_found() {
    let (harness, _dir, _workspace, terminal) = terminal_harness();

    harness
        .terminal("terminal_kill", json!({ "terminalId": terminal["id"] }))
        .expect("kill");
    harness
        .terminal("terminal_kill", json!({ "terminalId": terminal["id"] }))
        .expect("a second kill is not an error");
    assert_eq!(harness.pty.last().record().kills, 1);

    let error = harness
        .terminal("terminal_kill", json!({ "terminalId": "nope" }))
        .expect_err("unknown terminal");
    assert_eq!(error["code"], "NotFound");
    assert!(error["correlationId"].is_string());

    let error = harness
        .terminal(
            "terminal_write",
            json!({ "terminalId": terminal["id"], "data": "x" }),
        )
        .expect_err("the session has ended");
    assert_eq!(error["code"], "Conflict");
}

#[test]
fn a_terminal_cannot_be_started_outside_the_workspace() {
    let harness = Harness::new();
    let dir = TempData::new("terminal-escape");
    let workspace = harness.open_workspace(&dir.0);

    for cwd in ["../..", "/etc"] {
        let error = harness
            .terminal(
                "terminal_spawn",
                json!({ "workspaceId": workspace["id"], "cwd": cwd }),
            )
            .expect_err("a path outside the workspace is refused");
        assert!(
            error["code"] == "InvalidInput" || error["code"] == "PermissionDenied",
            "unexpected envelope for {cwd}: {error}"
        );
    }
    assert!(harness.pty.sessions().is_empty(), "nothing was spawned");
}

#[test]
fn a_malformed_terminal_request_is_invalid_input() {
    let (harness, _dir, _workspace, _terminal) = terminal_harness();

    let error = harness
        .invoke("terminal_write", json!({ "request": { "terminalId": 7 } }))
        .expect_err("malformed request");
    assert_eq!(error["code"], "InvalidInput");
}

#[test]
fn shutdown_terminates_every_terminal() {
    let (harness, _dir, workspace, _terminal) = terminal_harness();
    harness
        .terminal("terminal_spawn", json!({ "workspaceId": workspace["id"] }))
        .expect("a second terminal");
    let sessions = harness.pty.sessions();
    assert_eq!(sessions.len(), 2);

    harness.app.state::<Core>().shutdown();

    for session in &sessions {
        assert!(session.is_finished(), "a shell survived shutdown");
    }
}

#[test]
fn linking_an_agent_session_to_a_workstream_round_trips_through_the_adapters() {
    let root = TempWorkspace::new("agent-link");
    let harness = Harness::new();
    let workspace = harness.open_workspace(&root.0);
    let workspace_id = workspace["id"].clone();

    let workstream = harness
        .session(
            "workstreams_create",
            json!({ "workspaceId": workspace_id, "title": "Attention queue" }),
        )
        .expect("create the workstream");
    let workstream_id = workstream["id"].clone();

    // A session id the roster has never reported. Allowed on purpose: a link
    // records a fact about the past, and the roster is a snapshot of now.
    let link = json!({
        "workspaceId": workspace_id,
        "workstreamId": workstream_id,
        "provider": "CLAUDE_CODE",
        "sessionId": "claude_code:pid-42",
    });
    let change = harness
        .session("workstreams_link_agent", link.clone())
        .expect("link");
    assert_eq!(change["changed"], true);
    assert_eq!(change["links"].as_array().unwrap().len(), 1);
    assert_eq!(change["links"][0]["sessionId"], "claude_code:pid-42");
    assert_eq!(change["links"][0]["provider"], "CLAUDE_CODE");

    // The same link again: a no-op that says so rather than a second row.
    let again = harness
        .session("workstreams_link_agent", link.clone())
        .expect("duplicate");
    assert_eq!(again["changed"], false);
    assert_eq!(again["links"].as_array().unwrap().len(), 1);

    let listed = harness
        .session(
            "workstreams_agent_links",
            json!({ "workspaceId": workspace_id }),
        )
        .expect("list the workspace's links");
    assert_eq!(listed.as_array().unwrap().len(), 1);

    // A link is not a change to the workstream, so nothing was emitted for it.
    assert!(
        !harness
            .event_names()
            .iter()
            .any(|name| name == "WorkstreamChanged"),
        "the command returns its own snapshot instead (T047's precedent)"
    );

    // The workstream's own wire shape is untouched by having a link.
    let relisted = harness
        .session("workstreams_list", json!({ "workspaceId": workspace_id }))
        .expect("list");
    assert_eq!(
        relisted[0], workstream,
        "hard rule 6: no provider field appeared"
    );

    let removed = harness
        .session("workstreams_unlink_agent", link)
        .expect("unlink");
    assert_eq!(removed["changed"], true);
    assert!(removed["links"].as_array().unwrap().is_empty());
}

#[test]
fn a_malformed_link_request_is_invalid_input_and_writes_nothing() {
    let root = TempWorkspace::new("agent-link-bad");
    let harness = Harness::new();
    let workspace = harness.open_workspace(&root.0);
    let workspace_id = workspace["id"].clone();
    let workstream = harness
        .session(
            "workstreams_create",
            json!({ "workspaceId": workspace_id, "title": "Bounded" }),
        )
        .expect("create");

    // An unknown provider never reaches the domain: it is not one of the
    // enum's values, so the request does not parse.
    let error = harness
        .invoke(
            "workstreams_link_agent",
            json!({ "request": {
                "workspaceId": workspace_id,
                "workstreamId": workstream["id"],
                "provider": "SOME_OTHER_CLI",
                "sessionId": "s-1",
            }}),
        )
        .expect_err("unknown provider");
    assert_eq!(error["code"], "InvalidInput");

    let listed = harness
        .session(
            "workstreams_agent_links",
            json!({ "workspaceId": workspace_id }),
        )
        .expect("list");
    assert!(listed.as_array().unwrap().is_empty());
}

#[test]
fn notifications_are_off_until_the_setting_is_turned_on_through_the_adapter() {
    // The plugin is a macOS dependency, so `configure` registers it there and
    // nowhere else. The contract is the same either way and is what this
    // asserts: the setting reports what the platform can actually do, rather
    // than a value baked in at the time the test was written.
    const SUPPORTED: bool = cfg!(target_os = "macos");

    let harness = Harness::new();
    // Whatever the platform supports, the setting still starts off — nothing
    // about launching the application arms it (T223 §2).
    let settings = harness
        .invoke("agents_notifications", json!({}))
        .expect("read");
    assert_eq!(settings["supported"], SUPPORTED);
    assert_eq!(settings["enabled"], false);

    let after = harness
        .session("agents_set_notifications", json!({ "enabled": true }))
        .expect("set");
    if SUPPORTED {
        // The desktop plugin answers `Granted` without a prompt of its own; the
        // real permission is the operating system's, and it is asked for by the
        // first notification rather than by this call. What is asserted here is
        // that the setting follows the platform's answer instead of assuming one.
        assert_eq!(after["enabled"], after["permission"] == "GRANTED");
    } else {
        // Absent rather than broken, which is the failure table's wording:
        // asking for a notification Gritty cannot raise leaves the setting off
        // and does not fail the command.
        assert_eq!(after["enabled"], false);
        assert_ne!(after["permission"], "GRANTED");
    }
    assert_eq!(
        harness
            .invoke("agents_notifications", json!({}))
            .expect("read back")["enabled"],
        after["enabled"]
    );

    let off = harness
        .session("agents_set_notifications", json!({ "enabled": false }))
        .expect("set");
    assert_eq!(off["enabled"], false);

    let malformed = harness
        .invoke(
            "agents_set_notifications",
            json!({ "request": { "enabled": 7 } }),
        )
        .expect_err("malformed");
    assert_eq!(malformed["code"], "InvalidInput");
}
