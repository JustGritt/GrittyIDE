//! Answering a permission prompt (T335, ADR-020).
//!
//! The rest of this module observes. This file is the one place that acts, and
//! it acts only on something a person did: `answer` writes a verdict the hook
//! is waiting for, and nothing here ever decides on its own that an agent may
//! proceed — except where the same person already said so, in a grant they can
//! see and take back.
//!
//! **Everything is in memory.** Grants live for as long as Gritty does, and a
//! restart drops them. That is not an omission: a standing "yes" that outlived
//! the process that took it would be authority nobody remembers granting, and
//! losing it means the user is asked again, which is the direction this kind of
//! state should fail in.
//!
//! The adapter/domain split holds, as it does everywhere else here
//! (ADR-005): `gritty_agents::hook::HookAsk` is what the hook wrote,
//! [`AgentPermissionRequest`] is what Gritty means by it, and
//! `gritty_agents::verdict` is the file the answer travels in.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use gritty_agents::hook::{AskVerb, HookAsk};
use gritty_agents::verdict::{Behavior, VERDICT_VERSION, Verdict};

use crate::protocol::time::Timestamp;

use super::wire::{
    AgentPermissionAnswer, AgentPermissionGrant, AgentPermissionRequest, AgentPermissionVerb,
    AgentPermissionVerdict,
};

/// How many answers the roster carries for the undo affordance.
///
/// Small on purpose. This is "what did I just do", not a log; anything longer
/// would be a history the user did not ask for and cannot act on.
pub const MAX_RECENT_ANSWERS: usize = 8;

/// A ceiling on standing grants, so a session that is answered `AllowForRun`
/// repeatedly cannot grow this without bound (rule 8's spirit).
pub const MAX_GRANTS: usize = 64;

impl AgentPermissionVerb {
    pub(super) fn from_adapter(verb: AskVerb) -> Self {
        match verb {
            AskVerb::WriteFile => Self::WriteFile,
            AskVerb::RunCommand => Self::RunCommand,
            AskVerb::InstallDependency => Self::InstallDependency,
            AskVerb::Other => Self::Other,
        }
    }
}

/// One adapter ask to one domain request.
pub(super) fn translate(
    session_id: &str,
    ask: &HookAsk,
    since_ms: i64,
    shorten: impl Fn(&str) -> String,
) -> AgentPermissionRequest {
    let verb = AgentPermissionVerb::from_adapter(ask.verb);
    // Shortened here rather than in the hook, for the same reason a session's
    // root is: the renderer has no filesystem and no home directory, so display
    // is the core's job (ADR-010).
    //
    // **Before the scope is derived, not after.** A grant is matched against
    // later targets by prefix, so a scope taken from the raw path would never
    // match a target the user was shown — "allow for this run" would appear to
    // work and quietly ask again every time.
    let target = shorten(&ask.target);
    AgentPermissionRequest {
        ask_id: ask.ask_id.clone(),
        session_id: session_id.to_owned(),
        verb,
        suggested_scope: scope_of(verb, &target),
        target,
        detail: ask.detail.clone(),
        diff: ask.diff.clone(),
        added: ask.added,
        removed: ask.removed,
        outside_root: ask.outside_root,
        sensitive: ask.sensitive,
        asked_at: u64::try_from(since_ms)
            .map(Timestamp::from_millis)
            .unwrap_or_else(|_| Timestamp::from_millis(0)),
    }
}

