# ADR-018: Gritty may install a hook into another tool's configuration, additively and with consent

- **Status:** Accepted (implemented by T221)
- **Date:** 2026-09-05

## Context

T220 discovers the coding-agent sessions already running on the machine from
two sources it can read without anyone's cooperation: the process table, and
the transcripts the agents write. It deliberately stops short of one value.
`AgentActivity`'s own doc comment records why:

> There is deliberately no `WAITING_FOR_APPROVAL`. Poll-only discovery cannot
> tell a pending permission prompt from a finished turn — both look like a live
> process with nothing outstanding — and inventing the distinction would be
> exactly the kind of claim `AGENTS.md` rule 12 forbids. A hook integration is
> what would earn that value.

That missing value is the most important one in the product. `AGENTS.md` opens
with the list of questions every feature must let the user answer, and *what
needs my attention* is on it. A roster that shows four running agents but
cannot say which one is stopped at a permission prompt is a roster the user
still has to walk pane by pane — which is the exact failure the island exists
to remove.

There are three ways to earn it, and only one is honest.

**Screen-scraping.** Herdr, the closest comparable product, matches rule sets
against the live bottom of each pane ("screen manifests") on top of
foreground-process identity. Its own documentation lists non-determinism as a
known limitation: the manifests break when an agent's interface changes, and
unsupported agents stay `unknown`. Gritty also cannot do this at all for the
sessions T220 targets, because they run in *other* terminals whose screens
Gritty does not own.

**Owning the terminal.** Run the agent inside a Gritty PTY and read its output.
That works, and it is where T300 goes, but it only covers sessions Gritty
started. The whole point of T220 is the ones it did not.

**Asking the agent.** Every agent CLI in scope already has a hook system, and
every one of them fires an event precisely when it blocks on the user. This is
the agent reporting its own state rather than Gritty guessing at it — a
contract, not a heuristic.

The evidence that this is real, from the machine this was written on:
`~/.claude/settings.json` configures fourteen hook events, `PermissionRequest`
among them; `~/.codex/hooks.json` carries the same event names in the same JSON
shape. Both already point at a third-party menu-bar application. The mechanism
is established, cross-provider, and *already in use by someone else on this
machine* — which is where the difficulty lies.

Because hooks are configured in the user's own global files, taking this route
means Gritty writing into configuration it does not own. That is the decision
this ADR exists to constrain. Nothing else in the repository does it: ADR-017
went out of its way to reach a shell through the environment of a session
Gritty spawns, "never by editing the user's dotfiles", and T220's non-goals say
in as many words that Gritty "does not write to `~/.claude/settings.json`,
`~/.codex/config.toml` or any other tool's configuration".

This ADR is the deliberate exception those two documents anticipated, and it is
narrow.

## Decision

**Gritty may write a hook entry into a supported agent's configuration file,
subject to seven constraints. The hook writes a state file; the core interprets
it. Hook-sourced state is evidence, never authority.**

### 1. Consent, per provider, per install

Nothing is written without an explicit user action taken in the UI for that
provider. There is no install on first run, no install as a side effect of
discovery, no "we noticed you use Codex" prompt on startup, and no single
control that installs into several tools at once. Discovery keeps working with
nothing installed, which stays the default state.

### 2. Additive only, and identified

Gritty appends one entry to each event's existing array. It never replaces an
array, never reorders one, and never removes or edits an entry that is not its
own. Its entries are identified by the absolute path of its own hook binary, so
recognising them later needs no marker comment and no sidecar file.

This constraint is not hypothetical. The machine this was written on has
another tool registered on every event with `matcher: "*"`. An installer that
wrote the array it wished to see would silently disable that tool, and the user
would discover it as *the other app stopped working*, with nothing pointing at
Gritty. Agent CLIs run every matching hook, so appending costs the incumbent
nothing.

*Refined by T221.* "Identified by the absolute path of its own hook binary"
means the path the entry's command **runs**, recognised by that binary's own
file name — not equality with the path this build would install. Exact equality
would stop recognising Gritty's own entry the moment the application moved, and
an entry nobody recognises can be neither reported as broken (§7's uninstall
obligation) nor removed. The name is Gritty's, so nothing else claims it.

### 3. Inspectable before, reversible after

The exact change is shown before it is applied — the file, the events, the
entry — and the prior contents are copied to a timestamped backup beside the
original. Uninstall removes exactly the entries whose command path is Gritty's
and leaves everything else byte-identical.

