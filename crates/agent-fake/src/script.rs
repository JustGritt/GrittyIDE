//! The fake's step vocabulary, and the scripts written in it.
//!
//! [`FakeStep`] is what a caller writes; [`FakeAction`] is what the driver
//! reports, which is every step plus the four things that happen *to* a run
//! rather than in its script — an answer arriving out of band, a cancellation,
//! and the two ways a run is orphaned.
//!
//! Neither is `gritty_core::AgentRunEvent`, and the difference is the point
//! (task §9, hard rule 4). A script says `askPermission`; the protocol says
//! `PERMISSION_REQUESTED` **and** `RUN_BLOCKED`. A script says `present`; the
//! protocol says nothing at all and moves a state. A script never says
//! `RUN_STARTED`, because starting is something the manager did, not something
//! the agent reported. That gap is the translation seam T310's real adapter
//! needs, and a fake that emitted protocol events directly would prove nothing
//! about it.
//!
//! [`FakeAction::run_events`] publishes the mapping as the core's own wire
//! spellings so the two lanes can be checked against each other. It is a
//! specification, not an implementation: the implementation belongs to
//! `agent_run`.

use crate::error::FakeError;

/// How many steps [`FakeScriptPreset::Chatty`] emits.
///
/// Comfortably more than the core's `MAX_RUN_EVENTS` (512 at the time of
/// writing), so that T300-CORE can overflow its ring and watch `dropped` come
/// back non-zero without writing a script by hand. Named rather than literal,
/// and if the core's ring grows past this, this grows with it.
pub const CHATTY_STEPS: usize = 600;

/// One thing a scripted agent does.
///
/// Ordered by a script and advanced one at a time; nothing here happens because
/// time passed. Every string is the fake's *fiction* and is left exactly as
/// written — bounding and whitespace-collapsing are the core's job, at the one
/// ingest seam, and a fake that tidied its own output would be proving the
/// wrong thing (rule 13, and [`FakeScriptPreset::Hostile`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeStep {
    /// The user asked for something and the run begins working on it.
    Prompt { text: String },
    /// Working without doing anything observable yet.
    Think,
    /// Reading a file. The path is a raw string on purpose: whether it resolves
    /// inside the workspace is the core's question, and answering it is how
    /// `AgentRunPath::inside` / `::outside` gets decided.
    Read { path: String },
    /// Searching the workspace. Changes the activity; reports no event, because
    /// the protocol has no variant for it.
    Search { query: String },
    /// Wanting to write. Separate from [`Self::Write`] because the interception
    /// point T330 needs is exactly between the two.
    ProposeWrite { path: String },
    /// Having written. Legal only after a [`Self::ProposeWrite`] of the same
    /// path — the pairing is what makes an out-of-order script detectable.
    Write { path: String },
    /// Running a command. Shown, never run: this crate spawns no process.
    Command {
        command: String,
        cwd: Option<String>,
    },
    /// Running the test suite. A command whose activity is `TESTING` rather
    /// than `EXECUTING`; otherwise identical to [`Self::Command`].
    Test { command: String },
    /// How the command ended. Legal only while one is outstanding.
    CommandDone { ok: bool, exit_code: Option<i32> },
    /// The run answered the prompt.
    Reply { text: String },
    /// The run wants permission for something and stops until it is answered.
    AskPermission {
        tool: String,
        detail: Option<String>,
        reason: String,
    },
    /// The run needs a question answered and stops until it is.
    AskDecision { question: String },
    /// The turn is over; nothing is outstanding.
    Idle,
    /// The work is done and wants looking at.
    Present,
    /// Ended, having done what it was asked.
    Finish { summary: Option<String> },
    /// Ended badly, on purpose.
    Fail { reason: String },
}

