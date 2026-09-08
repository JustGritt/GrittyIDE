//! The undo journal for tree operations (T042, ADR-028).
//!
//! Pure state: a bounded stack of completed operations, each stored as the two
//! [`Step`]s that undo and redo it. It performs nothing — `files.rs` executes a
//! step against the filesystem — which is what lets the ordering, the bound and
//! the redo-clearing rule be tested without touching a disk.
//!
//! Three properties the shape is chosen for:
//!
//! - **Bounded.** [`JOURNAL_LIMIT`] entries; the oldest is dropped. An
//!   unbounded history of filesystem inverses is a leak with a friendly name.
//! - **Not persisted.** It lives as long as its workspace. ⌘Z reaching across a
//!   restart would cross a boundary the user cannot see (ADR-028).
//! - **A delete is never in it.** Trashed entries are recovered from the Trash,
//!   and a permanent delete is gone. Nothing here pretends otherwise, so
//!   `record` is simply never called for one.

use gritty_workspace::{EntryKind, WorkspacePath};

/// How many operations one workspace remembers.
pub const JOURNAL_LIMIT: usize = 32;

/// One direction of one operation: what to do to the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Remove each entry. The inverse of a create or a copy.
    Remove(Vec<WorkspacePath>),
    /// Move each `from` to `to`. Its own inverse, with the pairs flipped.
    Move(Vec<(WorkspacePath, WorkspacePath)>),
    /// Create each entry, empty. Only ever redoes a create, which is why an
    /// empty file is the whole of it.
    Create(Vec<(WorkspacePath, EntryKind)>),
    /// Copy each `from` to `to`. Only ever redoes a copy.
    Copy(Vec<(WorkspacePath, WorkspacePath)>),
}

/// A completed operation and the two ways to move through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    /// What the UI says it will undo — "Rename src/a.ts", "Move 3 items".
    pub label: String,
    pub undo: Step,
    pub redo: Step,
}

impl JournalEntry {
    pub fn create(path: WorkspacePath, kind: EntryKind) -> Self {
        Self {
            label: format!("Create {path}"),
            undo: Step::Remove(vec![path.clone()]),
            redo: Step::Create(vec![(path, kind)]),
        }
    }

    /// A rename and a move are the same operation; only the label differs.
    pub fn moved(pairs: Vec<(WorkspacePath, WorkspacePath)>, label: String) -> Self {
        let back = pairs
            .iter()
            .map(|(from, to)| (to.clone(), from.clone()))
            .collect();
        Self {
            label,
            undo: Step::Move(back),
            redo: Step::Move(pairs),
        }
    }

    pub fn copied(pairs: Vec<(WorkspacePath, WorkspacePath)>, label: String) -> Self {
        let made = pairs.iter().map(|(_, to)| to.clone()).collect();
        Self {
            label,
            undo: Step::Remove(made),
            redo: Step::Copy(pairs),
        }
    }
}

/// The undo/redo stacks of one open workspace.
#[derive(Debug, Default)]
pub struct Journal {
    done: Vec<JournalEntry>,
    undone: Vec<JournalEntry>,
}

