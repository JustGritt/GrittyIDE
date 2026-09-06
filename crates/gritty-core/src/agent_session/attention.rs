//! Attention notifications (T223): telling the user an agent needs them
//! without them having to be looking at the island.
//!
//! The whole of the policy is here and it is pure, because every rule in it is
//! a rule about *not* notifying and those are the ones worth testing:
//!
//! - **Edge-triggered.** A notification fires when a session moves *into*
//!   [`AgentActivity::Blocked`] or [`AgentActivity::Waiting`] from something
//!   else. The roster is polled, so a level-triggered notification would fire
//!   every interval for a session that is simply still waiting (rule 8).
//! - **Silent while the window is focused.** If the user is looking at Gritty,
//!   the island *is* the notification. Focus that is unknown counts as focused:
//!   a missed notification is better than one over the user's face.
//! - **Throttled**, at most one per session per [`NOTIFY_COOLDOWN`], and capped
//!   globally at [`NOTIFY_CAP_PER_WINDOW`] per [`NOTIFY_WINDOW`] so a machine
//!   with many agents cannot produce a burst.
//! - **Off by default.** Enabling it is a user action; nothing here is armed by
//!   starting the application.
//!
//! What crosses into the platform is [`AttentionAlert`], built here and shown
//! by the composition root through [`AttentionNotifier`]. The core never talks
//! to Tauri, and the renderer neither decides nor sends a notification: it is
//! raised from the same place that already observes the roster diff.
//!
//! Every string in an alert came out of an agent's transcript, so it is
//! untrusted input (rule 13). It is collapsed, stripped of control characters
//! and bounded here, and it is never a path the notification can open.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::wire::{AgentActivity, AgentProvider, AgentRoster, AgentSession};

/// At most one notification per session per this long. Long enough that a
/// session flapping between `WORKING` and `WAITING` cannot nag, short enough
/// that a genuinely new prompt half an hour later still reaches the user.
pub const NOTIFY_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// The window the global cap is measured over.
pub const NOTIFY_WINDOW: Duration = Duration::from_secs(60);

/// Most notifications raised in one [`NOTIFY_WINDOW`], across every session.
/// Ten agents finishing at once is one thing to look at, not ten.
pub const NOTIFY_CAP_PER_WINDOW: usize = 3;

/// Longest body a notification carries. Platforms truncate anyway; this is the
/// bound the core applies to untrusted text before it leaves.
pub const MAX_NOTIFICATION_BODY: usize = 140;

/// Whether the platform will show a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentNotificationPermission {
    /// It will.
    Granted,
    /// The user said no. The setting reports it and stays off; nothing retries.
    Denied,
    /// Nobody has been asked yet.
    Prompt,
}

/// `agents.notifications` and the answer to `agents.setNotifications`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentNotificationSettings {
    /// Off until the user turns it on, every run.
    pub enabled: bool,
    /// Whether this build can raise a notification at all. `false` means the
    /// setting is **absent** rather than broken: a platform without the
    /// notification plugin is not offered a switch that would do nothing.
    pub supported: bool,
    pub permission: AgentNotificationPermission,
}

/// `agents.setNotifications`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetAgentNotificationsRequest {
    pub enabled: bool,
}

/// One notification, already reduced to what the platform is handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionAlert {
    /// Which roster row it is about. Carried for logging and for a future
    /// surface that wants to select it; it is not a path or a handle.
    pub session_id: String,
    pub title: String,
    pub body: String,
}

/// The platform surface. Implemented by the composition root over
/// `tauri-plugin-notification`; tests record instead.
///
/// Deliberately not fallible: a notification that did not show is not worth
/// failing a roster scan for, and the implementation logs what it saw.
pub trait AttentionNotifier: Send + Sync + 'static {
    /// Whether notifications can be raised on this platform and in this build.
    fn supported(&self) -> bool;
    /// The platform's current answer, **without** asking the user.
    fn permission(&self) -> AgentNotificationPermission;
    /// Ask, prompting if the platform has never been asked. Called only from
    /// an explicit user action, never at startup.
    fn request_permission(&self) -> AgentNotificationPermission;
    fn show(&self, alert: &AttentionAlert);
}

/// The edge-triggering, the throttle and the cap, with no clock and no I/O of
/// its own so every rule in it is a test rather than a claim.
#[derive(Debug, Default)]
pub struct AttentionTracker {
    /// What each session was doing at the previous roster.
    previous: HashMap<String, AgentActivity>,
    /// When each session was last notified about.
    notified_at: HashMap<String, i64>,
    /// When the recent notifications were raised, for the global cap.
    recent: VecDeque<i64>,
    /// Whether any roster has been seen at all.
    seen: bool,
}