impl FakeStep {
    /// The step's kind, for tracing and for tests. Never its payload: a
    /// payload is untrusted adapter text and does not go in a log line.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Think => "think",
            Self::Read { .. } => "read",
            Self::Search { .. } => "search",
            Self::ProposeWrite { .. } => "proposeWrite",
            Self::Write { .. } => "write",
            Self::Command { .. } => "command",
            Self::Test { .. } => "test",
            Self::CommandDone { .. } => "commandDone",
            Self::Reply { .. } => "reply",
            Self::AskPermission { .. } => "askPermission",
            Self::AskDecision { .. } => "askDecision",
            Self::Idle => "idle",
            Self::Present => "present",
            Self::Finish { .. } => "finish",
            Self::Fail { .. } => "fail",
        }
    }

    pub fn prompt(text: impl Into<String>) -> Self {
        Self::Prompt { text: text.into() }
    }

    pub fn read(path: impl Into<String>) -> Self {
        Self::Read { path: path.into() }
    }

    pub fn search(query: impl Into<String>) -> Self {
        Self::Search {
            query: query.into(),
        }
    }

    pub fn propose_write(path: impl Into<String>) -> Self {
        Self::ProposeWrite { path: path.into() }
    }

    pub fn write(path: impl Into<String>) -> Self {
        Self::Write { path: path.into() }
    }

    pub fn command(command: impl Into<String>) -> Self {
        Self::Command {
            command: command.into(),
            cwd: None,
        }
    }

    pub fn test(command: impl Into<String>) -> Self {
        Self::Test {
            command: command.into(),
        }
    }

    pub fn reply(text: impl Into<String>) -> Self {
        Self::Reply { text: text.into() }
    }

    pub fn ask_permission(tool: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::AskPermission {
            tool: tool.into(),
            detail: None,
            reason: reason.into(),
        }
    }

    pub fn ask_decision(question: impl Into<String>) -> Self {
        Self::AskDecision {
            question: question.into(),
        }
    }

    pub fn finish(summary: impl Into<String>) -> Self {
        Self::Finish {
            summary: Some(summary.into()),
        }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self::Fail {
            reason: reason.into(),
        }
    }
}

/// Everything the driver can report, which is every [`FakeStep`] plus the four
/// things that happen *to* a run.
///
/// The extra four can never appear in a script, and that is a type-level fact
/// rather than a convention: a caller cannot write "and then I was cancelled".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeAction {
    /// A step the script asked for.
    Step(FakeStep),
    /// A pending permission ask was answered out of band (§10) and the run
    /// resumed. `granted` is recorded; the fake does not model an agent
    /// reasoning differently about a refusal.
    PermissionAnswered { granted: bool },
    /// A pending question was answered out of band (§10) and the run resumed.
    /// The answer itself is not carried: turning a `DECISION_REQUESTED` into
    /// something with an answer is T335's, and inventing half of it here would
    /// leave T335 to redo it.
    DecisionAnswered,
    /// Someone cancelled the run. Reported as an action so the record is
    /// complete; the protocol says this with the run's *state* and not with a
    /// fifteenth event variant.
    Cancelled,
    /// The run was orphaned and the fake resolved it rather than leaving it
    /// running (§11). One of [`crate::ORPHANED_UNTERMINATED`] or
    /// [`crate::ORPHANED_DROPPED`], both `&'static str` so the record stays
    /// byte-for-byte reproducible.
    Orphaned { reason: &'static str },
}

