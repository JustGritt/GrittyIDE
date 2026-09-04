//! Files and directories dropped onto the window.
//!
//! The platform hands the composition root a list of absolute native paths.
//! They are untrusted input like anything else that arrives from outside
//! (`AGENTS.md` rule 13): nothing is opened here. The core canonicalizes each
//! path, decides what it *could* mean for the workspace that is open, and
//! reports that through one event. The renderer applies policy and asks for
//! what it wants through the commands that already exist — `workspace.open`
//! for a directory, `documents.open` for a file inside the workspace.
//!
//! Why classification lives in the core and not in the WebView: deciding
//! whether a path is inside the workspace is path identity, which belongs to
//! `gritty-workspace` (ADR-010), and the WebView has no filesystem access to
//! answer it with.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use gritty_workspace::{WorkspacePath, WorkspaceRoot, canonicalize};

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::ids::WorkspaceId;
use crate::workspace::WorkspaceManager;

/// What a dropped path can mean. The renderer chooses what to do with each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DroppedKind {
    /// A directory: a candidate workspace root.
    Directory,
    /// A file inside the workspace that is open. `workspacePath` names it.
    FileInWorkspace,
    /// A file that no open workspace contains. Gritty keys documents by root
    /// plus relative path (ADR-010/011), so this one has no identity until a
    /// workspace containing it is open; `parent` is the directory to offer.
    FileOutsideWorkspace,
    /// The path did not resolve — deleted between the drag and the drop, a
    /// broken symlink, or unreadable.
    Unresolvable,
    /// It resolved, but it is neither a regular file nor a directory (a
    /// socket, a device, a fifo).
    Unsupported,
}

/// One dropped path, classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DroppedPath {
    /// Canonical absolute path, native form. Display and `workspace.open`.
    pub path: String,
    /// Last component, for messages.
    pub name: String,
    pub kind: DroppedKind,
    /// Set for `FILE_IN_WORKSPACE`: what to pass to `documents.open`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_path: Option<WorkspacePath>,
    /// Set for `FILE_IN_WORKSPACE`: which workspace it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_id: Option<WorkspaceId>,
    /// Set for `FILE_OUTSIDE_WORKSPACE`: the directory to offer opening.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parent: Option<String>,
}

/// `PathsDropped`: everything one drop gesture carried, already classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PathsDroppedPayload {
    pub items: Vec<DroppedPath>,
}

/// `DropHoverChanged`: a drag entered or left the window, so the UI can show
/// and hide its drop target.
///
/// Enter and leave only. The platform also reports every mouse move during a
/// drag; forwarding those would be exactly the unbounded high-frequency
/// stream `AGENTS.md` rule 8 forbids, and the overlay does not need them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DropHoverPayload {
    pub active: bool,
}

/// The open workspace, as classification needs to see it.
pub struct DropContext {
    pub workspace_id: WorkspaceId,
    pub root: Arc<WorkspaceRoot>,
}

/// Classify one drop. `context` is `None` when no workspace is open, in which
/// case every file is outside one.
///
/// Order is preserved and every input produces exactly one output: the
/// renderer reports what it skipped, so nothing may disappear silently.
pub fn classify(paths: &[PathBuf], context: Option<&DropContext>) -> PathsDroppedPayload {
    PathsDroppedPayload {
        items: paths
            .iter()
            .map(|path| classify_one(path, context))
            .collect(),
    }
}

