# ADR-018: Gritty may install a hook into another tool's configuration, additively and with consent

- **Status:** Proposed
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