impl FakeAction {
    /// The action's kind, for tracing. Never its payload.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Step(step) => step.kind(),
            Self::PermissionAnswered { .. } => "permissionAnswered",
            Self::DecisionAnswered => "decisionAnswered",
            Self::Cancelled => "cancelled",
            Self::Orphaned { .. } => "orphaned",
        }
    }

    /// The `AgentRunEvent` variants this action should become, in order,
    /// spelled as `AgentRunEvent::as_str` spells them.
    ///
    /// **A specification, not an implementation.** The translation itself lives
    /// in `agent_run` (T300-CORE) because this crate cannot depend on
    /// `gritty-core` without a cycle. Publishing it here as strings is what
    /// lets a reviewer check the two lanes agree, and lets this crate's tests
    /// assert that every action has a decided answer.
    ///
    /// Two variants of the protocol appear nowhere below, on purpose:
    /// `RUN_STARTED`, which the manager emits when it creates the run rather
    /// than anything the adapter reported, and nothing at all for a
    /// cancellation, which is a fact about the run's state.
    pub const fn run_events(&self) -> &'static [&'static str] {
        match self {
            Self::Step(FakeStep::Prompt { .. }) => &["PROMPT_STARTED"],
            Self::Step(FakeStep::Reply { .. }) => &["PROMPT_FINISHED"],
            Self::Step(FakeStep::Read { .. }) => &["FILE_READ"],
            Self::Step(FakeStep::ProposeWrite { .. }) => &["FILE_WRITE_REQUESTED"],
            Self::Step(FakeStep::Write { .. }) => &["FILE_WRITTEN"],
            Self::Step(FakeStep::Command { .. } | FakeStep::Test { .. }) => &["COMMAND_STARTED"],
            Self::Step(FakeStep::CommandDone { .. }) => &["COMMAND_FINISHED"],
            // Two events, and in this order: the ask, then the fact that the
            // run stopped on it. `RUN_BLOCKED` carries the step's `reason`.
            Self::Step(FakeStep::AskPermission { .. }) => &["PERMISSION_REQUESTED", "RUN_BLOCKED"],
            // One event: the state says `NEEDS_DECISION`, which is a different
            // stop from `BLOCKED` and does not also need announcing as one.
            Self::Step(FakeStep::AskDecision { .. }) => &["DECISION_REQUESTED"],
            Self::Step(FakeStep::Finish { .. }) => &["RUN_FINISHED"],
            Self::Step(FakeStep::Fail { .. }) | Self::Orphaned { .. } => &["RUN_FAILED"],
            // Activity or state only: the protocol has no variant, and
            // inventing one to fill the gap is another ticket's call.
            Self::Step(
                FakeStep::Think | FakeStep::Search { .. } | FakeStep::Idle | FakeStep::Present,
            ) => &[],
            Self::PermissionAnswered { .. } | Self::DecisionAnswered => &["RUN_RESUMED"],
            Self::Cancelled => &[],
        }
    }

    /// The `AgentRunActivityKind` this action puts the run in, spelled as
    /// `AgentRunActivityKind::as_str` spells it, or `None` when the run has no
    /// particular activity — which is every ending.
    ///
    /// A suggestion in the same sense as [`Self::run_events`]: the core
    /// decides, and the paths and commands an `AgentRunActivity` carries come
    /// from the action's own payload.
    pub const fn run_activity(&self) -> Option<&'static str> {
        match self {
            Self::Step(FakeStep::Prompt { .. } | FakeStep::Think) => Some("THINKING"),
            Self::Step(FakeStep::Read { .. }) => Some("READING"),
            Self::Step(FakeStep::Search { .. }) => Some("SEARCHING"),
            Self::Step(FakeStep::ProposeWrite { .. } | FakeStep::Write { .. }) => Some("EDITING"),
            Self::Step(FakeStep::Command { .. }) => Some("EXECUTING"),
            Self::Step(FakeStep::Test { .. }) => Some("TESTING"),
            Self::Step(FakeStep::CommandDone { .. } | FakeStep::Reply { .. }) => Some("THINKING"),
            Self::Step(FakeStep::AskPermission { .. }) => Some("REQUESTING_PERMISSION"),
            Self::Step(FakeStep::AskDecision { .. }) => Some("REQUESTING_DECISION"),
            Self::Step(FakeStep::Idle) => Some("WAITING"),
            Self::PermissionAnswered { .. } | Self::DecisionAnswered => Some("THINKING"),
            Self::Step(FakeStep::Present | FakeStep::Finish { .. } | FakeStep::Fail { .. })
            | Self::Cancelled
            | Self::Orphaned { .. } => None,
        }
    }
}

/// An ordered script, and a name for it.
///
/// The name is what `runs.startFake` will echo back; it carries no behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeScript {
    name: String,
    steps: Vec<FakeStep>,
}

impl FakeScript {
    pub fn new(name: impl Into<String>, steps: Vec<FakeStep>) -> Self {
        Self {
            name: name.into(),
            steps,
        }
    }

    /// One of the presets, by value.
    pub fn preset(preset: FakeScriptPreset) -> Self {
        Self::new(preset.as_str(), preset.steps())
    }