impl Journal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a completed operation. Clears the redo stack, because a new
    /// operation makes the branch that was undone unreachable — the same rule
    /// every text editor's undo follows.
    pub fn record(&mut self, entry: JournalEntry) {
        self.undone.clear();
        self.done.push(entry);
        if self.done.len() > JOURNAL_LIMIT {
            self.done.remove(0);
        }
    }

    /// The operation ⌘Z would reverse, without taking it.
    pub fn peek_undo(&self) -> Option<&JournalEntry> {
        self.done.last()
    }

    pub fn peek_redo(&self) -> Option<&JournalEntry> {
        self.undone.last()
    }

    /// Take the next operation to undo. The caller performs its `undo` step and
    /// then calls [`Self::undo_succeeded`] or [`Self::undo_failed`] — the entry
    /// is not moved until the filesystem agreed, so a refused undo leaves the
    /// stack exactly as it was.
    pub fn take_undo(&mut self) -> Option<JournalEntry> {
        self.done.pop()
    }

    pub fn take_redo(&mut self) -> Option<JournalEntry> {
        self.undone.pop()
    }

    /// The undo happened: it becomes redoable.
    pub fn undo_succeeded(&mut self, entry: JournalEntry) {
        self.undone.push(entry);
    }

    /// The undo was refused; put it back where it was.
    pub fn undo_failed(&mut self, entry: JournalEntry) {
        self.done.push(entry);
    }

    /// The redo happened: it becomes undoable again.
    pub fn redo_succeeded(&mut self, entry: JournalEntry) {
        self.done.push(entry);
    }

    pub fn redo_failed(&mut self, entry: JournalEntry) {
        self.undone.push(entry);
    }

    pub fn len(&self) -> usize {
        self.done.len()
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> WorkspacePath {
        WorkspacePath::parse(raw).expect("valid wire path")
    }

    #[test]
    fn a_rename_undoes_by_moving_back_and_redoes_by_moving_forward() {
        let entry = JournalEntry::moved(
            vec![(path("a.ts"), path("b.ts"))],
            "Rename a.ts".to_string(),
        );
        assert_eq!(entry.undo, Step::Move(vec![(path("b.ts"), path("a.ts"))]));
        assert_eq!(entry.redo, Step::Move(vec![(path("a.ts"), path("b.ts"))]));
    }

    #[test]
    fn a_create_undoes_by_removing_and_a_copy_undoes_by_removing_the_copy() {
        let created = JournalEntry::create(path("new.ts"), EntryKind::File);
        assert_eq!(created.undo, Step::Remove(vec![path("new.ts")]));
        assert_eq!(
            created.redo,
            Step::Create(vec![(path("new.ts"), EntryKind::File)])
        );

        let copied = JournalEntry::copied(
            vec![(path("src/a.ts"), path("dst/a.ts"))],
            "Copy a.ts".to_string(),
        );
        // Only the copy is removed — never the original.
        assert_eq!(copied.undo, Step::Remove(vec![path("dst/a.ts")]));
    }

    #[test]
    fn recording_clears_the_redo_stack() {
        let mut journal = Journal::new();
        journal.record(JournalEntry::create(path("a"), EntryKind::File));
        let entry = journal.take_undo().expect("one entry");
        journal.undo_succeeded(entry);
        assert!(journal.peek_redo().is_some());

        journal.record(JournalEntry::create(path("b"), EntryKind::File));
        assert!(
            journal.peek_redo().is_none(),
            "a new operation makes the undone branch unreachable"
        );
    }

    #[test]
    fn a_refused_undo_leaves_the_stack_untouched() {
        let mut journal = Journal::new();
        journal.record(JournalEntry::create(path("a"), EntryKind::File));
        let entry = journal.take_undo().expect("one entry");
        journal.undo_failed(entry);

        assert_eq!(journal.len(), 1);
        assert!(journal.peek_redo().is_none());
        assert_eq!(
            journal.peek_undo().map(|e| e.label.as_str()),
            Some("Create a")
        );
    }

    #[test]
    fn the_stack_is_bounded_and_drops_the_oldest() {
        let mut journal = Journal::new();
        for n in 0..JOURNAL_LIMIT + 5 {
            journal.record(JournalEntry::create(
                path(&format!("f{n}.ts")),
                EntryKind::File,
            ));
        }
        assert_eq!(journal.len(), JOURNAL_LIMIT);
        assert_eq!(
            journal.peek_undo().map(|e| e.label.as_str()),
            Some(format!("Create f{}.ts", JOURNAL_LIMIT + 4).as_str())
        );
    }

    #[test]
    fn undo_then_redo_returns_the_entry_to_the_undo_stack() {
        let mut journal = Journal::new();
        journal.record(JournalEntry::moved(
            vec![(path("a"), path("b"))],
            "Rename a".to_string(),
        ));

        let undone = journal.take_undo().expect("one entry");
        journal.undo_succeeded(undone);
        assert!(journal.is_empty());

        let redone = journal.take_redo().expect("one redo");
        journal.redo_succeeded(redone);
        assert_eq!(journal.len(), 1);
        assert!(journal.peek_redo().is_none());
    }
}
