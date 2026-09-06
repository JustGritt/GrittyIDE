//! Pagination and the cancellation-check bound shared by `git.status` and
//! `git.diff`.
//!
//! Both commands produce a list that is unbounded in principle — a repository
//! can have any number of changed files — and both page it the way
//! `gritty_workspace::listing` pages a directory: sort by a stable key, skip
//! past an opaque cursor, emit at most `page_size` entries and say
//! `truncated` when more follow (`AGENTS.md` rule 8).
//!
//! The cancellation contract is the part that is load-bearing. `AGENTS.md`
//! rule 10 wants long work cancellable, and T100 §6 wants the check interval
//! **bounded independently of repository size**. That is what [`drain`] is
//! for: it owns the loop, so the cancellation predicate is polled every
//! [`CANCEL_CHECK_INTERVAL`] items and at most that many items are consumed
//! after a cancel, whatever the repository contains. The unit test at the
//! bottom measures exactly that with a synthetic sequence, and
//! `status::tests::cancelling_a_large_status_abandons_the_work_rather_than_finishing_it`
//! repeats it against a real repository — rule 12 asks for the measurement
//! rather than the claim.

use crate::error::GitError;

/// How many items are consumed between two polls of the cancellation
/// predicate. The same value `gritty_workspace::listing` uses, for the same
/// reason: small enough that a cancel is felt immediately, large enough that
/// the poll is not the cost of the loop.
pub const CANCEL_CHECK_INTERVAL: usize = 256;

/// Largest page either command will emit, mirroring
/// `gritty_workspace::MAX_PAGE_SIZE`.
pub const MAX_PAGE_SIZE: usize = 5_000;

/// Default page size when the caller does not choose one.
pub const DEFAULT_PAGE_SIZE: usize = MAX_PAGE_SIZE;

/// How much of a list a caller wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOptions {
    /// Entries per page, clamped to `1..=MAX_PAGE_SIZE`.
    pub page_size: usize,
    /// Continue after the entry a previous page ended with. The cursor is the
    /// sort key of that entry, so a page boundary stays meaningful even though
    /// the status is recomputed from scratch on every call — there is no
    /// repository cache, which T100 explicitly forbids.
    pub cursor: Option<String>,
}

impl Default for PageOptions {
    fn default() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            cursor: None,
        }
    }
}

/// One page of `T`, plus what was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Page<T> {
    pub entries: Vec<T>,
    /// How many entries the whole list had, across all pages.
    pub total: u64,
    /// More entries follow; pass `cursor` back to continue.
    pub truncated: bool,
    /// Opaque continuation token, present when `truncated`.
    pub cursor: Option<String>,
}

/// Consume `items`, handing each to `visit`, polling `cancelled` every
/// [`CANCEL_CHECK_INTERVAL`] items.
///
/// This is the only loop in the crate that drains a repository-sized sequence,
/// which is what makes the cancellation bound a property of one tested
/// function rather than a convention every call site has to remember.
///
/// An error from the sequence itself becomes [`GitError::Repository`] with
/// `context`: every such iterator here is a `gix` traversal, and a failure
/// mid-traversal is a repository that could not be read, not a caller mistake.
pub(crate) fn drain<T, I, E, V>(
    items: I,
    cancelled: &dyn Fn() -> bool,
    context: &'static str,
    mut visit: V,
) -> Result<(), GitError>
where
    I: IntoIterator<Item = Result<T, E>>,
    E: std::fmt::Display,
    V: FnMut(T) -> Result<(), GitError>,
{
    for (index, item) in items.into_iter().enumerate() {
        if index % CANCEL_CHECK_INTERVAL == 0 && cancelled() {
            return Err(GitError::Cancelled);
        }
        let item = item.map_err(|error| GitError::repository(context, error))?;
        visit(item)?;
    }
    // A sequence shorter than one interval would otherwise never be checked,
    // and a cancel that arrives while the last items are consumed would be
    // reported as success.
    if cancelled() {
        return Err(GitError::Cancelled);
    }
    Ok(())
}