fn classify_one(path: &Path, context: Option<&DropContext>) -> DroppedPath {
    let display = |p: &Path| p.to_string_lossy().into_owned();
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| display(p))
    };

    // Canonicalize first: the platform may hand over a path through a symlink,
    // and every comparison against a root is made on canonical paths.
    let Ok(canonical) = canonicalize(path) else {
        return DroppedPath {
            path: display(path),
            name: name(path),
            kind: DroppedKind::Unresolvable,
            workspace_path: None,
            workspace_id: None,
            parent: None,
        };
    };

    let Ok(metadata) = std::fs::metadata(&canonical) else {
        return DroppedPath {
            path: display(&canonical),
            name: name(&canonical),
            kind: DroppedKind::Unresolvable,
            workspace_path: None,
            workspace_id: None,
            parent: None,
        };
    };

    if metadata.is_dir() {
        return DroppedPath {
            path: display(&canonical),
            name: name(&canonical),
            kind: DroppedKind::Directory,
            workspace_path: None,
            workspace_id: None,
            parent: None,
        };
    }

    if !metadata.is_file() {
        return DroppedPath {
            path: display(&canonical),
            name: name(&canonical),
            kind: DroppedKind::Unsupported,
            workspace_path: None,
            workspace_id: None,
            parent: None,
        };
    }

    // A regular file: inside the open workspace, or not.
    if let Some(context) = context
        && context.root.contains(&canonical)
        && let Some(relative) = context.root.relativize(&canonical)
    {
        return DroppedPath {
            path: display(&canonical),
            name: name(&canonical),
            kind: DroppedKind::FileInWorkspace,
            workspace_path: Some(relative),
            workspace_id: Some(context.workspace_id.clone()),
            parent: None,
        };
    }

    DroppedPath {
        path: display(&canonical),
        name: name(&canonical),
        kind: DroppedKind::FileOutsideWorkspace,
        workspace_path: None,
        workspace_id: None,
        parent: canonical.parent().map(display),
    }
}

/// Turns a platform drop into the one event the renderer listens for.
///
/// The composition root owns the platform event; this owns what it means. The
/// service opens nothing: a drop that the user did not follow up on must leave
/// the workspace exactly as it was.
pub struct FileDropService {
    events: Arc<EventBus>,
    workspaces: Arc<WorkspaceManager>,
}

impl FileDropService {
    pub fn new(events: Arc<EventBus>, workspaces: Arc<WorkspaceManager>) -> Arc<Self> {
        Arc::new(Self { events, workspaces })
    }

    /// Classify `paths` against the open workspace and announce them.
    pub fn dropped(&self, paths: &[PathBuf]) {
        let context = self
            .workspaces
            .current_open()
            .map(|(workspace_id, root)| DropContext { workspace_id, root });
        let payload = classify(paths, context.as_ref());
        tracing::debug!(count = payload.items.len(), "paths dropped onto the window");
        self.events.emit(
            &EventOrigin {
                workspace_id: context.map(|c| c.workspace_id),
                correlation_id: None,
                actor: Actor::User,
            },
            CoreEvent::PathsDropped(payload),
        );
    }