    /// One of the presets, by the name `runs.startFake` will be given.
    pub fn preset_named(name: &str) -> Result<Self, FakeError> {
        FakeScriptPreset::parse(name).map(Self::preset)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn steps(&self) -> &[FakeStep] {
        &self.steps
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// The scripts that ship with the fake.
///
/// `runs.startFake` takes one of these by name (§16), so the set is part of
/// what the command surface offers rather than test scaffolding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FakeScriptPreset {
    /// The acceptance script, and exactly it: `STARTING → RUNNING → BLOCKED
    /// → RUNNING → NEEDS_DECISION → RUNNING → FINISHED`, with both stops
    /// cleared out of band. Nothing else is in it, so the sequence a test
    /// asserts is the sequence the criterion names.
    FullLifecycle,
    /// The other three non-terminal states, which the acceptance script
    /// deliberately does not visit: `STARTING → RUNNING → WAITING → RUNNING →
    /// REVIEW → FINISHED`. Together with [`Self::FullLifecycle`],
    /// [`Self::Failing`] and a cancellation, this covers the whole state
    /// machine.
    Reviewed,
    /// Prompt, reply, finish. The shortest run that is still a run.
    Quick,
    /// Prompt, then a scripted failure.
    Failing,
    /// A script with no terminal step at all, so that §11's guarantee — a run
    /// resolves rather than sitting in `RUNNING` forever — has something to be
    /// demonstrated on.
    Unterminated,
    /// [`CHATTY_STEPS`] reads, so a bounded ring can be overflowed on purpose.
    Chatty,
    /// Every string a hostile shape: far past the core's limits, full of
    /// newlines and control characters, a path that escapes the workspace root,
    /// a command that looks runnable. The fake emits all of it **verbatim** —
    /// proving the boundary works is the entire reason it exists, and a fake
    /// that sanitised its own output would prove the opposite.
    Hostile,
}

impl FakeScriptPreset {
    pub const ALL: &'static [Self] = &[
        Self::FullLifecycle,
        Self::Reviewed,
        Self::Quick,
        Self::Failing,
        Self::Unterminated,
        Self::Chatty,
        Self::Hostile,
    ];

    /// The name on the wire. camelCase, like every other protocol string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FullLifecycle => "fullLifecycle",
            Self::Reviewed => "reviewed",
            Self::Quick => "quick",
            Self::Failing => "failing",
            Self::Unterminated => "unterminated",
            Self::Chatty => "chatty",
            Self::Hostile => "hostile",
        }
    }

    /// Parse a preset name. An unknown name is refused rather than defaulted:
    /// starting the wrong script silently is worse than not starting one.
    pub fn parse(raw: &str) -> Result<Self, FakeError> {
        Self::ALL
            .iter()
            .copied()
            .find(|preset| preset.as_str() == raw)
            .ok_or_else(|| FakeError::UnknownPreset {
                name: raw.to_owned(),
            })
    }

    /// The steps. Deterministic and allocation-order-independent: the same
    /// call gives the same `Vec` every time, which is what makes a run
    /// reproducible before the driver is even involved.
    pub fn steps(self) -> Vec<FakeStep> {
        match self {
            Self::FullLifecycle => vec![
                FakeStep::prompt("Add a session timeout to the auth module."),
                FakeStep::Think,
                FakeStep::search("createSession"),
                FakeStep::read("src/auth/session.ts"),
                FakeStep::ask_permission("Edit", "editing a file outside the last approved set"),
                FakeStep::propose_write("src/auth/session.ts"),
                FakeStep::write("src/auth/session.ts"),
                FakeStep::ask_decision("Should an expired session log the user out, or refresh?"),
                FakeStep::Command {
                    command: "cargo test -p gritty-core".to_owned(),
                    cwd: Some(".".to_owned()),
                },
                FakeStep::CommandDone {
                    ok: true,
                    exit_code: Some(0),
                },
                FakeStep::test("pnpm vitest run"),
                FakeStep::CommandDone {
                    ok: true,
                    exit_code: Some(0),
                },
                FakeStep::reply("Sessions now expire after 30 minutes."),
                FakeStep::finish("Added a session timeout and a test for it."),
            ],
            Self::Reviewed => vec![
                FakeStep::prompt("Tidy the changelog."),
                FakeStep::read("CHANGELOG.md"),
                FakeStep::propose_write("CHANGELOG.md"),
                FakeStep::write("CHANGELOG.md"),
                FakeStep::reply("Tidied."),
                FakeStep::Idle,
                FakeStep::prompt("Add today's date to the top entry."),
                FakeStep::Present,
                FakeStep::finish("Changelog tidied and dated."),
            ],
            Self::Quick => vec![
                FakeStep::prompt("What does this module do?"),
                FakeStep::reply("It owns document state."),
                FakeStep::finish("Answered."),
            ],
            Self::Failing => vec![
                FakeStep::prompt("Refactor the parser."),
                FakeStep::read("src/parser.rs"),
                FakeStep::fail("the provider closed the connection"),
            ],
            Self::Unterminated => vec![
                FakeStep::prompt("Look around."),
                FakeStep::read("README.md"),
                FakeStep::read("AGENTS.md"),
            ],
            Self::Chatty => {
                let mut steps = Vec::with_capacity(CHATTY_STEPS + 2);
                steps.push(FakeStep::prompt("Read everything."));
                for index in 0..CHATTY_STEPS {
                    steps.push(FakeStep::read(format!("src/file-{index:04}.rs")));
                }
                steps.push(FakeStep::finish("Read everything."));
                steps
            }
            Self::Hostile => vec![
                FakeStep::prompt("x".repeat(64 * 1024)),
                // A path that leaves the workspace root: the core must refuse
                // to make it openable and keep it as a display string only.
                FakeStep::read("../../../../etc/passwd"),
                // Newlines and a control character in a one-line field.
                FakeStep::search("first line\nsecond line\r\n\t\u{7}bell"),
                FakeStep::Command {
                    command: "rm -rf / # ".to_owned() + &"y".repeat(4096),
                    cwd: Some("~/elsewhere".to_owned()),
                },
                FakeStep::CommandDone {
                    ok: false,
                    exit_code: None,
                },
                FakeStep::AskPermission {
                    tool: "T".repeat(4096),
                    detail: Some("<script>alert(1)</script>".to_owned()),
                    reason: "  \n  lots   of   space  \n  ".to_owned(),
                },
                FakeStep::ask_decision("q".repeat(8192)),
                FakeStep::fail("f".repeat(8192)),
            ],
        }
    }
}

