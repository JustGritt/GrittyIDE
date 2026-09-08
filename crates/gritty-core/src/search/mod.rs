//! Search (T080): `search.query` scans the live workspace tree and streams
//! matches as it finds them. Nothing is indexed.
//!
//! The walk lives in `gritty-search`; this module owns identity, the error
//! envelope, the operation and the *delivery* of matches — which is the part
//! that has a rule attached to it.
//!
//! ## Always an operation
//!
//! `workspace.list` waits a budget and only becomes an operation when the
//! directory outlives it (`ListDirectoryResponse::{Ready,Pending}`). A search
//! has no such fallback: a listing is bounded and completes as a page, while a
//! search is open-ended and its output is a stream. So `search.query` creates
//! the operation every time and returns it immediately, and the caller watches
//! `SearchMatchesFound` plus the usual `Operation*` events.
//!
//! ## What runs where (rule 10)
//!
//! Validation is synchronous and cheap: an empty query or a pattern that does
//! not compile is `InvalidInput` with no operation created and no event
//! emitted, exactly as `StartDemoRequest::validate` behaves. Everything after
//! that runs on the blocking pool through `spawn_blocking`, so no filesystem
//! work happens on a command handler or the renderer's thread. Cancellation is
//! `operations.cancel` and nothing else: there is no `search.cancel`, the
//! walk's own token is checked between files and inside large ones, and the
//! registry's "no `OperationCompleted` after a cancel" guarantee is reused
//! rather than reimplemented.
//!
//! ## What is bounded (rule 8)
//!
//! Match delivery is batched, throttled and queue-bounded in [`batch`], and a
//! dropped match is always counted into `truncatedMatches` rather than
//! silently missing. `OperationCompleted.result` carries a
//! [`SearchCompletion`] — counts only, never the match list, which already
//! streamed.

pub mod batch;
pub mod wire;

use std::sync::Arc;
use std::time::Instant;

use gritty_search::{CompiledQuery, Query, SearchError, SearchMatch, SearchSink};
use gritty_workspace::WorkspaceRoot;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::operation::registry::ProgressReporter;
use crate::operation::{
    Operation, OperationKind, OperationProgress, OperationRegistry, OperationSpec,
};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::{CorrelationId, OperationId};
use crate::workspace::WorkspaceManager;

use batch::{Batch, MatchBatcher};
use wire::{SearchCompletion, SearchMatchesFoundPayload, SearchQueryRequest};

/// Operation kind of a running search.
pub const SEARCH_KIND: &str = "search.query";

/// Runs searches over the open workspace.
pub struct SearchManager {
    workspaces: Arc<WorkspaceManager>,
    events: Arc<EventBus>,
    registry: Arc<OperationRegistry>,
    handle: Handle,
    /// Tests slow the blocking walk down so cancellation mid-stream can be
    /// exercised deterministically rather than by racing a real tree. The same
    /// hook `WorkspaceManager::listing_delay` exists for, and for the same
    /// reason.
    #[cfg(test)]
    scan_delay: std::sync::Mutex<std::time::Duration>,
}

impl SearchManager {
    pub fn new(
        workspaces: Arc<WorkspaceManager>,
        events: Arc<EventBus>,
        registry: Arc<OperationRegistry>,
        handle: Handle,
    ) -> Arc<Self> {
        Arc::new(Self {
            workspaces,
            events,
            registry,
            handle,
            #[cfg(test)]
            scan_delay: std::sync::Mutex::new(std::time::Duration::ZERO),
        })
    }