impl AttentionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one roster in and return what should be raised now.
    ///
    /// The transition table is updated **whichever way the settings go**, so
    /// turning notifications on does not fire for every session that was
    /// already blocked while they were off: those are not transitions, they are
    /// state the user has simply not been told about yet, and the island is
    /// where they belong.
    ///
    /// The very first roster of a process records and notifies nothing. Gritty
    /// has just noticed these sessions; their state did not change, our
    /// knowledge did, and a launch that fires three notifications for work that
    /// has been sitting there for an hour is noise. A session that appears
    /// *after* that is a real arrival and does notify.
    pub fn advance(
        &mut self,
        roster: &AgentRoster,
        now_ms: i64,
        enabled: bool,
        focused: bool,
    ) -> Vec<AttentionAlert> {
        let first = !self.seen;
        self.seen = true;
        let mut candidates: Vec<&AgentSession> = Vec::new();
        for session in &roster.sessions {
            let before = self.previous.get(&session.id).copied();
            let entered = before != Some(session.activity) && session.activity.needs_you();
            // A first sighting has no "something else" to have come from; on
            // any roster after the first, arriving already needing you is one.
            if entered && !(first && before.is_none()) {
                candidates.push(session);
            }
        }
        // The roster is the whole population, so rebuilding rather than merging
        // is what keeps this bounded by what is running.
        self.previous = roster
            .sessions
            .iter()
            .map(|session| (session.id.clone(), session.activity))
            .collect();
        self.prune(roster, now_ms);

        // Suppressed, not deferred: the reason for staying silent is that the
        // user is either looking at the island or has not asked for this, and
        // neither becomes a reason to notify later.
        if !enabled || focused {
            return Vec::new();
        }

        let mut alerts = Vec::new();
        for session in candidates {
            if self.recent.len() >= NOTIFY_CAP_PER_WINDOW {
                tracing::debug!(
                    dropped = 1,
                    "agent attention notification suppressed by the global cap"
                );
                break;
            }
            if let Some(last) = self.notified_at.get(&session.id)
                && now_ms.saturating_sub(*last) < millis(NOTIFY_COOLDOWN)
            {
                continue;
            }
            self.notified_at.insert(session.id.clone(), now_ms);
            self.recent.push_back(now_ms);
            alerts.push(alert_for(session));
        }
        alerts
    }

    /// Whether the caller has just been told about this session — exposed so a
    /// test can assert the throttle rather than infer it from an empty vector.
    pub fn notified_at(&self, session_id: &str) -> Option<i64> {
        self.notified_at.get(session_id).copied()
    }

    /// Drop what neither the cap nor the throttle can still refer to. Without
    /// this, a long-lived process accumulates a row per session it has ever
    /// seen.
    fn prune(&mut self, roster: &AgentRoster, now_ms: i64) {
        while self
            .recent
            .front()
            .is_some_and(|at| now_ms.saturating_sub(*at) >= millis(NOTIFY_WINDOW))
        {
            self.recent.pop_front();
        }
        let cooldown = millis(NOTIFY_COOLDOWN);
        self.notified_at.retain(|id, at| {
            now_ms.saturating_sub(*at) < cooldown || roster.sessions.iter().any(|s| &s.id == id)
        });
    }
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// What one session's notification says.
///
/// The title names the provider and why it stopped; the body names the task and
/// the working directory, which together are what tells the user *which*
/// terminal to go to. Both are bounded and neither is ever resolved.
fn alert_for(session: &AgentSession) -> AttentionAlert {
    let title = match session.activity {
        AgentActivity::Blocked => format!("{} is waiting for permission", label(session.provider)),
        _ => format!("{} is waiting for you", label(session.provider)),
    };
    let task = session
        .title
        .as_deref()
        .or(session.last_prompt.as_deref())
        .or(session.last_reply.as_deref());
    let body = match (task, session.root.as_deref()) {
        (Some(task), Some(root)) => format!("{task} · {root}"),
        (Some(task), None) => task.to_owned(),
        (None, Some(root)) => root.to_owned(),
        (None, None) => "No transcript yet".to_owned(),
    };
    AttentionAlert {
        session_id: session.id.clone(),
        title,
        body: bounded(&body),
    }
}

const fn label(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::ClaudeCode => "Claude Code",
    }
}