/// What "for this run" would cover, said in terms the user can check.
///
/// A write is scoped to the directory it lands in — the artboard's "allow
/// writes in `crates/terminal` for this run" — and a command to the program it
/// runs. Where neither can be derived, the answer is `None` and the middle
/// button is simply not offered: an unscoped standing grant is a blanket yes,
/// and this is not the surface to hand one out from.
pub(super) fn scope_of(verb: AgentPermissionVerb, target: &str) -> Option<String> {
    match verb {
        AgentPermissionVerb::WriteFile => {
            let (parent, _) = target.rsplit_once('/')?;
            (!parent.is_empty()).then(|| parent.to_owned())
        }
        // An install is scoped to the installer, not to the package: "yes to
        // pnpm add" is a decision about a tool, and it is at least a decision
        // the user can read back.
        AgentPermissionVerb::RunCommand | AgentPermissionVerb::InstallDependency => {
            let program = target.split_whitespace().next()?;
            (!program.is_empty()).then(|| program.to_owned())
        }
        // Nothing is known about what the tool would touch, so nothing can be
        // scoped. `Allow once` is the only honest standing offer.
        AgentPermissionVerb::Other => None,
    }
}

/// Whether a grant covers an ask.
///
/// Prefix matching on a path boundary for a write, exact match on the program
/// for a command. Deliberately narrow: this decides that a person is *not*
/// asked, so where it is unsure it must say no and let them answer.
fn covers(grant: &AgentPermissionGrant, request: &AgentPermissionRequest) -> bool {
    if grant.session_id != request.session_id || grant.verb != request.verb {
        return false;
    }
    match request.verb {
        AgentPermissionVerb::WriteFile => {
            let scope = grant.scope.trim_end_matches('/');
            request
                .target
                .strip_prefix(scope)
                .is_some_and(|rest| rest.starts_with('/'))
        }
        AgentPermissionVerb::RunCommand | AgentPermissionVerb::InstallDependency => {
            request.target.split_whitespace().next() == Some(grant.scope.as_str())
        }
        AgentPermissionVerb::Other => false,
    }
}

/// What answering did, so the caller knows what to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Answered {
    /// The verdict was written where the waiting hook will find it.
    Written(AgentPermissionAnswer),
    /// There is no such ask. Either it was already answered, or the user
    /// answered in the terminal first and it vanished — both normal, and
    /// neither is an error worth an error envelope's alarm.
    Unknown,
}

/// Everything the core remembers about permissions, which is deliberately
/// little: what is being asked, what was granted, and what was just answered.
#[derive(Debug, Default)]
pub(super) struct Permissions {
    /// Asks the core has seen but not yet answered, by ask id.
    open: Mutex<HashMap<String, AgentPermissionRequest>>,
    grants: Mutex<Vec<AgentPermissionGrant>>,
    recent: Mutex<Vec<AgentPermissionAnswer>>,
    /// Monotonic, per process. Grant ids are not durable — nothing outside this
    /// process ever sees one — so they need to be unique, not unguessable.
    next_grant: Mutex<u64>,
}

impl Permissions {
    /// Replace what is open with what this scan found, and report the asks a
    /// standing grant should answer without the user seeing them.
    ///
    /// Returns the requests to auto-allow. The caller writes those verdicts,
    /// because writing a file is I/O and this type holds a lock.
    pub(super) fn observe(
        &self,
        found: Vec<AgentPermissionRequest>,
    ) -> Vec<(AgentPermissionRequest, String)> {
        let mut auto = Vec::new();
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let mut grants = self.grants.lock().unwrap_or_else(PoisonError::into_inner);
        let known: Vec<String> = found.iter().map(|request| request.ask_id.clone()).collect();
        for request in found {
            if open.contains_key(&request.ask_id) {
                continue;
            }
            if let Some(grant) = grants.iter_mut().find(|grant| covers(grant, &request)) {
                grant.used = grant.used.saturating_add(1);
                auto.push((request, grant.id.clone()));
                continue;
            }
            open.insert(request.ask_id.clone(), request);
        }
        // An ask that is no longer on disk was answered somewhere else — in the
        // terminal, or by the session ending. Forgetting it is the whole of
        // what has to happen: ADR-020 says both answer paths are live at once
        // and the first one wins.
        open.retain(|ask_id, _| known.contains(ask_id));
        auto.iter()
            .map(|(request, grant)| (request.clone(), grant.clone()))
            .collect()
    }