### 4. Refuse what is not understood

A configuration file that does not parse is **refused with an explanation**,
never rewritten, never "repaired", never replaced from a template. The same
holds for a file whose hook section has a shape the installer does not
recognise. Losing a user's hook configuration to a confident parser would be
unrecoverable and would fail hard rule 1's spirit: the user's files are the
durable artifacts.

Formatting is preserved as far as the format allows. A JSON file is re-emitted
with two-space indentation and its key order retained; a TOML file is edited
rather than re-serialised. Gritty does not reformat a file it was invited to
append one entry to.

### 5. The hook never blocks or breaks the agent

The hook binary writes one small file and exits. It performs no network access,
starts no process, holds no lock, and takes no input from the agent beyond the
payload on its stdin. Its own failure is silent to the agent — a non-zero exit
from a hook is the agent's problem, so it never returns one — and visible only
inside Gritty, as a stale or missing state file.

An agent must be no slower and no less reliable for having Gritty installed. If
that cannot be guaranteed for a provider, that provider does not get an
integration.

### 6. The hook knows nothing of the Gritty protocol

It writes a small JSON state file to Gritty's app-data directory. It does not
link `gritty-core`, does not know what an `AgentSession` is, and does not decide
what any phase *means*. The core reads the file and derives the activity, the
same adapter/domain split ADR-005 asks for and T220 already applies to `ps` and
to transcripts. A hook is a third evidence source, not a second brain.

A file, rather than a socket, because Gritty is usually **not running** when a
hook fires. A socket connect would either fail — leaving the hook to decide
what to do about it, which is logic that does not belong in a binary that must
never block — or stall the agent while it timed out. A write to a small file
costs microseconds and needs nothing listening.

### 7. Evidence, not authority

Process evidence still wins. An agent killed at a permission prompt leaves a
state file saying `BLOCKED` and no process; the roster reports `ENDED`, because
T220's ordering already puts the absence of a process above everything else. A
state file is one more input to a derivation that was designed to combine
disagreeing sources, and the resulting `activitySource` says which one spoke.

This is also what keeps the integration optional rather than load-bearing: with
it, `BLOCKED` becomes available; without it, the roster is exactly what T220
shipped.

## Consequences

- **`AgentActivity` gains `BLOCKED`, and it is only ever set from hook
  evidence.** No transcript pattern, no process state and no elapsed-time
  threshold may produce it. This keeps rule 12 satisfied by construction: the
  value exists only where something told Gritty the truth.
- **`AgentSession` gains `activitySource`.** With three sources that can
  disagree, the UI has to be able to say why a row claims what it claims.
  Herdr ships this as an `agent explain` subcommand; here it is a field,
  because the island already has somewhere to put it.
- **Gritty acquires an uninstall obligation.** Anything installed can be
  orphaned — by an app upgrade that moves the binary, by the app being deleted.
  A hook pointing at a missing binary must be harmless (the agent CLIs tolerate
  a failing hook), and the integration status must report it as broken rather
  than as installed.
- **The write is a new security surface** (hard rule 13). What the hook receives
  on stdin is untrusted input from a process Gritty does not control, and what
  it writes is read back by the core as untrusted input a second time: bounded,
  never interpreted, never logged in full. The state file lives in Gritty's own
  app-data directory, not in a world-writable location.
- **T220's non-goal is superseded, narrowly.** Read-only discovery remains the
  default and remains complete on its own. This adds an opt-in path to one extra
  value; it does not make discovery depend on cooperation.
- **This does not open the door to control.** A hook is a notification the agent
  emits, not a channel into it. Answering a permission prompt, cancelling a
  turn or injecting a keystroke are none of them consequences of this decision;
  they belong to runs Gritty owns (T300).
- **One mechanism, several providers.** Claude Code and Codex already agree on
  the event names and the JSON shape, so a provider is a config path, a parse
  and a set of event names — not a new design.

## Alternatives rejected

**Screen-scraping the agent's TUI.** Impossible for terminals Gritty does not
own, and non-deterministic where it is possible. Herdr does it and documents it
as breaking on interface changes; adopting a technique whose author already
lists it as unreliable would trade a missing value for a wrong one.

**A socket the hook connects to.** Requires Gritty to be running and puts
failure handling inside the hook. See constraint 6.

