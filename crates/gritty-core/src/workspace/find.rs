//! `workspace.findFiles` (T130-C): the ranked-path half of the workspace wire.
//!
//! `list` answers "what is in this directory", one level at a time. This
//! answers "which files *look like* `epn`", over the whole tree, ranked. It is
//! `workspace.*` rather than `search.*` for that reason: both questions are
//! about which paths exist, and `search.query` stays the grep surface.
//!
//! ## Why this is not `search.query` with a scope
//!
//! `SearchMatch` carries a single `matchStart`/`matchLength` pair. A substring
//! hit is one contiguous run, but a subsequence hit — `epn` →
//! **e**ditor-**p**a**n**e — is a *set* of positions, and cannot be expressed on
//! that shape at all. Ranking client-side over `search.query` would therefore
//! also mean re-running the matcher in TypeScript purely to highlight: the
//! scorer written twice, in two languages, drifting.
//!
//! ## The budget hand-over, and why the pending arm is the normal case
//!
//! Same shape as [`list`](super::WorkspaceManager::list): the walk runs on the
//! blocking pool and the command waits up to a budget, after which the *same*
//! job continues as a cancellable operation whose completion carries the same
//! [`FoundFiles`]. T080 measured a filename-scope walk of 10,429 files at 94 ms
//! against a 100 ms budget, so a repository modestly larger than the fixture
//! tips past it every time. The pending arm is not a decoration.
//!
//! Cancellation is `operations.cancel`, as `search.query`'s is — one
//! cancellation surface rather than a second one nobody would find.
//! `spawn_blocking` cannot be aborted, so a [`CancellationToken`] the walk
//! checks between entries is how it learns.
//!
//! Nothing here adds a `CoreEvent` variant, deliberately: a bounded top-N
//! response does not need one, and avoiding it keeps this off
//! `crates/gritty-core/src/event/mod.rs` and the exhaustive match in
//! `crates/gritty-core/src/session/tap.rs` — the pair `docs/merge-queue.md`
//! warns merges *cleanly and wrongly*.

use std::sync::Arc;
use std::time::Duration;

use gritty_search::{
    DEFAULT_FIND_LIMIT, MAX_FIND_LIMIT, RankedFiles, Scorer, SearchError, rank_workspace_files,
};
use gritty_workspace::WorkspacePath;
use serde::{Deserialize, Serialize};
use tokio::task::JoinError;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use super::{CancelOnDrop, WorkspaceManager};
use crate::operation::{Operation, OperationKind, OperationSpec};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, WorkspaceId};

/// Operation kind of a ranked walk that outlived its budget.
pub const FIND_KIND: &str = "workspace.findFiles";

/// How long `workspace.findFiles` waits before handing the walk to an operation.
///
/// The same 100 ms as `DEFAULT_LIST_BUDGET`, for the same reason: it is roughly
/// the point past which a synchronous answer stops feeling synchronous.
pub const DEFAULT_FIND_BUDGET: Duration = Duration::from_millis(100);

/// `workspace.findFiles`: the best-ranked paths in the open workspace for
/// `query`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FindFilesRequest {
    pub workspace_id: WorkspaceId,
    /// What the path should look like. Matched as a case-insensitive
    /// subsequence over the workspace-relative path, not as a substring.
    ///
    /// An empty query is not an error: it returns the first `limit` paths in
    /// walk order, which is what a quick-open shows before a key is pressed.
    pub query: String,
    /// How many matches to return, `1..=200`; defaults to 50.
    ///
    /// Outside that range is `InvalidInput` rather than a clamp, matching
    /// `pageSize`'s treatment in `ListDirectoryRequest`: a caller that asked
    /// for 5000 wanted something this command cannot do, and silently giving
    /// it 200 would hide that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// One ranked path, with the spans that matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FoundFile {
    pub path: WorkspacePath,
    /// Higher is better. Comparable only within one response: the scale is the
    /// scorer's bonuses, which are tuned relative to each other and to nothing
    /// else. Do not render it.
    pub score: i32,
    /// Matched spans as `(start, length)` in **char** indices over `path`,
    /// ascending, non-overlapping, adjacent runs merged.
    ///
    /// Char and not byte, so a path containing a non-ASCII character
    /// highlights correctly. A TypeScript consumer must slice by code point
    /// (`Array.from(path)`) rather than by `String.prototype.slice`, whose
    /// indices are UTF-16 code units and disagree past the BMP.
    ///
    /// Present so the UI highlights without re-running the matcher, which is
    /// the whole reason this is not a client-side rank over `search.query`.
    pub ranges: Vec<(u32, u32)>,
}