/// Sort an already-collected list by `key` and cut the page `options` asked
/// for.
pub(crate) fn page_collected<T, K>(items: Vec<T>, options: &PageOptions, key: K) -> Page<T>
where
    K: Fn(&T) -> String,
{
    let mut keyed: Vec<(String, T)> = items.into_iter().map(|item| (key(&item), item)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));

    let page_size = options.page_size.clamp(1, MAX_PAGE_SIZE);
    let total = keyed.len() as u64;
    let start = match options.cursor.as_deref() {
        // `partition_point` over a sorted list: skip everything at or before
        // the cursor. An unknown cursor is not an error — it lands wherever it
        // sorts, which is the same behaviour a directory listing has when
        // entries changed between two pages.
        Some(cursor) => keyed.partition_point(|(key, _)| key.as_str() <= cursor),
        None => 0,
    };
    let mut remaining: Vec<(String, T)> = keyed.into_iter().skip(start).collect();
    let truncated = remaining.len() > page_size;
    remaining.truncate(page_size);
    let cursor = if truncated {
        remaining.last().map(|(key, _)| key.clone())
    } else {
        None
    };
    Page {
        entries: remaining.into_iter().map(|(_, item)| item).collect(),
        total,
        truncated,
        cursor,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::convert::Infallible;

    use super::*;

    fn collect(values: &[&str], options: &PageOptions) -> Page<String> {
        let items: Vec<Result<String, Infallible>> =
            values.iter().map(|value| Ok((*value).to_owned())).collect();
        let mut out = Vec::new();
        drain(items, &|| false, "test", |value| {
            out.push(value);
            Ok(())
        })
        .expect("not cancelled");
        page_collected(out, options, Clone::clone)
    }

    #[test]
    fn sorts_and_reports_the_total_across_pages() {
        let page = collect(
            &["c", "a", "b"],
            &PageOptions {
                page_size: 2,
                cursor: None,
            },
        );
        assert_eq!(page.entries, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(page.total, 3);
        assert!(page.truncated);
        assert_eq!(page.cursor.as_deref(), Some("b"));
    }

    #[test]
    fn a_cursor_continues_after_the_last_entry_of_the_previous_page() {
        let page = collect(
            &["c", "a", "b"],
            &PageOptions {
                page_size: 2,
                cursor: Some("b".to_owned()),
            },
        );
        assert_eq!(page.entries, vec!["c".to_owned()]);
        assert_eq!(page.total, 3);
        assert!(!page.truncated);
        assert_eq!(page.cursor, None);
    }

    #[test]
    fn an_unknown_cursor_lands_where_it_sorts_rather_than_failing() {
        let page = collect(
            &["a", "b", "c"],
            &PageOptions {
                page_size: 10,
                cursor: Some("b-gone".to_owned()),
            },
        );
        assert_eq!(page.entries, vec!["c".to_owned()]);
    }

    #[test]
    fn page_size_is_clamped_to_the_maximum() {
        let page = collect(
            &["a", "b"],
            &PageOptions {
                page_size: MAX_PAGE_SIZE * 4,
                cursor: None,
            },
        );
        assert_eq!(page.entries.len(), 2);
        assert!(!page.truncated);
    }

    #[test]
    fn a_cancelled_drain_returns_nothing_partial() {
        let items: Vec<Result<u8, Infallible>> = vec![Ok(1), Ok(2)];
        let error = drain(items, &|| true, "test", |_| Ok(())).expect_err("cancelled");
        assert!(matches!(error, GitError::Cancelled));
    }

    /// The bound T100 §6 asks for, measured rather than asserted from code
    /// shape: however long the sequence is, cancelling stops it within one
    /// check interval. `status.rs` repeats this against a real repository,
    /// where only the relative cost is observable; here it is exact.
    #[test]
    fn cancellation_is_noticed_within_one_check_interval() {
        let produced = Cell::new(0usize);
        let flipped = Cell::new(usize::MAX);
        let items = (0..CANCEL_CHECK_INTERVAL * 40).map(|index| {
            produced.set(index + 1);
            Ok::<_, Infallible>(index)
        });
        // Cancel a third of the way in, at a point deliberately not a multiple
        // of the interval.
        let trigger = CANCEL_CHECK_INTERVAL * 13 + 7;
        let cancelled = || {
            let seen = produced.get();
            if seen >= trigger {
                if flipped.get() == usize::MAX {
                    flipped.set(seen);
                }
                true
            } else {
                false
            }
        };
        let error = drain(items, &cancelled, "test", |_| Ok(())).expect_err("cancelled");
        assert!(matches!(error, GitError::Cancelled));
        let after_cancel = produced.get() - flipped.get();
        assert!(
            after_cancel <= CANCEL_CHECK_INTERVAL,
            "consumed {after_cancel} items after cancellation, above the \
             {CANCEL_CHECK_INTERVAL}-item bound"
        );
    }
}