/// Untrusted text, made safe to hand to a platform notification: control
/// characters and newlines collapsed to spaces, then bounded by characters.
fn bounded(text: &str) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= MAX_NOTIFICATION_BODY {
        return collapsed;
    }
    let head: String = collapsed
        .chars()
        .take(MAX_NOTIFICATION_BODY.saturating_sub(1))
        .collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_session::wire::{AgentActivitySource, AgentEvidence};

    fn session(id: &str, activity: AgentActivity) -> AgentSession {
        AgentSession {
            id: id.to_owned(),
            provider: AgentProvider::ClaudeCode,
            activity,
            activity_source: AgentActivitySource::Process,
            evidence: AgentEvidence::Both,
            title: Some("Wire the attention queue".to_owned()),
            last_prompt: None,
            last_reply: None,
            root: Some("~/code/gritty".to_owned()),
            root_name: Some("gritty".to_owned()),
            in_workspace: true,
            git_branch: None,
            model: None,
            tool: None,
            pid: Some(4),
            tty: None,
            last_active_at: None,
            pending: None,
        }
    }

    fn roster(sessions: Vec<AgentSession>) -> AgentRoster {
        AgentRoster {
            needs_you: u32::try_from(sessions.iter().filter(|s| s.activity.needs_you()).count())
                .unwrap(),
            sessions,
            process_evidence: true,
            recent_answers: Vec::new(),
        }
    }

    /// The first roster is what Gritty has just noticed, not what changed.
    fn warmed() -> AttentionTracker {
        let mut tracker = AttentionTracker::new();
        assert!(
            tracker
                .advance(&roster(Vec::new()), 0, true, false)
                .is_empty()
        );
        tracker
    }

    #[test]
    fn a_transition_into_blocked_fires_once_and_not_again_while_it_persists() {
        let mut tracker = warmed();
        let working = roster(vec![session("s-1", AgentActivity::Working)]);
        assert!(tracker.advance(&working, 1_000, true, false).is_empty());

        let blocked = roster(vec![session("s-1", AgentActivity::Blocked)]);
        let alerts = tracker.advance(&blocked, 2_000, true, false);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].session_id, "s-1");
        assert!(
            alerts[0].title.contains("waiting for permission"),
            "{alerts:?}"
        );
        assert_eq!(alerts[0].body, "Wire the attention queue · ~/code/gritty");

        // Still blocked on the next poll, and the one after: the roster is
        // polled, so this is exactly the level-triggered case rule 8 forbids.
        assert!(tracker.advance(&blocked, 3_000, true, false).is_empty());
        assert!(tracker.advance(&blocked, 4_000, true, false).is_empty());
    }

    #[test]
    fn leaving_the_state_does_not_notify_and_re_entering_it_does() {
        let mut tracker = warmed();
        let waiting = roster(vec![session("s-1", AgentActivity::Waiting)]);
        assert_eq!(tracker.advance(&waiting, 1_000, true, false).len(), 1);

        let working = roster(vec![session("s-1", AgentActivity::Working)]);
        assert!(
            tracker.advance(&working, 2_000, true, false).is_empty(),
            "a transition *out* is not a reason to interrupt"
        );

        // Back to waiting inside the cooldown: still nothing.
        assert!(tracker.advance(&waiting, 3_000, true, false).is_empty());
        // And past it, the same transition is worth telling the user about.
        let later = 3_000 + millis(NOTIFY_COOLDOWN);
        assert!(tracker.advance(&working, later, true, false).is_empty());
        assert_eq!(tracker.advance(&waiting, later + 1, true, false).len(), 1);
    }

    #[test]
    fn a_focused_window_is_silent_and_the_transition_is_not_replayed_afterwards() {
        let mut tracker = warmed();
        let blocked = roster(vec![session("s-1", AgentActivity::Blocked)]);
        assert!(
            tracker.advance(&blocked, 1_000, true, true).is_empty(),
            "the island is the notification while the window is focused"
        );
        // Unfocusing does not resurrect it: the state persisted, so there is
        // no transition left to report.
        assert!(tracker.advance(&blocked, 2_000, true, false).is_empty());
        assert_eq!(tracker.notified_at("s-1"), None);
    }

    #[test]
    fn nothing_fires_while_the_setting_is_off_and_enabling_it_does_not_backfill() {
        let mut tracker = warmed();
        let blocked = roster(vec![session("s-1", AgentActivity::Blocked)]);
        assert!(tracker.advance(&blocked, 1_000, false, false).is_empty());
        // Turned on afterwards: the session is still blocked, which is not a
        // transition, so it stays in the island rather than arriving as a
        // notification about something that happened before the user asked.
        assert!(tracker.advance(&blocked, 2_000, true, false).is_empty());
    }

    #[test]
    fn the_global_cap_collapses_a_burst_and_recovers_after_the_window() {
        let mut tracker = warmed();
        let idle: Vec<AgentSession> = (0..5)
            .map(|index| session(&format!("s-{index}"), AgentActivity::Working))
            .collect();
        tracker.advance(&roster(idle), 1_000, true, false);

        let all_blocked: Vec<AgentSession> = (0..5)
            .map(|index| session(&format!("s-{index}"), AgentActivity::Blocked))
            .collect();
        let alerts = tracker.advance(&roster(all_blocked.clone()), 2_000, true, false);
        assert_eq!(
            alerts.len(),
            NOTIFY_CAP_PER_WINDOW,
            "five at once is one thing to look at, not five"
        );
        // The two that were capped are not owed a notification later: they
        // never transitioned again.
        assert!(
            tracker
                .advance(&roster(all_blocked), 2_100, true, false)
                .is_empty()
        );

        // A fresh transition once the window has passed is allowed again.
        let after = 2_000 + millis(NOTIFY_WINDOW) + millis(NOTIFY_COOLDOWN);
        let one = vec![session("s-9", AgentActivity::Blocked)];
        assert_eq!(tracker.advance(&roster(one), after, true, false).len(), 1);
    }

    #[test]
    fn a_session_that_arrives_already_blocked_after_the_first_roster_notifies() {
        let mut tracker = warmed();
        tracker.advance(
            &roster(vec![session("s-1", AgentActivity::Working)]),
            1_000,
            true,
            false,
        );
        let arrived = roster(vec![
            session("s-1", AgentActivity::Working),
            session("s-2", AgentActivity::Blocked),
        ]);
        let alerts = tracker.advance(&arrived, 2_000, true, false);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].session_id, "s-2");
    }

    #[test]
    fn the_very_first_roster_of_a_process_notifies_about_nothing() {
        let mut tracker = AttentionTracker::new();
        let blocked = roster(vec![
            session("s-1", AgentActivity::Blocked),
            session("s-2", AgentActivity::Waiting),
        ]);
        assert!(
            tracker.advance(&blocked, 1_000, true, false).is_empty(),
            "launching Gritty is not something that happened to the agents"
        );
    }

    #[test]
    fn the_body_is_bounded_stripped_and_never_a_path_to_open() {
        let mut tracker = warmed();
        let mut noisy = session("s-1", AgentActivity::Waiting);
        noisy.title = Some(format!("line one\nline\ttwo {}", "x".repeat(300)));
        noisy.root = Some("~/code/gritty".to_owned());
        let alerts = tracker.advance(&roster(vec![noisy]), 1_000, true, false);
        let body = &alerts[0].body;
        assert_eq!(body.chars().count(), MAX_NOTIFICATION_BODY);
        assert!(body.ends_with('…'), "{body}");
        assert!(!body.contains('\n') && !body.contains('\t'), "{body}");
    }

    #[test]
    fn a_session_with_no_transcript_still_says_where_it_is() {
        let mut tracker = warmed();
        let mut bare = session("s-1", AgentActivity::Waiting);
        bare.title = None;
        let alerts = tracker.advance(&roster(vec![bare]), 1_000, true, false);
        assert_eq!(alerts[0].body, "~/code/gritty");

        let mut nowhere = session("s-2", AgentActivity::Waiting);
        nowhere.title = None;
        nowhere.root = None;
        let alerts = tracker.advance(&roster(vec![nowhere]), 2_000, true, false);
        assert_eq!(alerts[0].body, "No transcript yet");
    }

    #[test]
    fn ended_and_idle_sessions_are_never_worth_interrupting_for() {
        let mut tracker = warmed();
        let running = roster(vec![
            session("s-1", AgentActivity::Working),
            session("s-2", AgentActivity::Working),
        ]);
        tracker.advance(&running, 1_000, true, false);
        let quiet = roster(vec![
            session("s-1", AgentActivity::Idle),
            session("s-2", AgentActivity::Ended),
        ]);
        assert!(tracker.advance(&quiet, 2_000, true, false).is_empty());
    }

    #[test]
    fn sessions_that_left_the_roster_are_forgotten() {
        let mut tracker = warmed();
        tracker.advance(
            &roster(vec![session("s-1", AgentActivity::Working)]),
            1_000,
            true,
            false,
        );
        tracker.advance(
            &roster(vec![session("s-1", AgentActivity::Blocked)]),
            2_000,
            true,
            false,
        );
        assert_eq!(tracker.notified_at("s-1"), Some(2_000));
        let long_after = 2_000 + millis(NOTIFY_COOLDOWN);
        tracker.advance(&roster(Vec::new()), long_after, true, false);
        assert_eq!(tracker.notified_at("s-1"), None);
        assert!(tracker.previous.is_empty());
    }
}