/// What the walk found: the payload of both response arms.
///
/// The `ready` arm carries these two fields inline and the operation completes
/// with this struct, so a caller that has to handle both writes one mapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FoundFiles {
    /// Best first: score descending, then workspace-relative path ascending.
    /// The tie-break is on the path and never on walk order, so two identical
    /// queries cannot return two different orders.
    pub matches: Vec<FoundFile>,
    /// The walk scored a candidate it then discarded to stay within `limit`.
    ///
    /// Always said out loud: a truncated list must never read as a complete
    /// one. `matches.len() == limit` is *not* the same fact — a corpus with
    /// exactly `limit` matches is complete.
    pub truncated: bool,
}

/// What `workspace.findFiles` returns: the matches when the walk finished
/// within the budget, otherwise the operation that will complete with the same
/// [`FoundFiles`] in `OperationCompleted.result` (or end `CANCELLED` /
/// `FAILED`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FindFilesResponse {
    Ready {
        matches: Vec<FoundFile>,
        truncated: bool,
    },
    Pending {
        operation: Box<Operation>,
    },
}

impl From<RankedFiles> for FoundFiles {
    fn from(ranked: RankedFiles) -> Self {
        Self {
            matches: ranked
                .matches
                .into_iter()
                .map(|ranked| FoundFile {
                    path: ranked.path,
                    score: ranked.score,
                    ranges: ranked.ranges,
                })
                .collect(),
            truncated: ranked.truncated,
        }
    }
}

impl From<FoundFiles> for FindFilesResponse {
    fn from(found: FoundFiles) -> Self {
        Self::Ready {
            matches: found.matches,
            truncated: found.truncated,
        }
    }
}

impl WorkspaceManager {
    /// Rank the workspace's paths against `query`. Runs off the caller's
    /// thread; see the module documentation for the budget hand-over.
    pub async fn find_files(
        self: &Arc<Self>,
        request: FindFilesRequest,
        actor: Actor,
        correlation: CorrelationId,
        budget: Duration,
    ) -> Result<FindFilesResponse, CoreError> {
        let (workspace, root) = self.require(&request.workspace_id, &correlation)?;
        let limit = match request.limit {
            None => DEFAULT_FIND_LIMIT,
            Some(limit) if (1..=MAX_FIND_LIMIT as u32).contains(&limit) => limit as usize,
            Some(_) => {
                return Err(CoreError::invalid_input(
                    "limit",
                    format!("must be between 1 and {MAX_FIND_LIMIT}"),
                ));
            }
        };
        // Validate the query before anything exists, the way `search.query`
        // compiles its pattern first: an over-long query must not leave an
        // operation behind. `Scorer::new` is the single place that rule lives,
        // so this is the same check and not a second copy of it.
        Scorer::new(&request.query)?;

        let token = CancellationToken::new();
        #[cfg(test)]
        let delay = *self
            .find_delay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut job = {
            let root = Arc::clone(&root);
            let query = request.query.clone();
            let token = token.clone();
            self.handle.spawn_blocking(move || {
                #[cfg(test)]
                std::thread::sleep(delay);
                rank_workspace_files(&root, &query, limit, &|| token.is_cancelled())
            })
        };
        match tokio::time::timeout(budget, &mut job).await {
            Ok(joined) => Ok(finish_walk(joined, limit)?.into()),
            Err(_elapsed) => {
                // The query is what a person typed into a field, and a path
                // fragment is workspace data (rule 13). Counts and durations
                // only; never the query and never a path.
                tracing::debug!(
                    workspace = %workspace.id,
                    query_chars = request.query.chars().count(),
                    limit,
                    budget_ms = budget.as_millis() as u64,
                    "ranked walk exceeded its budget; continuing as an operation"
                );
                let spec = OperationSpec {
                    kind: OperationKind::new(FIND_KIND),
                    actor,
                    workspace_id: Some(workspace.id),
                    correlation_id: correlation,
                    durable: false,
                };
                let operation = self.registry.start(spec, move |ctx| async move {
                    // Stops the blocking walk when the body finishes, is
                    // cancelled, or is aborted at an await point.
                    let job_token = CancelOnDrop(token);
                    tokio::select! {
                        biased;
                        () = ctx.cancelled() => {
                            job_token.0.cancel();
                            Err(CoreError::cancelled("file search cancelled"))
                        }
                        joined = &mut job => {
                            let found = finish_walk(joined, limit)?;
                            Ok(Some(serde_json::to_value(found)?))
                        }
                    }
                });
                Ok(FindFilesResponse::Pending {
                    operation: Box::new(operation),
                })
            }
        }
    }
}