    /// `search.query`. Validates, then starts the operation the matches stream
    /// from. Invalid input has no side effects at all.
    pub fn query(
        self: &Arc<Self>,
        request: SearchQueryRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Operation, CoreError> {
        let (workspace, root) = self
            .workspaces
            .require(&request.workspace_id, &correlation)?;
        // Fast validation first: an empty query or an uncompilable pattern
        // must not leave an operation behind (T080 §2).
        let compiled = query_of(&request).compile()?;

        let spec = OperationSpec {
            kind: OperationKind::new(SEARCH_KIND),
            actor: actor.clone(),
            workspace_id: Some(workspace.id.clone()),
            correlation_id: correlation.clone(),
            durable: false,
        };
        let origin = EventOrigin {
            workspace_id: Some(workspace.id),
            correlation_id: Some(correlation),
            actor,
        };
        let events = Arc::clone(&self.events);
        let handle = self.handle.clone();
        #[cfg(test)]
        let delay = *self
            .scan_delay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        Ok(self.registry.start(spec, move |ctx| async move {
            // Cancelling this token is the only thing that stops the blocking
            // walk: `spawn_blocking` cannot be aborted. It is cancelled when
            // the operation is cancelled, and by `CancelOnDrop` when this body
            // finishes or is aborted at an await point.
            let token = CancellationToken::new();
            let guard = CancelOnDrop(token.clone());
            let sink = EmittingSink {
                events,
                origin,
                operation_id: ctx.id().clone(),
                reporter: ctx.reporter().clone(),
                batcher: MatchBatcher::default(),
            };
            let mut job = handle.spawn_blocking(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                run(&root, &compiled, sink, &token)
            });
            tokio::select! {
                biased;
                () = ctx.cancelled() => {
                    guard.0.cancel();
                    Err(CoreError::cancelled("search cancelled"))
                }
                joined = &mut job => {
                    let completion = joined
                        .map_err(|join| CoreError::internal(format!("search task failed: {join}")))??;
                    Ok(Some(serde_json::to_value(completion)?))
                }
            }
        }))
    }

    /// Slow the blocking walk down (tests only).
    #[cfg(test)]
    fn set_scan_delay(&self, delay: std::time::Duration) {
        *self
            .scan_delay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = delay;
    }
}

/// The blocking half: walk, deliver, then flush whatever the throttle held.
fn run(
    root: &WorkspaceRoot,
    compiled: &CompiledQuery,
    mut sink: EmittingSink,
    token: &CancellationToken,
) -> Result<SearchCompletion, CoreError> {
    let outcome = gritty_search::search(root, compiled, &mut sink, &|| token.is_cancelled());
    // Flush before propagating: matches already found are delivered even when
    // the walk ended early, because they are true.
    sink.flush();
    let summary = outcome?;
    Ok(SearchCompletion {
        summary,
        truncated_matches: sink.dropped_total(),
    })
}

fn query_of(request: &SearchQueryRequest) -> Query {
    let options = request.options.unwrap_or_default();
    Query {
        text: request.query.clone(),
        case_sensitive: options.case_sensitive.unwrap_or(false),
        whole_word: options.whole_word.unwrap_or(false),
        use_regex: options.use_regex.unwrap_or(false),
        scope: options.scope.unwrap_or_default(),
        include_ignored: options.include_ignored.unwrap_or(false),
    }
}

/// Turns matches into `SearchMatchesFound` events, under [`batch`]'s bounds.
struct EmittingSink {
    events: Arc<EventBus>,
    origin: EventOrigin,
    operation_id: OperationId,
    reporter: ProgressReporter,
    batcher: MatchBatcher,
}

impl EmittingSink {
    fn emit(&self, batch: Batch) {
        self.events.emit(
            &self.origin,
            CoreEvent::SearchMatchesFound(SearchMatchesFoundPayload {
                operation_id: self.operation_id.clone(),
                matches: batch.matches,
                truncated_matches: batch.truncated,
            }),
        );
    }

    fn pump(&mut self, now: Instant) {
        if let Some(batch) = self.batcher.take(now) {
            self.emit(batch);
        }
    }

    fn flush(&mut self) {
        for batch in self.batcher.drain() {
            self.emit(batch);
        }
    }

    const fn dropped_total(&self) -> u64 {
        self.batcher.dropped_total()
    }
}

impl SearchSink for EmittingSink {
    fn matched(&mut self, found: SearchMatch) -> bool {
        let keep = self.batcher.push(found);
        self.pump(Instant::now());
        keep
    }

    fn scanned(&mut self, files: u64) {
        // Generic scan progress, sharing no event with match streaming. There
        // is no `total`: a live walk does not know how many files it will see
        // until it has seen them, and inventing one would be a lie the
        // progress bar tells.
        self.reporter.report(OperationProgress {
            current: files,
            total: None,
            message: Some(format!("scanned {files} files")),
        });
        // A quiet stretch is also a chance to deliver what the throttle held.
        self.pump(Instant::now());
    }
}