impl std::fmt::Display for FakeScriptPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One of every [`FakeStep`] shape, for exhaustive tests. Payloads are
/// irrelevant here; only the shapes are.
#[cfg(test)]
pub(crate) fn every_step_shape() -> Vec<FakeStep> {
    vec![
        FakeStep::prompt("p"),
        FakeStep::Think,
        FakeStep::read("a"),
        FakeStep::search("q"),
        FakeStep::propose_write("a"),
        FakeStep::write("a"),
        FakeStep::command("c"),
        FakeStep::test("t"),
        FakeStep::CommandDone {
            ok: true,
            exit_code: Some(0),
        },
        FakeStep::reply("r"),
        FakeStep::ask_permission("Edit", "why"),
        FakeStep::ask_decision("q?"),
        FakeStep::Idle,
        FakeStep::Present,
        FakeStep::finish("s"),
        FakeStep::fail("e"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_shape_covers_the_vocabulary() {
        let shapes = every_step_shape();
        let mut kinds: Vec<&str> = shapes.iter().map(FakeStep::kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), shapes.len(), "a shape is duplicated");
        assert_eq!(shapes.len(), 16);
    }

    /// The translation is a specification and a specification has to be total:
    /// every action has a decided answer, even when the answer is "no event".
    #[test]
    fn every_action_declares_the_events_it_becomes() {
        let mut actions: Vec<FakeAction> = every_step_shape()
            .into_iter()
            .map(FakeAction::Step)
            .collect();
        actions.push(FakeAction::PermissionAnswered { granted: true });
        actions.push(FakeAction::DecisionAnswered);
        actions.push(FakeAction::Cancelled);
        actions.push(FakeAction::Orphaned {
            reason: crate::ORPHANED_DROPPED,
        });

        const VOCABULARY: &[&str] = &[
            "RUN_STARTED",
            "RUN_FINISHED",
            "PROMPT_STARTED",
            "PROMPT_FINISHED",
            "FILE_READ",
            "FILE_WRITE_REQUESTED",
            "FILE_WRITTEN",
            "COMMAND_STARTED",
            "COMMAND_FINISHED",
            "PERMISSION_REQUESTED",
            "DECISION_REQUESTED",
            "RUN_BLOCKED",
            "RUN_RESUMED",
            "RUN_FAILED",
        ];
        const ACTIVITIES: &[&str] = &[
            "THINKING",
            "READING",
            "EDITING",
            "SEARCHING",
            "EXECUTING",
            "TESTING",
            "WAITING",
            "REQUESTING_PERMISSION",
            "REQUESTING_DECISION",
        ];

        let mut produced: Vec<&str> = Vec::new();
        for action in &actions {
            for name in action.run_events() {
                assert!(
                    VOCABULARY.contains(name),
                    "{name} is not an AgentRunEvent variant"
                );
                produced.push(name);
            }
            if let Some(activity) = action.run_activity() {
                assert!(
                    ACTIVITIES.contains(&activity),
                    "{activity} is not an AgentRunActivityKind"
                );
            }
        }

        // `RUN_STARTED` is the manager's, not an adapter report; everything
        // else in the vocabulary is reachable from some action.
        for name in VOCABULARY {
            let expected = *name != "RUN_STARTED";
            assert_eq!(
                produced.contains(name),
                expected,
                "{name} reachability disagrees with the mapping table"
            );
        }
    }

    /// A run that never says `RUN_BLOCKED` for a decision is a deliberate
    /// choice, not an omission: the state carries it.
    #[test]
    fn a_permission_ask_announces_the_block_and_a_decision_does_not() {
        assert_eq!(
            FakeAction::Step(FakeStep::ask_permission("Edit", "why")).run_events(),
            &["PERMISSION_REQUESTED", "RUN_BLOCKED"]
        );
        assert_eq!(
            FakeAction::Step(FakeStep::ask_decision("q?")).run_events(),
            &["DECISION_REQUESTED"]
        );
        assert!(FakeAction::Cancelled.run_events().is_empty());
    }

    /// All nine activity kinds are reachable from the two scripted presets that
    /// between them walk the whole state machine. If the protocol grows a
    /// tenth, this fails until a preset can produce it.
    #[test]
    fn every_activity_kind_the_protocol_has_is_reachable_from_a_preset() {
        let mut seen: Vec<&str> = [FakeScriptPreset::FullLifecycle, FakeScriptPreset::Reviewed]
            .into_iter()
            .flat_map(FakeScriptPreset::steps)
            .map(FakeAction::Step)
            .filter_map(|action| action.run_activity())
            .collect();
        seen.push(
            FakeAction::PermissionAnswered { granted: true }
                .run_activity()
                .unwrap(),
        );
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen,
            vec![
                "EDITING",
                "EXECUTING",
                "READING",
                "REQUESTING_DECISION",
                "REQUESTING_PERMISSION",
                "SEARCHING",
                "TESTING",
                "THINKING",
                "WAITING",
            ]
        );
    }

    #[test]
    fn presets_round_trip_through_their_wire_name() {
        for &preset in FakeScriptPreset::ALL {
            assert_eq!(FakeScriptPreset::parse(preset.as_str()).unwrap(), preset);
            assert_eq!(preset.to_string(), preset.as_str());
            assert!(!preset.steps().is_empty(), "{preset} has no steps");
        }
        let error = FakeScriptPreset::parse("FULL_LIFECYCLE").expect_err("not a preset name");
        assert!(matches!(error, FakeError::UnknownPreset { .. }));
    }

    /// Determinism starts before the driver: the same preset built twice is the
    /// same `Vec`, every field included.
    #[test]
    fn a_preset_builds_identically_every_time() {
        for &preset in FakeScriptPreset::ALL {
            assert_eq!(preset.steps(), preset.steps(), "{preset} is not stable");
            assert_eq!(FakeScript::preset(preset), FakeScript::preset(preset));
        }
    }

    #[test]
    fn the_chatty_preset_can_overflow_a_bounded_ring() {
        let steps = FakeScriptPreset::Chatty.steps();
        assert_eq!(steps.len(), CHATTY_STEPS + 2);
        const { assert!(CHATTY_STEPS > 512, "must exceed the core's MAX_RUN_EVENTS") };
    }

    /// The fake does not tidy its own output. If this ever starts passing with
    /// bounded strings, the boundary moved to the wrong side.
    #[test]
    fn the_hostile_preset_is_left_exactly_as_written() {
        let steps = FakeScriptPreset::Hostile.steps();
        let FakeStep::Prompt { text } = &steps[0] else {
            panic!("the hostile script starts with a prompt");
        };
        assert_eq!(text.chars().count(), 64 * 1024);
        assert!(matches!(&steps[1], FakeStep::Read { path } if path.starts_with("../")));
        assert!(matches!(&steps[2], FakeStep::Search { query } if query.contains('\n')));
    }

    #[test]
    fn a_script_carries_its_name_and_its_steps() {
        let script = FakeScript::preset_named("quick").unwrap();
        assert_eq!(script.name(), "quick");
        assert_eq!(script.len(), 3);
        assert!(!script.is_empty());
        assert!(FakeScript::preset_named("nope").is_err());
    }
}