**An append-only event log per session.** More faithful — it would carry
ordering and history — but it grows without bound, so it needs rotation,
truncation-tolerant parsing and an age policy, all to answer a question that
only ever concerns the *current* phase. The overwritten state file has none of
that and cannot be corrupted by a partial write, because the write is atomic.

**Shipping a wrapper command the user runs instead of `claude`.** Herdr's
documentation is explicit that it does not wrap agents, and for good reason: a
wrapper has to track every flag, every exit path and every TTY behaviour of the
tool it fronts, forever. It also would not cover sessions started any other way,
which is most of them.

**Doing nothing.** Defensible — the roster is useful as T220 shipped it — but it
leaves the product's central question answerable only by hand, and the mechanism
to answer it properly is already installed on the machine by someone else.

## Revisit when

An agent CLI offers a first-class query interface (a socket, a status file it
maintains itself, a documented state API) that makes an installed hook
unnecessary — at which point this becomes one adapter among several rather than
the only route. Or when Gritty owns the agent's terminal (T300/T310) and can
read the state from a process it started, which makes the integration redundant
for those sessions and leaves it covering only the external ones.

## Amendment (AGENT-TURN-END, 2026-09-14): the turn, not only the prompt

- **Status:** Accepted
- **Date:** 2026-09-14

This ADR's Consequences said `BLOCKED` was the one value the derivation could
not reach without the hook, and T221 ignored the hook's other phases on that
basis. That was wrong. Claude Code writes a transcript block only once it has
finished streaming, so a model **thinking or writing** leaves the transcript
showing nothing outstanding — the same evidence as a finished turn. The roster
called that `WAITING`, the rail said Done, and T223 notified on it. A
hook-driven monitor watching the same session said *in progress*, correctly.

**What changes.**

- `WORKING` means *the agent's turn is in progress*: a tool in flight, or the
  newest turn evidence says the turn started or continued. `WAITING` from
  `HOOK` or `TRANSCRIPT` means a turn was seen to end.
- The hook's `ACTIVE` phase is evidence of a turn in progress, and `SETTLED`
  of one ended, each at its `since_ms`. The transcript contributes the same two
  facts: a prompt, a tool result or any model record is in progress; a
  `system/turn_duration`, a `system/stop_hook_summary` or the
  `[Request interrupted by user…]` line is an end.
- **Newest evidence wins**, a tie goes to the hook. The two describe one turn
  and normally agree; when they do not, the later report knows more.
- **A newer transcript end discards `BLOCKED`.** Only an end, and only a newer
  one: nothing written while a prompt stands may clear it.
- With **no turn evidence** a live session stays `WAITING` from `PROCESS`, which
  the UI calls **Alive**, and which is not counted as needing the user and does
  not notify.
- An in-progress transcript mark older than the idle threshold is no evidence,
  like a stale hook phase. An end does not decay.

**What does not change.** `BLOCKED` is still set only from hook evidence (the
new rules can only *discard* it). Process absence still outranks everything
(§7). The integration stays optional: without it the transcript carries the
turn on its own. No wire shape changed, so there is no version bump.

**Measured, not assumed** (Claude Code 2.1.270, a pty-driven session with
logging hooks):

| Scenario | Hook events | Transcript |
|---|---|---|
| Prompt, generation, answer | `UserPromptSubmit` (+40 ms after the prompt record), `Stop` | thinking at +1.9 s, text at +6.5 s, then `stop_hook_summary` and `turn_duration` in `Stop`'s millisecond |
| Esc during generation | **none after `UserPromptSubmit`** | partial text, then `[Request interrupted by user]`; no `turn_duration` |
| Esc at a Bash permission prompt | `PreToolUse`, `PermissionRequest`, **then none** | error `tool_result`, `[Request interrupted by user for tool use]`, `turn_duration` |

The second row is why the transcript's interrupt line must be an end marker in
its own right and why newest-wins exists: a hook-only rule would hold such a
session at `WORKING` until the phase went stale. The third is why a newer end
discards `BLOCKED`: before this amendment that row read Pending validation for
up to five minutes after the user had already dismissed the prompt.

Replaying 30 days of real transcripts (114 files) through the real scanner,
cut after every record and scored against an oracle that ignores end markers
(did the model write again before the next human input?): time spent **in
progress but shown Done** fell from **56.4 h to 1.3 h**; ended-but-Working is
0.2 h; Alive covers 4.7 h, mostly sessions that have not received a first
prompt. Failed and rate-limited turns all wrote `turn_duration` (41 of 41).