/// Cancels the walk's token when dropped, so nothing is left scanning after
/// the operation body goes away.
struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl From<SearchError> for CoreError {
    fn from(error: SearchError) -> Self {
        match error {
            SearchError::InvalidQuery { field, reason } => Self::invalid_input(field, reason),
            SearchError::Workspace(inner) => Self::from(inner),
            SearchError::Io {
                ref context,
                ref source,
            } => Self::new(CoreErrorCode::Io, format!("{context}: {source}")),
            SearchError::Cancelled => Self::cancelled("search cancelled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use gritty_search::MatchKind;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::*;
    use crate::event::EventEnvelope;
    use crate::event::testing::RecordingSink;
    use crate::operation::OperationStatus;
    use crate::protocol::error::CoreErrorCode;
    use crate::protocol::ids::WorkspaceId;
    use crate::workspace::UnsavedPolicy;
    use wire::SearchOptions;

    struct Harness {
        searches: Arc<SearchManager>,
        workspaces: Arc<WorkspaceManager>,
        registry: Arc<OperationRegistry>,
        sink: Arc<RecordingSink>,
        rx: mpsc::UnboundedReceiver<EventEnvelope>,
    }

    fn harness() -> Harness {
        let (sink, rx) = RecordingSink::with_channel();
        let events = crate::event::EventBus::new(sink.clone());
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry.clone());
        let searches = SearchManager::new(
            workspaces.clone(),
            events,
            registry.clone(),
            Handle::current(),
        );
        Harness {
            searches,
            workspaces,
            registry,
            sink,
            rx,
        }
    }

    impl Harness {
        async fn next_named(&mut self, name: &str) -> EventEnvelope {
            timeout(Duration::from_secs(10), async {
                loop {
                    let envelope = self.rx.recv().await.expect("event stream open");
                    if envelope.event.name() == name {
                        return envelope;
                    }
                }
            })
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {name}"))
        }

        /// Every `SearchMatchesFound` batch delivered so far.
        fn batches(&self) -> Vec<SearchMatchesFoundPayload> {
            self.sink
                .recorded()
                .into_iter()
                .filter_map(|envelope| match envelope.event {
                    CoreEvent::SearchMatchesFound(payload) => Some(payload),
                    _ => None,
                })
                .collect()
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "gritty-core-search-{label}-{}-{n}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(path.join("README.md"), "the needle is here\n").unwrap();
            fs::write(
                path.join("src/main.rs"),
                "fn main() {\n    // needle\n    println!(\"needle\");\n}\n",
            )
            .unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, contents: impl AsRef<[u8]>) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    async fn open(h: &Harness, dir: &TempDir) -> crate::workspace::Workspace {
        h.workspaces
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .expect("open workspace")
    }

    fn request(workspace_id: &WorkspaceId, query: &str) -> SearchQueryRequest {
        SearchQueryRequest {
            workspace_id: workspace_id.clone(),
            query: query.to_owned(),
            options: None,
        }
    }

    #[tokio::test]
    async fn an_invalid_query_creates_no_operation_and_emits_nothing() {
        let h = harness();
        let dir = TempDir::new("invalid");
        let workspace = open(&h, &dir).await;
        let before = h.sink.recorded().len();

        for (query, options) in [
            ("", None),
            ("   ", None),
            (
                "foo(",
                Some(SearchOptions {
                    use_regex: Some(true),
                    ..SearchOptions::default()
                }),
            ),
        ] {
            let error = h
                .searches
                .query(
                    SearchQueryRequest {
                        options,
                        ..request(&workspace.id, query)
                    },
                    Actor::User,
                    CorrelationId::new(),
                )
                .expect_err("invalid");
            assert_eq!(error.code, CoreErrorCode::InvalidInput, "{query:?}");
            assert_eq!(error.details.unwrap()["field"], "query", "{query:?}");
        }
        assert!(h.registry.list().is_empty(), "no operation was created");
        assert_eq!(h.sink.recorded().len(), before, "no event was emitted");
    }

    #[tokio::test]
    async fn a_workspace_that_is_not_open_is_not_found_before_the_query_is_read() {
        let h = harness();
        let error = h
            .searches
            .query(
                // Also an invalid query: the workspace must be checked first,
                // so a stale id never reads as a typo in the query.
                request(&WorkspaceId::new(), ""),
                Actor::User,
                CorrelationId::new(),
            )
            .expect_err("not found");
        assert_eq!(error.code, CoreErrorCode::NotFound);
        assert!(h.registry.list().is_empty());
    }

    #[tokio::test]
    async fn a_search_always_becomes_an_operation_and_streams_its_matches() {
        let mut h = harness();
        let dir = TempDir::new("stream");
        let workspace = open(&h, &dir).await;

        let operation = h
            .searches
            .query(
                request(&workspace.id, "needle"),
                Actor::User,
                CorrelationId::new(),
            )
            .expect("started");
        assert_eq!(operation.kind.as_str(), SEARCH_KIND);
        assert_eq!(operation.status, OperationStatus::Created);
        assert_eq!(operation.workspace_id, Some(workspace.id.clone()));

        let completed = h.next_named("OperationCompleted").await;
        let CoreEvent::OperationCompleted(payload) = completed.event else {
            unreachable!()
        };
        assert_eq!(payload.operation.id, operation.id);
        let completion: SearchCompletion =
            serde_json::from_value(payload.result.expect("a summary")).expect("summary shape");
        assert_eq!(completion.truncated_matches, 0);
        // `README.md` line 1, `src/main.rs` lines 2 and 3. Three matches, not
        // four: a line with two occurrences is reported once, line-oriented.
        assert_eq!(completion.summary.matches_found, 3);
        assert_eq!(completion.summary.files_matched, 2);
        assert!(!completion.summary.limit_reached);

        let batches = h.batches();
        assert!(!batches.is_empty(), "matches streamed");
        let delivered: usize = batches.iter().map(|batch| batch.matches.len()).sum();
        assert_eq!(delivered, 3);
        assert!(
            batches
                .iter()
                .all(|batch| batch.operation_id == operation.id),
            "every batch names its operation"
        );
        assert!(
            batches.iter().all(|batch| batch.truncated_matches == 0),
            "nothing was dropped"
        );
        let lines: Vec<Option<u64>> = batches
            .iter()
            .flat_map(|batch| batch.matches.iter())
            .filter(|found| found.kind == MatchKind::Content)
            .map(|found| found.line)
            .collect();
        assert!(lines.iter().all(Option::is_some), "{lines:?}");
    }

    #[tokio::test]
    async fn the_completion_summary_carries_counts_and_never_the_matches() {
        let mut h = harness();
        let dir = TempDir::new("summary");
        dir.write("binary.bin", [0u8, b'n', b'e', 0u8]);
        dir.write("node_modules/pkg/index.js", "needle\n");
        let workspace = open(&h, &dir).await;

        h.searches
            .query(
                request(&workspace.id, "needle"),
                Actor::User,
                CorrelationId::new(),
            )
            .expect("started");
        let completed = h.next_named("OperationCompleted").await;
        let CoreEvent::OperationCompleted(payload) = completed.event else {
            unreachable!()
        };
        let raw = payload.result.expect("a summary");
        let text = serde_json::to_string(&raw).unwrap();
        assert!(
            !text.contains("needle"),
            "the result must carry counts, not matches: {text}"
        );
        let completion: SearchCompletion = serde_json::from_value(raw).unwrap();
        assert_eq!(completion.summary.skipped.binary, 1);
        assert_eq!(completion.summary.skipped.ignored_directories, 1);
    }

    #[tokio::test]
    async fn options_reach_the_walk() {
        let mut h = harness();
        let dir = TempDir::new("options");
        dir.write("node_modules/pkg/index.js", "needle\n");
        let workspace = open(&h, &dir).await;

        h.searches
            .query(
                SearchQueryRequest {
                    options: Some(SearchOptions {
                        include_ignored: Some(true),
                        scope: Some(gritty_search::SearchScope::Content),
                        ..SearchOptions::default()
                    }),
                    ..request(&workspace.id, "needle")
                },
                Actor::User,
                CorrelationId::new(),
            )
            .expect("started");
        h.next_named("OperationCompleted").await;
        let found: Vec<String> = h
            .batches()
            .iter()
            .flat_map(|batch| batch.matches.iter())
            .map(|found| found.path.to_string())
            .collect();
        assert!(
            found.contains(&"node_modules/pkg/index.js".to_owned()),
            "{found:?}"
        );
    }

    #[tokio::test]
    async fn cancelling_mid_stream_ends_cancelled_with_no_further_batches() {
        let mut h = harness();
        let dir = TempDir::new("cancel");
        // Enough files that the walk is still going when the cancel lands.
        for index in 0..400 {
            dir.write(&format!("gen/f{index:04}.txt"), "needle\n".repeat(20));
        }
        let workspace = open(&h, &dir).await;
        // Slow the blocking job so the cancel is guaranteed to arrive while it
        // is running, rather than racing a fast tree.
        h.searches.set_scan_delay(Duration::from_millis(150));

        let operation = h
            .searches
            .query(
                request(&workspace.id, "needle"),
                Actor::User,
                CorrelationId::new(),
            )
            .expect("started");
        h.next_named("OperationStarted").await;
        let cancelled = h.registry.cancel(&operation.id).expect("cancel accepted");
        assert!(cancelled.cancellation_requested);

        let ended = h.next_named("OperationCancelled").await;
        let CoreEvent::OperationCancelled(payload) = ended.event else {
            unreachable!()
        };
        assert_eq!(payload.operation.id, operation.id);
        assert_eq!(
            h.registry.get(&operation.id).unwrap().status,
            OperationStatus::Cancelled
        );

        // No `OperationCompleted` for a cancelled search, and no batch after
        // the cancellation event: the registry's guarantee, reused.
        let recorded = h.sink.recorded();
        let cancel_at = recorded
            .iter()
            .position(|envelope| envelope.event.name() == "OperationCancelled")
            .expect("the cancellation is in the log");
        assert!(
            !recorded
                .iter()
                .any(|envelope| envelope.event.name() == "OperationCompleted"),
            "a cancelled search must not also complete"
        );
        assert!(
            !recorded[cancel_at + 1..]
                .iter()
                .any(|envelope| envelope.event.name() == "SearchMatchesFound"),
            "matches arrived after the search was cancelled"
        );
        // `seq` is contiguous across everything the sink saw.
        let seqs: Vec<u64> = recorded.iter().map(|envelope| envelope.meta.seq).collect();
        assert!(
            seqs.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "sequence gap: {seqs:?}"
        );
    }

    #[tokio::test]
    async fn a_search_reports_scan_progress_without_inventing_a_total() {
        let mut h = harness();
        let dir = TempDir::new("progress");
        for index in 0..200 {
            dir.write(&format!("gen/f{index:04}.txt"), "needle\n");
        }
        let workspace = open(&h, &dir).await;
        h.searches
            .query(
                request(&workspace.id, "needle"),
                Actor::User,
                CorrelationId::new(),
            )
            .expect("started");
        h.next_named("OperationCompleted").await;
        let progress: Vec<crate::operation::OperationProgress> = h
            .sink
            .recorded()
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                CoreEvent::OperationProgress(payload) => Some(payload.progress),
                _ => None,
            })
            .collect();
        assert!(!progress.is_empty(), "progress was reported");
        assert!(
            progress.iter().all(|report| report.total.is_none()),
            "a live walk knows no total: {progress:?}"
        );
        assert!(progress.iter().all(|report| report.current > 0));
    }

    #[tokio::test]
    async fn a_root_that_disappeared_is_not_found_before_anything_is_started() {
        let h = harness();
        let dir = TempDir::new("vanish");
        let workspace = open(&h, &dir).await;
        fs::remove_dir_all(&dir.0).unwrap();

        // `require` notices first and closes the workspace (`ROOT_MISSING`),
        // so no operation is created for a tree that is not there. A root that
        // goes away *after* that check is the walk's own `Io` failure
        // (`gritty-search`'s `a_root_that_disappeared_fails_rather_than_...`).
        let error = h
            .searches
            .query(
                request(&workspace.id, "needle"),
                Actor::User,
                CorrelationId::new(),
            )
            .expect_err("not found");
        assert_eq!(error.code, CoreErrorCode::NotFound);
        assert_eq!(error.details.unwrap()["reason"], "ROOT_MISSING");
        assert!(h.registry.list().is_empty());
    }

    #[test]
    fn search_errors_map_onto_the_core_envelope() {
        let invalid = CoreError::from(SearchError::invalid_query("query", "nope"));
        assert_eq!(invalid.code, CoreErrorCode::InvalidInput);
        let cancelled = CoreError::from(SearchError::Cancelled);
        assert_eq!(cancelled.code, CoreErrorCode::Cancelled);
        let io = CoreError::from(SearchError::io(
            "a.txt",
            std::io::Error::other("disk on fire"),
        ));
        assert_eq!(io.code, CoreErrorCode::Io);
        assert!(io.message.contains("a.txt"));
    }
}