    /// The ask a session is stopped at, for the roster.
    pub(super) fn pending_for(&self, session_id: &str) -> Option<AgentPermissionRequest> {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .find(|request| request.session_id == session_id)
            .cloned()
    }

    /// Take one ask off the open list and record the answer.
    ///
    /// Returns the verdict to write alongside the record, because — as with
    /// [`Self::observe`] — the file write belongs outside the lock.
    pub(super) fn answer(
        &self,
        ask_id: &str,
        verdict: AgentPermissionVerdict,
        now: Timestamp,
    ) -> (Answered, Option<Verdict>) {
        let Some(request) = self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(ask_id)
        else {
            return (Answered::Unknown, None);
        };

        let grant_id = (verdict == AgentPermissionVerdict::AllowForRun)
            .then(|| request.suggested_scope.clone())
            .flatten()
            .map(|scope| self.grant(&request, scope, now));

        let answer = AgentPermissionAnswer {
            ask_id: request.ask_id.clone(),
            session_id: request.session_id.clone(),
            verb: request.verb,
            target: request.target.clone(),
            verdict,
            answered_at: now,
            grant_id,
        };
        self.remember(answer.clone());

        let behavior = match verdict {
            AgentPermissionVerdict::AllowOnce | AgentPermissionVerdict::AllowForRun => {
                Behavior::Allow
            }
            AgentPermissionVerdict::Deny => Behavior::Deny,
        };
        (
            Answered::Written(answer),
            Some(Verdict {
                version: VERDICT_VERSION,
                ask_id: request.ask_id,
                behavior,
                // Measured to reach the model as feedback it reasons about
                // (ADR-020), so it says who decided rather than only that
                // something did.
                message: (behavior == Behavior::Deny)
                    .then(|| "Denied in Gritty by the user.".to_owned()),
            }),
        )
    }

    /// The verdict a standing grant produces, with no user in the loop.
    pub(super) fn auto_verdict(ask_id: &str) -> Verdict {
        Verdict {
            version: VERDICT_VERSION,
            ask_id: ask_id.to_owned(),
            behavior: Behavior::Allow,
            message: None,
        }
    }

    fn grant(&self, request: &AgentPermissionRequest, scope: String, now: Timestamp) -> String {
        let mut next = self
            .next_grant
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *next += 1;
        let id = format!("grant-{next}");
        drop(next);

        let mut grants = self.grants.lock().unwrap_or_else(PoisonError::into_inner);
        // Oldest out first. A grant nobody has used in a long session is the
        // one whose loss costs least, and the bound has to be somewhere.
        if grants.len() >= MAX_GRANTS {
            grants.remove(0);
        }
        grants.push(AgentPermissionGrant {
            id: id.clone(),
            session_id: request.session_id.clone(),
            verb: request.verb,
            scope,
            granted_at: now,
            used: 0,
        });
        id
    }

    fn remember(&self, answer: AgentPermissionAnswer) {
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        recent.insert(0, answer);
        recent.truncate(MAX_RECENT_ANSWERS);
    }