/// Join the blocking walk and map its two failure modes.
///
/// A root that vanished between the command and the walk surfaces as the
/// walk's `io` error rather than an empty `ready` list: "it failed" and "there
/// were none" are different answers and only one of them is true.
fn finish_walk(
    joined: Result<Result<RankedFiles, SearchError>, JoinError>,
    limit: usize,
) -> Result<FoundFiles, CoreError> {
    let ranked = joined
        .map_err(|join| CoreError::internal(format!("file search task failed: {join}")))??;
    tracing::debug!(
        matches = ranked.matches.len(),
        limit,
        truncated = ranked.truncated,
        files_scanned = ranked.files_scanned,
        directories_scanned = ranked.directories_scanned,
        elapsed_ms = ranked.elapsed_ms,
        "workspace ranked"
    );
    Ok(ranked.into())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gritty_search::MAX_QUERY_CHARS;
    use tokio::runtime::Handle;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::*;
    use crate::event::testing::RecordingSink;
    use crate::event::{CoreEvent, EventBus, EventEnvelope};
    use crate::operation::{OperationRegistry, OperationStatus};
    use crate::protocol::error::CoreErrorCode;
    use crate::workspace::{UnsavedPolicy, Workspace};

    /// Long enough that a walk of four files always finishes inside it, so the
    /// `ready` tests are not timing tests.
    const AMPLE_BUDGET: Duration = Duration::from_secs(30);
    /// Short enough that the hand-over happens, paired with `find_delay` so the
    /// walk is still running when it does.
    const SHORT_BUDGET: Duration = Duration::from_millis(10);

    struct Harness {
        manager: Arc<WorkspaceManager>,
        registry: Arc<OperationRegistry>,
        sink: Arc<RecordingSink>,
        rx: mpsc::UnboundedReceiver<EventEnvelope>,
    }

    fn harness() -> Harness {
        let (sink, rx) = RecordingSink::with_channel();
        let events = EventBus::new(sink.clone());
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let manager = WorkspaceManager::new(Handle::current(), events, registry.clone());
        Harness {
            manager,
            registry,
            sink,
            rx,
        }
    }

    impl Harness {
        async fn next_named(&mut self, name: &str) -> EventEnvelope {
            timeout(Duration::from_secs(5), async {
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

        /// Every operation event this run has emitted, in order.
        fn operation_events(&self) -> Vec<&'static str> {
            self.sink
                .names()
                .into_iter()
                .filter(|name| name.starts_with("Operation"))
                .collect()
        }
    }

    /// A tree with a deliberate near-miss in it: `epn` matches both
    /// `editor-pane.tsx` and `expand.rs`, so a response that returns the second
    /// first is a ranking failure and not an empty result.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "gritty-core-find-{label}-{}-{n}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(path.join("src/editor-pane.tsx"), "x").unwrap();
            fs::write(path.join("src/expand.rs"), "x").unwrap();
            fs::write(path.join("README.md"), "x").unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn open(h: &Harness, dir: &TempDir) -> Workspace {
        h.manager
            .open(
                &dir.0,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .expect("workspace opens")
    }

    fn request(workspace: &Workspace, query: &str) -> FindFilesRequest {
        FindFilesRequest {
            workspace_id: workspace.id.clone(),
            query: query.to_string(),
            limit: None,
        }
    }

    async fn find(
        h: &Harness,
        request: FindFilesRequest,
        budget: Duration,
    ) -> Result<FindFilesResponse, CoreError> {
        h.manager
            .find_files(request, Actor::User, CorrelationId::new(), budget)
            .await
    }

    fn ready(response: FindFilesResponse) -> (Vec<FoundFile>, bool) {
        match response {
            FindFilesResponse::Ready { matches, truncated } => (matches, truncated),
            other => panic!("expected a ready response, got {other:?}"),
        }
    }

    /// The acceptance criterion, over a corpus somebody else wrote.
    ///
    /// A temporary tree proves the wiring; it cannot prove a *ranking*, because
    /// the person writing the assertion also chose the counter-examples. That
    /// is precisely how T130-R's scorer passed nineteen tests while ranking
    /// `epn` wrongly. `fixtures/small-repo` is committed and deterministic
    /// (`fixtures/README.md`), and `lib` is a real three-way contest in it.
    #[tokio::test]
    async fn the_committed_fixture_ranks_a_basename_above_a_directory_component() {
        let h = harness();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/small-repo");
        let workspace = h
            .manager
            .open(
                &root,
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .expect("the committed fixture opens");

        let (matches, truncated) = ready(
            find(&h, request(&workspace, "lib"), AMPLE_BUDGET)
                .await
                .expect("the walk"),
        );
        let ranked: Vec<&str> = matches.iter().map(|found| found.path.as_str()).collect();
        assert_eq!(
            ranked,
            vec![
                // `lib` is the whole basename and starts it.
                "crates/demo/src/lib.rs",
                // In these two it is a *directory* component, borrowed from
                // above the file name — the weaker signal, and the one whose
                // absence made `pixel.png` outrank `editor-pane.tsx`.
                "src/lib/util.ts",
                "src/lib/nested/deep/leaf.ts",
            ],
            "the ranking changed; re-judge it rather than re-writing the assertion"
        );
        assert!(!truncated);
        // Every match carries spans, and they index the path by char.
        for found in &matches {
            let chars = found.path.as_str().chars().count() as u32;
            for (start, len) in &found.ranges {
                assert!(
                    start + len <= chars,
                    "{}: range ({start}, {len}) runs past {chars} chars",
                    found.path.as_str()
                );
            }
        }
    }

    #[tokio::test]
    async fn a_query_inside_the_budget_answers_directly_and_ranks() {
        let h = harness();
        let dir = TempDir::new("ready");
        let workspace = open(&h, &dir);

        let (matches, truncated) = ready(
            find(&h, request(&workspace, "epn"), AMPLE_BUDGET)
                .await
                .expect("the walk succeeds"),
        );

        assert!(!truncated, "three files cannot exhaust the default limit");
        assert_eq!(
            matches.first().map(|found| found.path.as_str()),
            Some("src/editor-pane.tsx"),
            "epn must rank editor-pane.tsx above expand.rs: {matches:?}"
        );
        // The whole reason this is not a client-side rank over `search.query`:
        // the highlight arrives with the match.
        assert!(
            !matches[0].ranges.is_empty(),
            "a match carries the spans that matched"
        );
        assert!(
            matches[0]
                .ranges
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "ranges are ascending: {:?}",
            matches[0].ranges
        );
        assert!(h.operation_events().is_empty(), "no operation was needed");
    }

    #[tokio::test]
    async fn an_empty_query_is_not_an_error() {
        let h = harness();
        let dir = TempDir::new("empty-query");
        let workspace = open(&h, &dir);

        let (matches, _) = ready(
            find(&h, request(&workspace, ""), AMPLE_BUDGET)
                .await
                .expect("an empty query is what the field shows before a key is pressed"),
        );

        assert_eq!(matches.len(), 3, "every path, in walk order: {matches:?}");
        assert!(
            matches.iter().all(|found| found.ranges.is_empty()),
            "nothing was matched, so nothing is highlighted"
        );
    }

    #[tokio::test]
    async fn truncated_is_set_when_the_limit_discarded_a_scored_candidate() {
        let h = harness();
        let dir = TempDir::new("truncated");
        let workspace = open(&h, &dir);

        let mut one = request(&workspace, "e");
        one.limit = Some(1);
        let (matches, truncated) = ready(find(&h, one, AMPLE_BUDGET).await.expect("the walk"));

        assert_eq!(matches.len(), 1);
        assert!(
            truncated,
            "more than one path matched `e` and only one came back"
        );
    }

    #[tokio::test]
    async fn a_limit_outside_the_range_is_refused_before_any_walk() {
        let h = harness();
        let dir = TempDir::new("limit");
        let workspace = open(&h, &dir);

        for bad in [0u32, MAX_FIND_LIMIT as u32 + 1, 5000] {
            let mut over = request(&workspace, "e");
            over.limit = Some(bad);
            let error = find(&h, over, AMPLE_BUDGET)
                .await
                .expect_err("outside 1..=200 is InvalidInput, not a clamp");
            assert_eq!(error.code, CoreErrorCode::InvalidInput, "limit {bad}");
        }

        // The two edges are inside the range, so they must not be refused.
        for good in [1u32, MAX_FIND_LIMIT as u32] {
            let mut edge = request(&workspace, "e");
            edge.limit = Some(good);
            find(&h, edge, AMPLE_BUDGET)
                .await
                .unwrap_or_else(|error| panic!("limit {good} is inside the range: {error:?}"));
        }

        assert!(
            h.operation_events().is_empty(),
            "invalid input left an operation behind: {:?}",
            h.operation_events()
        );
    }

    #[tokio::test]
    async fn an_over_long_query_is_refused_before_any_walk() {
        let h = harness();
        let dir = TempDir::new("long-query");
        let workspace = open(&h, &dir);

        let long = "a".repeat(MAX_QUERY_CHARS + 1);
        let error = find(&h, request(&workspace, &long), AMPLE_BUDGET)
            .await
            .expect_err("above MAX_QUERY_CHARS is InvalidInput, never a silent truncation");
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(h.operation_events().is_empty());

        // Exactly at the limit is not over it.
        let at = "a".repeat(MAX_QUERY_CHARS);
        find(&h, request(&workspace, &at), AMPLE_BUDGET)
            .await
            .expect("the limit itself is allowed");
    }

    #[tokio::test]
    async fn an_unknown_workspace_is_not_found() {
        let h = harness();
        let dir = TempDir::new("unknown");
        let workspace = open(&h, &dir);

        let mut stranger = request(&workspace, "e");
        stranger.workspace_id = WorkspaceId::new();
        let error = find(&h, stranger, AMPLE_BUDGET)
            .await
            .expect_err("an id nobody opened");
        assert_eq!(error.code, CoreErrorCode::NotFound);
    }

    #[tokio::test]
    async fn a_walk_over_its_budget_becomes_an_operation_carrying_the_same_matches() {
        let mut h = harness();
        let dir = TempDir::new("pending");
        let workspace = open(&h, &dir);
        *h.manager.find_delay.lock().unwrap() = Duration::from_millis(150);

        let correlation = CorrelationId::new();
        let started = std::time::Instant::now();
        let response = h
            .manager
            .find_files(
                request(&workspace, "epn"),
                Actor::User,
                correlation.clone(),
                SHORT_BUDGET,
            )
            .await
            .expect("the hand-over is not a failure");
        assert!(
            started.elapsed() < Duration::from_millis(140),
            "the command returned within its budget, not after the walk ({:?})",
            started.elapsed()
        );

        let FindFilesResponse::Pending { operation } = response else {
            panic!("expected a pending operation, got {response:?}");
        };
        assert_eq!(operation.kind.as_str(), FIND_KIND);
        assert_eq!(operation.workspace_id, Some(workspace.id.clone()));
        assert_eq!(operation.correlation_id, correlation);

        let completed = h.next_named("OperationCompleted").await;
        let CoreEvent::OperationCompleted(payload) = completed.event else {
            panic!("expected OperationCompleted");
        };
        assert_eq!(payload.operation.id, operation.id);
        // The same payload the ready arm would have carried, which is the
        // point of `FoundFiles` being one type.
        let found: FoundFiles =
            serde_json::from_value(payload.result.expect("a completion carries its result"))
                .expect("the completion result is FoundFiles");
        assert_eq!(
            found.matches.first().map(|found| found.path.as_str()),
            Some("src/editor-pane.tsx")
        );
        assert!(!found.truncated);
    }

    #[tokio::test]
    async fn a_pending_walk_cancels_promptly_and_no_late_completion_overwrites_it() {
        let mut h = harness();
        let dir = TempDir::new("cancel");
        let workspace = open(&h, &dir);
        *h.manager.find_delay.lock().unwrap() = Duration::from_millis(300);

        let response = h
            .manager
            .find_files(
                request(&workspace, "epn"),
                Actor::User,
                CorrelationId::new(),
                SHORT_BUDGET,
            )
            .await
            .expect("pending");
        let FindFilesResponse::Pending { operation } = response else {
            panic!("expected a pending operation, got {response:?}");
        };

        h.next_named("OperationStarted").await;
        let cancelled = h.registry.cancel(&operation.id).expect("a live operation");
        assert!(cancelled.cancellation_requested);

        let bound = std::time::Instant::now();
        h.next_named("OperationCancelled").await;
        assert!(
            bound.elapsed() < Duration::from_millis(250),
            "cancellation waited for the walk ({:?})",
            bound.elapsed()
        );

        // Let the blocking walk run past its delay. Its result must not
        // surface: a cancelled operation stays cancelled.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let names = h.sink.names();
        assert!(
            !names.contains(&"OperationCompleted"),
            "a late completion overwrote the cancellation: {names:?}"
        );
        assert_eq!(
            h.registry.get(&operation.id).expect("still known").status,
            OperationStatus::Cancelled
        );
    }
}