**Two more starts (TURN-START-QUEUED, 2026-09-14).** A
`queue-operation`/`dequeue` record and a `<bash-input>` user record (a `!`
command) are in-progress boundaries too. The rules stay the same; only the list
of records grows. Measured first (TURN-START-DEQUEUE) on the same corpus: the
dequeue is the only marker in front of queue-delivered meta input (a peer
session's message, a usage-limit "continue"), and the model answered `!`
commands, which the AGENT-TURN-END classifier had assumed it did not. Together
they remove about 20 minutes per 30 days of in-progress-shown-Done, for 22 s of
ended-shown-Working. A dequeue that no turn follows reads Working for at most the
idle threshold, then decays like any in-progress mark.

## Amendment (ADR-018A, 2026-09-14): the hook may refuse a write on ground another workstream holds

- **Status:** Accepted
- **Date:** 2026-09-14
- **Accepted:** 2026-09-14 by Alexis, as drafted, taking the recommended answer
  to each question the draft left open:
  1. With Gritty not running, nothing is refused (§4, item 2).
  2. A stale lease still refuses a claim and no longer refuses a write (§2).
  3. An unlinked session is never refused (§2).
  4. Shell writes are not refused by pattern; they stay named as not enforced
     (*Alternatives rejected*).
  5. Consent is one switch per provider over every live lease, not a per-lease
     mode; an `OwnershipMode` stays undecided (§5, *Not decided here*).
  6. The refusal verb is `deny`, and the override lives in Gritty (§6).
  7. An owned run stays observed until T330-R has measured `PreToolUse` under
     `-p` (§7). **Whether Gritty may pass `--settings` when it launches a run is
     not decided by this acceptance**: T330-R brings that back with its
     measurement if the installed entry turns out not to fire.

  Nothing is enforced until T330 ships, so ADR-019's "observed rather than
  enforced" and `docs/architecture/ownership.md` stay true until then (§8).
- **Amends:** §5, §6 and §7 of this ADR; ADR-020's "The core never synthesises
  one" and "Enforcement is still not this"; ADR-019 §7, whose own *Revisit when*
  names this amendment.
- **Unblocks:** T330 (write interception): `T330-R` the interception,
  `T330-U` the denial surface (`execution-board.md:320`).

**Why now.** ADR-019 §7 made ownership observed, not enforced, and said changing
that is "an amendment to ADR-018 first and code second"
(`ADR-019-workspace-ownership.md:120-127`; again at
`T325-workspace-ownership.md:282-286`). T326 then built the after-the-fact half:
a write inside another workstream's lease becomes an `OBSERVED` conflict
(`T326-ownership-violations.md:229-234`). Its own report closes on the case for
this one: "every `OBSERVED` row is a write enforcement would have stopped"
(`:460-462`).

Detection also has a gap enforcement does not. T326 reads `FileTouched`, which
the transcript scanner derives from Claude Code's `file-history-delta` records
(`crates/agents/src/transcript.rs:853-857`) — checkpointing. Checkpointing
"does not track files modified by bash commands", and for subagents "Claude
Code usually doesn't capture those edits in your session's checkpoints"
(code.claude.com/docs/en/checkpointing, *Limitations*, read 2026-09-14). A
`PreToolUse` hook, by contrast, fires for a subagent's tool calls too (below).

The mechanism is already in daily use in this repository. `scripts/agent-lease.py
guard` is a `PreToolUse` hook that denies a file-tool write inside a live
foreign lease (`agent-lease.py:567-599`, wired at `.claude/settings.json:44-53`)
and fails open. This amendment is that idea held to ADR-018's constraints.

**What was checked** (Claude Code 2.1.270; documentation read 2026-09-14, not
executed — the rows marked *unverified* are T330-R's to measure before any
claim, rule 12):

| Question | Answer | Source |
|---|---|---|
| When does `PreToolUse` run? | "before the permission prompt, for every tool except `EndConversation`" | docs/en/permissions, *Extend permissions with hooks* |
| Can it refuse? | `permissionDecision: "deny"` blocks the call; Claude sees `permissionDecisionReason` | docs/en/hooks, *PreToolUse decision control* |
| Does a hook `allow` widen permissions? | It can skip the prompt; deny and ask **rules** still apply | docs/en/permissions, same section |
| Exit code | JSON is read on every exit code; exit 2 blocks regardless of JSON; exit 0 with invalid JSON is a non-blocking error and the call proceeds | docs/en/hooks, *Exit code* |
| Timeout, or a hook that cannot start | "doesn't block the tool call … don't count on a stalled hook to act as a gate"; default `timeout` 600 s | docs/en/hooks |
| Subagents | `PreToolUse` fires for a subagent's tool calls, with `agent_id` / `agent_type` | docs/en/hooks, *common input fields* |
| MCP tools | seen, named `mcp__<server>__<tool>`; input shape is the server's | docs/en/hooks, *Match MCP tools* |
| Several matching hooks | "run in parallel"; how their decisions combine is **not documented** | docs/en/hooks |
| `bypassPermissions` | whether a `PreToolUse` deny still holds is **not documented** — *unverified* | — |
| `-p` (owned runs) | whether `PreToolUse` fires is **unverified**; `PermissionRequest` does not — print mode shows no prompt to run before, observed while probing hooks for ADR-020 and not recorded in the repository until now | — |

### Decision

**The installed hook may refuse a file-tool write, and only that, when Gritty
is running, enforcement was separately consented to, and a lease the core has
already published says the target belongs to another workstream. It refuses on
evidence the core wrote; it never refuses on its own inference, its own
failure, or Gritty's absence. It never allows.**

**1. One event, one verb, four tools.** Only `PreToolUse`, only for `Write`,
`Edit`, `MultiEdit` and `NotebookEdit` — the tools whose target is a named path
— and only `deny`. The hook never returns `allow` (the docs above: that can
skip the agent's own prompt, which would *widen* what the agent may do) and
never `ask`. Silence remains the answer to everything else, exactly as today.

**2. What refuses a write.** A target is refused when it lies inside a lease
that is held (not released, not `EXPIRED`), **not stale** (its workstream is
not `DONE` or `ABANDONED`, ADR-019 §6), and held by a workstream that is not
among the workstreams the session is linked to through T223's
`(provider, session_id)` link. Three consequences are deliberate:

- **An unlinked session is never refused.** A write nobody can attribute is not
  a violation (`T325-workspace-ownership.md:278-281`, T326 §3), and refusing
  every unlinked session would refuse the holder's own unattached terminal.
- **A stale lease still refuses a claim and no longer refuses a write.** A
  finished workstream holding ground is the "session died holding a lease"
  failure this repository has hit repeatedly; its lease guard stops enforcing
  after 8 h for the same reason (`AGENTS.md`, *Concurrent agent sessions*).
- **Your own ground is never refused**, as `acquire` already allows
  (`T325-workspace-ownership.md:73`).

**3. The core decides; the hook applies what it published.** The core writes one
file, the **fence**, to Gritty's app-data directory beside the verdicts
(`crates/agents/src/verdict.rs:179-186`): the canonical workspace root, and per
linked `(provider, session_id)` the targets that session may not write, each
with the holding workstream's name and lease id and a bounded refusal sentence
already composed. It is rewritten atomically whenever `OwnershipChanged` fires
(the whole map, per transaction — `T325-workspace-ownership.md:84-85`) or a link
changes, bounded in bytes and entries like every other file here, and removed
when enforcement is turned off or no workspace is open.

The hook reads the fence, relativises the tool's absolute target against its
root with `WorkspacePath`'s rules (ADR-010: a `..`, a backslash or a path
outside the root is *not* matched, never guessed), and tests it with the
**same** segment matcher ownership uses (`OwnershipTarget::covers`,
`crates/gritty-core/src/ownership/target.rs:91`). That matcher moves into a
leaf crate both link, rather than being copied: one matcher is ADR-019 §4's
point, and a second one would drift toward "not covered".

**4. §5, restated rather than dropped.** Every link fails open, and the exit
code is `0` on every path, as `crates/agent-hook/src/main.rs:22-36` already
guarantees:

1. Enforcement not consented (item 5 below) ⇒ today's behaviour, byte for byte.
2. Liveness marker missing or stale (`verdict.rs:240-257`, 4 s) ⇒ silence.
   **When Gritty is not running, nothing is enforced**: nobody could see why a
   write was refused or override it, and a wall with no door is worse than the
   observed ownership it replaces. Coordination plan §4.4 asks enforcement for
   "explicit behavior"; this is it.
3. Fence missing, unreadable, over its bound, malformed, of an unknown version,
   or naming a different root than the target lies in ⇒ silence.
4. Target unresolvable, outside the root, or not matched ⇒ silence.
5. A match ⇒ `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":…}}`
   on stdout, exit `0`. **Not exit 2**, which blocks regardless of what was
   printed: a bug in the refusal path must fail toward the write, not away
   from it.

The hook does not wait. There is no round trip to Gritty per write; the fence
is already on disk. The installed `PreToolUse` entry, which today carries no
`timeout` and so inherits 600 s (`crates/agents/src/integration.rs:688-695`),
gains a short explicit one, so a pathological stall costs seconds rather than
ten minutes — and per the docs a timeout lets the call through.

**5. Refusal is its own consent.** §1 covers observation. Enforcement is a
second, explicit action per provider, never implied by installing the hook,
never on by default. It is visible where §3 wants it: a flag on the installed
entry's command line, shown in the diff before it is applied, removed by
uninstall, and turning it off leaves observation installed and working.

**6. The refusal reaches the agent, the user, and a door.**

- **The agent** reads the reason, composed by the core, bounded, and made only
  of the target, the holding workstream's name and what to do: *work outside
  it, and do not write it by another route; ask the user to release or
  override the lease in Gritty.* A workstream name is user-authored text
  entering a model's context, so it is clipped and stripped of control
  characters like every other string here (hard rule 13); a lease's note never
  goes in.
- **The user** sees it because the hook records the refusal — target, lease,
  time — in the state file it already writes first
  (`crates/agent-hook/src/lib.rs:233-244`), and the core records it as an
  ownership conflict in a state of its own, distinct from `OBSERVED`: a write
  that did **not** happen is a different fact from one that did, and the
  queue must not word them alike (T326 §12). The state's name, and the minor
  protocol bump it costs, are T330-R's.
- **The door** is ADR-019 §8, unchanged: release, or an attributed
  `OVERRIDE`. The fence is rewritten, and the agent's next attempt succeeds.
  There is no "allow this once" and no answer in the terminal — an override
  that happens outside Gritty is one nobody attributed.

**7. One authority, not two.** The ownership refusal and the permission verdict
answer different questions and run in a fixed order. `PreToolUse` fires before
the permission prompt, so a refused write never reaches ADR-020's
`PermissionRequest` or T310-ANSWER's `can_use_tool` — **the user is never asked
to approve a write ownership will refuse.** Verdicts answer only what survives.
Ownership never answers either of those, and they never override a refusal.

Owned runs get the same fence and the same rule. `can_use_tool` cannot be the
enforcement point: it is asked only for calls the CLI gates, and "an `echo` in
the working directory ran with no ask at all" (`T310-ANSWER-host-permissions.md:30-32`).
How the hook is attached to a run — the user's installed entry if `PreToolUse`
fires under `-p`, or a `--settings` Gritty passes at launch
(`crates/agent-claude/src/launch.rs:122-136` fixes the line today) — is
T330-R's, after measuring. Until then an owned run's ground is observed.

**8. ENFORCED and OBSERVED survive as a per-session fact, not a mode.** A lease
is *enforced against a session* only when all of these hold: enforcement is
consented for its provider, Gritty's entry is installed and recognised, Gritty
is running, the session is linked, and the write goes through one of §1's four
tools. Anything else is observed. The UI says which, per session and next to
the lease; it never says a file is "protected" or "locked", and
`docs/architecture/ownership.md:17-26`'s line moves only when T330 ships, not
when this is accepted.

**§6, amended.** The hook still links nothing of `gritty-core`, knows no
`AgentSession`, no workstream and no protocol, and decides no meaning. It gains
a second Gritty-written file to read and a path matcher to run over it. What a
refusal *means* — who holds the ground, why, what the user may do — is written
by the core into the fence, the same split §6 draws for phases.

**§7, amended.** Hook-sourced *state* is still evidence, never authority. A
refusal travels the other way and is authority, and ADR-020's line holds for it
in a narrower form: the core does not synthesise a decision a person did not
take. A refusal is the mechanical application of two decisions a person did
take — claiming the ground, and turning enforcement on — to a path the agent
named. It is only ever a refusal.

### What is enforced, and what is not

| Write | Seen by `PreToolUse` | Refused | Detected afterwards (T326) |
|---|---|---|---|
| `Write` / `Edit` / `MultiEdit` / `NotebookEdit`, main conversation | yes | **yes**, under §8's conditions | yes |
| The same, inside a subagent | yes (docs) | **yes** | usually not (checkpointing) |
| A shell command that writes (`>`, `sed -i`, `mv`, a script, a formatter, `git checkout`) | as command text only | **no** | **no** |
| An MCP tool that writes | yes, in the server's own shape | **no** | no |
| A process the agent started earlier, git hooks, another editor | no | no | no |
| An unlinked session; Codex before T222; any agent without the hook | — | no | linked sessions only |
| Gritty not running | — | no | when it next reads the transcript |
| A second spelling on a case-insensitive volume, a symlink into owned ground | yes | no (ADR-019 §5, inherited) | no |
| `bypassPermissions` mode | — | *unverified* | — |

**The shell is the hole, and it is the likeliest route around a refusal**: a
model told it may not `Edit` a file can `cat >` it. The reason sentence says not
to; nothing makes it true. That is why §8 names the tools a lease is enforced
through rather than calling the lease enforced.

### Consequences

- **§5 narrows a second time.** After ADR-020: "the hook never blocks unless
  Gritty is alive and the agent was about to block on a human anyway". After
  this: it also refuses, without waiting, a write to ground the core published
  as another workstream's, while Gritty is alive. Every failure is still
  silence.
- **Every tool call pays a file read and a match.** The entry already runs on
  every `PreToolUse` with matcher `*` (`integration.rs:58-69`, `:699`), so the
  process launch is not new; the fence read is. T330-R measures the added cost
  before claiming "no slower".
- **A fence can lag the registry by one write.** A release not yet published
  refuses a write that is now allowed, and the agent retries; a claim not yet
  published lets one through, and T326 records it. The first is the cheaper
  error, which is ADR-019 §4's bias.
- **New files, one new wire value.** The fence (app-data, private, atomic,
  bounded); a refusal field in the hook state file (`HOOK_STATE_VERSION` moves);
  a conflict state. Integration status gains "enforcing" beside "installed".
- **The installer writes one more flag and one more `timeout`,** under §§2–4
  unchanged: appended, identified, shown, reversible.
- **`crates/agents/src/hook.rs` and `crates/agent-hook` stop being
  opinion-free by construction.** The opinion is still the core's and still on
  disk where a test can read it; the binary's new code is a relativise, a
  match and a print.

### Alternatives rejected

**Ask Gritty per write and wait for the answer.** A round trip on every file
tool call, which is exactly what ADR-020 declined when it chose
`PermissionRequest` over `PreToolUse`. The fence answers from disk in the
hook's own time.

**Return `ask` instead of `deny`.** It puts the override in the terminal, where
nobody attributes it (ADR-019 §8), and in `dontAsk` mode an ask becomes a
denial anyway (docs/en/permissions).

**Exit 2.** It blocks regardless of stdout, so a crash in the wrong place
becomes a refusal. Silence must stay the failure.

**Refuse shell writes by pattern.** `agent-lease.py` does
(`command_writes`, `:498-503`, and a literal-prefix match, `:573-590`), and
that is right for a developer convention whose refusal says "use a read-only
form". A product refusing `grep src/auth` because a regex saw a `>` trains
users to switch enforcement off, and still misses every script.

**Enforce when Gritty is not running.** Leases persist, so it is possible. It
refuses with no explanation anyone can open and no override anyone can take.

**Refuse writes from unlinked sessions.** It turns an unattached terminal,
including the holder's own, into a refused one.

**An OS-level lock or sandbox.** ADR-019 §1's reasons stand, and Claude Code's
own sandbox is the user's configuration to set, not Gritty's to write.

### Not decided here

Shell and MCP writes; Codex (T222 brings its event names, and its reply shape
belongs in the adapter); a one-shot exemption; per-lease enforcement as an
`OwnershipMode`; read leases; how a run attaches the hook (§7, T330-R); what
happens across worktrees, where T400 makes two workstreams unable to collide on
disk at all.

### Revisit when

A provider offers a pre-write hook that covers shell writes, or a sandbox API
Gritty may configure per run; T330-R's measurements show `PreToolUse` deny does
not hold under `bypassPermissions`, or does not fire under `-p`; the added cost
per tool call is measurable; false refusals turn out to be common, at which
point the fix is a better matcher, not a looser rule (T325's own remaining risk);
or T400 lands and the interesting conflict moves to merge time.