    pub(super) fn recent(&self) -> Vec<AgentPermissionAnswer> {
        self.recent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn grants(&self) -> Vec<AgentPermissionGrant> {
        self.grants
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Take back a standing grant. What Undo calls, and what the standing-rules
    /// row calls.
    ///
    /// `false` when there is no such grant, which includes revoking one twice.
    pub(super) fn revoke(&self, grant_id: &str) -> bool {
        let mut grants = self.grants.lock().unwrap_or_else(PoisonError::into_inner);
        let before = grants.len();
        grants.retain(|grant| grant.id != grant_id);
        let removed = grants.len() != before;
        drop(grants);
        if removed {
            // The answer stops offering an undo it can no longer perform.
            let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
            for answer in recent.iter_mut() {
                if answer.grant_id.as_deref() == Some(grant_id) {
                    answer.grant_id = None;
                }
            }
        }
        removed
    }

    /// Drop everything belonging to sessions that are no longer running.
    ///
    /// A grant is "for this run", and this is what makes that true: when the
    /// session goes, its standing yes goes with it.
    pub(super) fn retain_sessions(&self, live: &[String]) {
        self.grants
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|grant| live.contains(&grant.session_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(session: &str, verb: AgentPermissionVerb, target: &str) -> AgentPermissionRequest {
        AgentPermissionRequest {
            ask_id: format!("ask-{session}-{target}"),
            session_id: session.to_owned(),
            verb,
            target: target.to_owned(),
            detail: None,
            diff: Vec::new(),
            added: 0,
            removed: 0,
            outside_root: false,
            sensitive: false,
            asked_at: Timestamp::from_millis(1_000),
            suggested_scope: scope_of(verb, target),
        }
    }

    fn now() -> Timestamp {
        Timestamp::from_millis(2_000)
    }

    #[test]
    fn a_write_is_scoped_to_its_directory_and_a_command_to_its_program() {
        assert_eq!(
            scope_of(
                AgentPermissionVerb::WriteFile,
                "~/p/crates/terminal/src/pty.rs"
            ),
            Some("~/p/crates/terminal/src".to_owned())
        );
        assert_eq!(
            scope_of(AgentPermissionVerb::RunCommand, "cargo publish --dry-run"),
            Some("cargo".to_owned())
        );
        // Nothing is known about what it would touch, so nothing is offered.
        assert_eq!(scope_of(AgentPermissionVerb::Other, "WebFetch"), None);
        assert_eq!(scope_of(AgentPermissionVerb::WriteFile, "pty.rs"), None);
    }

    #[test]
    fn allowing_once_grants_nothing_and_the_same_ask_comes_back() {
        let permissions = Permissions::default();
        let ask = request(
            "s1",
            AgentPermissionVerb::WriteFile,
            "~/p/crates/terminal/src/pty.rs",
        );
        permissions.observe(vec![ask.clone()]);
        let (answered, verdict) =
            permissions.answer(&ask.ask_id, AgentPermissionVerdict::AllowOnce, now());
        assert!(matches!(answered, Answered::Written(_)));
        assert_eq!(verdict.expect("a verdict").behavior, Behavior::Allow);
        assert!(permissions.grants().is_empty());

        // A later ask in the same directory still reaches the user.
        let next = request(
            "s1",
            AgentPermissionVerb::WriteFile,
            "~/p/crates/terminal/src/grid.rs",
        );
        assert!(permissions.observe(vec![next.clone()]).is_empty());
        assert_eq!(permissions.pending_for("s1"), Some(next));
    }

    #[test]
    fn allowing_for_the_run_answers_the_next_matching_ask_without_the_user() {
        let permissions = Permissions::default();
        let ask = request(
            "s1",
            AgentPermissionVerb::WriteFile,
            "~/p/crates/terminal/src/pty.rs",
        );
        permissions.observe(vec![ask.clone()]);
        permissions.answer(&ask.ask_id, AgentPermissionVerdict::AllowForRun, now());

        let next = request(
            "s1",
            AgentPermissionVerb::WriteFile,
            "~/p/crates/terminal/src/grid.rs",
        );
        let auto = permissions.observe(vec![next.clone()]);
        assert_eq!(auto.len(), 1, "answered by the grant");
        assert_eq!(auto[0].0.ask_id, next.ask_id);
        assert_eq!(
            permissions.pending_for("s1"),
            None,
            "and it never reaches the user"
        );
        assert_eq!(permissions.grants()[0].used, 1, "and the grant says so");
    }

    #[test]
    fn a_grant_does_not_reach_past_its_own_scope_or_session() {
        let permissions = Permissions::default();
        let ask = request(
            "s1",
            AgentPermissionVerb::WriteFile,
            "~/p/crates/terminal/src/pty.rs",
        );
        permissions.observe(vec![ask.clone()]);
        permissions.answer(&ask.ask_id, AgentPermissionVerdict::AllowForRun, now());

        for other in [
            // A sibling directory whose name merely starts the same way.
            request(
                "s1",
                AgentPermissionVerb::WriteFile,
                "~/p/crates/terminal/src-gen/x.rs",
            ),
            // Another session entirely.
            request(
                "s2",
                AgentPermissionVerb::WriteFile,
                "~/p/crates/terminal/src/grid.rs",
            ),
            // The same place, a different kind of thing.
            request(
                "s1",
                AgentPermissionVerb::RunCommand,
                "rm ~/p/crates/terminal/src/pty.rs",
            ),
        ] {
            let session = other.session_id.clone();
            assert!(
                permissions.observe(vec![other.clone()]).is_empty(),
                "{} must still be asked",
                other.target
            );
            assert!(permissions.pending_for(&session).is_some());
        }
    }

    #[test]
    fn revoking_a_grant_brings_the_question_back_and_undo_stops_being_offered() {
        let permissions = Permissions::default();
        let ask = request("s1", AgentPermissionVerb::RunCommand, "cargo test -p x");
        permissions.observe(vec![ask.clone()]);
        permissions.answer(&ask.ask_id, AgentPermissionVerdict::AllowForRun, now());
        let grant_id = permissions.recent()[0]
            .grant_id
            .clone()
            .expect("an undoable answer");

        assert!(permissions.revoke(&grant_id));
        assert!(permissions.grants().is_empty());
        assert_eq!(
            permissions.recent()[0].grant_id,
            None,
            "an answer stops offering an undo it can no longer perform"
        );
        assert!(
            !permissions.revoke(&grant_id),
            "revoking twice is not an error"
        );

        let next = request("s1", AgentPermissionVerb::RunCommand, "cargo build");
        assert!(permissions.observe(vec![next]).is_empty());
        assert!(permissions.pending_for("s1").is_some());
    }

    #[test]
    fn answering_an_ask_that_is_gone_is_not_an_error() {
        // The user answered in the terminal first, or answered twice. ADR-020
        // says both answer paths are live at once and the first one wins.
        let permissions = Permissions::default();
        let (answered, verdict) =
            permissions.answer("nobody", AgentPermissionVerdict::AllowOnce, now());
        assert_eq!(answered, Answered::Unknown);
        assert!(verdict.is_none());
    }

    #[test]
    fn an_ask_that_vanishes_from_disk_is_forgotten() {
        let permissions = Permissions::default();
        let ask = request("s1", AgentPermissionVerb::RunCommand, "cargo test");
        permissions.observe(vec![ask]);
        assert!(permissions.pending_for("s1").is_some());
        permissions.observe(Vec::new());
        assert_eq!(permissions.pending_for("s1"), None);
    }

    #[test]
    fn a_grant_dies_with_the_session_that_earned_it() {
        let permissions = Permissions::default();
        let ask = request("s1", AgentPermissionVerb::RunCommand, "cargo test");
        permissions.observe(vec![ask.clone()]);
        permissions.answer(&ask.ask_id, AgentPermissionVerdict::AllowForRun, now());
        assert_eq!(permissions.grants().len(), 1);

        permissions.retain_sessions(&["s2".to_owned()]);
        assert!(
            permissions.grants().is_empty(),
            "\"for this run\" means for this run"
        );
    }

    #[test]
    fn the_recent_answers_stay_bounded() {
        let permissions = Permissions::default();
        for n in 0..MAX_RECENT_ANSWERS * 2 {
            let ask = request(
                "s1",
                AgentPermissionVerb::RunCommand,
                &format!("cargo test {n}"),
            );
            permissions.observe(vec![ask.clone()]);
            permissions.answer(&ask.ask_id, AgentPermissionVerdict::Deny, now());
        }
        assert_eq!(permissions.recent().len(), MAX_RECENT_ANSWERS);
        assert!(
            permissions.recent()[0]
                .target
                .ends_with(&format!("{}", MAX_RECENT_ANSWERS * 2 - 1)),
            "newest first"
        );
    }
}