    /// A drag entered (`true`) or left (`false`) the window.
    pub fn hover(&self, active: bool) {
        // Logged because this is the seam where the platform hands over. A
        // drag the OS never reported and a drag the UI ignored look identical
        // from the outside; this line is what tells them apart. Bounded by
        // construction — enter and leave only, never `Over` (rule 8).
        tracing::debug!(active, "drag hover changed");
        self.events.emit(
            &EventOrigin {
                workspace_id: self.workspaces.current_open().map(|(id, _)| id),
                correlation_id: None,
                actor: Actor::User,
            },
            CoreEvent::DropHoverChanged(DropHoverPayload { active }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gritty-drop-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("workspace/src")).expect("fixture");
            std::fs::create_dir_all(dir.join("elsewhere")).expect("fixture");
            std::fs::write(dir.join("workspace/src/lib.rs"), "fn main() {}").expect("fixture");
            std::fs::write(dir.join("elsewhere/notes.md"), "# notes").expect("fixture");
            Self { dir }
        }

        fn context(&self) -> DropContext {
            DropContext {
                workspace_id: WorkspaceId::new(),
                root: Arc::new(
                    WorkspaceRoot::open(&self.dir.join("workspace")).expect("workspace root"),
                ),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_file_inside_the_workspace_carries_the_path_to_open_it_with() {
        let fixture = Fixture::new("inside");
        let context = fixture.context();
        let dropped = classify(&[fixture.dir.join("workspace/src/lib.rs")], Some(&context));
        let item = &dropped.items[0];
        assert_eq!(item.kind, DroppedKind::FileInWorkspace);
        assert_eq!(item.name, "lib.rs");
        assert_eq!(
            item.workspace_path.as_ref().map(WorkspacePath::as_str),
            Some("src/lib.rs")
        );
        assert_eq!(item.workspace_id.as_ref(), Some(&context.workspace_id));
        assert!(item.parent.is_none());
    }

    #[test]
    fn a_file_outside_the_workspace_offers_its_parent_instead() {
        let fixture = Fixture::new("outside");
        let context = fixture.context();
        let dropped = classify(&[fixture.dir.join("elsewhere/notes.md")], Some(&context));
        let item = &dropped.items[0];
        assert_eq!(item.kind, DroppedKind::FileOutsideWorkspace);
        assert!(item.workspace_path.is_none(), "no identity without a root");
        assert!(
            item.parent
                .as_ref()
                .expect("a parent to offer")
                .ends_with("elsewhere")
        );
    }

    #[test]
    fn with_no_workspace_open_every_file_is_outside_one() {
        let fixture = Fixture::new("noworkspace");
        let dropped = classify(&[fixture.dir.join("workspace/src/lib.rs")], None);
        assert_eq!(dropped.items[0].kind, DroppedKind::FileOutsideWorkspace);
    }

    #[test]
    fn a_directory_is_a_candidate_workspace_wherever_it_is() {
        let fixture = Fixture::new("dir");
        let context = fixture.context();
        let dropped = classify(
            &[
                fixture.dir.join("elsewhere"),
                fixture.dir.join("workspace/src"),
            ],
            Some(&context),
        );
        assert_eq!(dropped.items[0].kind, DroppedKind::Directory);
        assert_eq!(dropped.items[1].kind, DroppedKind::Directory);
    }

    #[test]
    fn a_path_that_no_longer_resolves_is_reported_not_dropped() {
        let fixture = Fixture::new("missing");
        let context = fixture.context();
        let dropped = classify(&[fixture.dir.join("workspace/gone.rs")], Some(&context));
        assert_eq!(dropped.items[0].kind, DroppedKind::Unresolvable);
        assert_eq!(dropped.items[0].name, "gone.rs");
    }

    #[test]
    fn every_dropped_path_produces_exactly_one_item_in_order() {
        let fixture = Fixture::new("order");
        let context = fixture.context();
        let paths = vec![
            fixture.dir.join("workspace/src/lib.rs"),
            fixture.dir.join("workspace/gone.rs"),
            fixture.dir.join("elsewhere"),
        ];
        let dropped = classify(&paths, Some(&context));
        assert_eq!(dropped.items.len(), 3, "nothing may be dropped silently");
        assert_eq!(dropped.items[0].kind, DroppedKind::FileInWorkspace);
        assert_eq!(dropped.items[1].kind, DroppedKind::Unresolvable);
        assert_eq!(dropped.items[2].kind, DroppedKind::Directory);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_the_workspace_is_resolved_before_it_is_judged() {
        let fixture = Fixture::new("symlink");
        let context = fixture.context();
        let link = fixture.dir.join("elsewhere/link.rs");
        std::os::unix::fs::symlink(fixture.dir.join("workspace/src/lib.rs"), &link)
            .expect("symlink");
        let dropped = classify(&[link], Some(&context));
        // The link lives outside the root but its target does not: what gets
        // opened is the target, so the target is what decides.
        assert_eq!(dropped.items[0].kind, DroppedKind::FileInWorkspace);
        assert_eq!(
            dropped.items[0]
                .workspace_path
                .as_ref()
                .map(WorkspacePath::as_str),
            Some("src/lib.rs")
        );
    }
}
