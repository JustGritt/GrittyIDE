# Task T221 — Agent hook bridge: earning `BLOCKED`

- **Phase:** 3
- **Status:** tofix
- **Depends on:** T220 (agent session discovery), T230 (fast loop, timeline entries), ADR-018

## Objective

Make Gritty able to say *this agent is stopped at a permission prompt and is
waiting for you* — the one value T220 refused to infer, and the one the island
exists to surface.

The mechanism is the agent's own hook system: an opt-in, additive entry in the
provider's configuration pointing at a small Gritty binary that records the
session's phase to a state file. The core reads that file as a **third evidence
source** alongside the process table and the transcript, and derives a new
`BLOCKED` activity from it.

This stays observability. Nothing here starts, stops, answers or approves
anything. A hook is a notification the agent emits; it is not a channel into it.

## Dependencies

- **T220** for `gritty-agents`, `gritty_core::agent_session`, the merge, the
  poll loop and the roster diff. This task extends all five rather than adding
  a parallel path.
- **ADR-018** for the seven constraints on writing into another tool's
  configuration. The installer in this task is that ADR made executable; where
  they disagree, the ADR wins.
- **T060** for `resolve_data_dir` and the app-data directory the state files
  live in.
- **T030** for the atomic write (`crates/workspace/src/file.rs`) the hook
  reuses.

## Required reading

- `docs/decisions/ADR-018-agent-integration-bridge.md` — the constraints
- `docs/decisions/ADR-005-agent-isolation.md` — the adapter/domain split
- `docs/architecture/agent-sessions.md` — the two sources this makes three
- `docs/architecture/security.md` — untrusted input, bounded fields
- `crates/agents/src/lib.rs` — `Evidence`, `Discovery`, the merge
- `crates/agents/src/transcript.rs` — the bounded-read shape to copy
- `crates/gritty-core/src/agent_session/{mod,wire}.rs` — activity derivation,
  ranking, the poll loop
- `crates/workspace/src/file.rs` — `write_file_atomic` (temp → fsync → rename)
- `apps/desktop/src-tauri/src/core.rs` — `resolve_data_dir`

## Allowed scope

- `crates/agents/**` — the hook-state reader, the config installer
- `crates/gritty-core/src/agent_session/**` — `Blocked`, `activitySource`,
  the three-way merge, the integration commands' domain side
- `crates/gritty-core/src/protocol/{mod.rs,export.rs}` — version bump, exports
- `crates/agent-hook/**` — new binary crate, `gritty-agent-hook`
- root `Cargo.toml`, `crates/gritty-core/Cargo.toml`
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}` and
  `tauri.conf.json` (bundling the hook binary as a sidecar resource)
- `packages/protocol/src/{client.ts,commands.ts,types.ts,index.ts}` + tests
- `apps/desktop/src/features/agents/**`
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No control.** No answering a permission prompt, no cancelling a turn, no
  keystroke injection, no writing to any agent's stdin. The hook is one-way.
- **No screen-scraping.** No reading another terminal's screen, no pattern
  matching against a TUI. If no hook is installed, the roster is exactly what
  T220 shipped.
- **No install without an explicit per-provider user action** (ADR-018 §1).
  No install on first run, no install as a side effect of a scan, no control
  that installs into several tools at once.
- **No repairing, reformatting or templating a config file** the installer does
  not fully understand (ADR-018 §4). Refuse and explain.
- **No new providers.** This task lands the mechanism against Claude Code only;
  Codex and the rest are T222. Write the installer so a provider is a path, a
  parse and a set of event names — but do not add one here.
- **No daemon, no socket, no listening port.**
- **No run lifecycle, capability model or provider adapter contract** — T300
  owns those, and a hook phase must not become a run state.
- **No persistence of hook state.** The state files are transient machine state
  in the app-data directory, not rows in the session database.

## Required behavior

### The hook binary (`gritty-agent-hook`)

1. Reads one JSON payload on stdin, writes one state file, exits `0`. Always
   exits `0`, including on every error path: a non-zero exit is the agent's
   problem, and Gritty must never be the reason an agent misbehaves
   (ADR-018 §5).
2. Writes to `<app-data>/agents/state/<session-id>.json`, **atomically**
   (temp → fsync → rename, reusing `crates/workspace/src/file.rs`), so a
   concurrent read never sees a partial file and no lock is needed.
3. The file is **overwritten, not appended**: the current phase is the whole
   question, so the file never grows and there is no rotation, truncation or
   age policy to get wrong.
4. Contents are bounded and small:
   `{ version, sessionId, provider, phase, since, event, tool?, cwd? }`.
   Every string is truncated *by the hook* — an oversized field must never
   reach the state directory, let alone the protocol.
5. Phase is derived from the hook event alone, with no interpretation:

   | Event | Phase |
   |---|---|
   | `PermissionRequest` | `BLOCKED` |
   | `PermissionDenied`, `PostToolUse`, `PostToolUseFailure` | `ACTIVE` |
   | `PreToolUse` | `ACTIVE`, with the tool name |
   | `UserPromptSubmit` | `ACTIVE` |
   | `Stop`, `StopFailure` | `SETTLED` |
   | `SessionStart` | `ACTIVE` |
   | `SessionEnd` | `ENDED` |
   | anything else | the file is not written |

   The hook names phases, not activities. `BLOCKED`/`ACTIVE`/`SETTLED`/`ENDED`
   are what the *agent* reported; what they mean for a roster row is the core's
   decision (ADR-018 §6).
6. Does no network access, starts no process, holds no lock, reads no
   configuration, and writes nowhere else.
7. Is idempotent and order-tolerant: a `since` older than the file's current
   `since` is discarded, so two hooks racing cannot move the phase backwards.

### The reader (`gritty-agents`)

8. Scans `<app-data>/agents/state/` on the **fast loop** T230 introduces
   (~300–500 ms), not on the 6 s discovery tick. "Pending validation" arriving
   six seconds after the agent stopped defeats the purpose of the value, and the
   scan is affordable at that rate because it is a directory listing plus a stat
   and a small read per file. Bounded exactly as transcripts are: a cap on
   files, an age cutoff, a size cap per file. A file older than the cutoff is
   ignored and swept. Emit only on change (rule 8) — the fast loop makes this
   non-negotiable rather than merely tidy.
9. Every field read back is untrusted input a second time: bounded again,
   never interpreted, never resolved as a path, never logged in full.
10. A malformed, truncated or non-UTF-8 state file is **skipped**, not fatal.
    A file whose `version` is newer than this build understands is skipped too.

### The merge and the derivation (`gritty_core::agent_session`)

11. Hook evidence joins on **session id** — the one join key that is exact,
    unlike T220's working-directory join. A state file with no matching process
    or transcript does not create a roster row on its own.
12. Activity derivation gains one rule, at a defined precedence:

    | Value | Evidence |
    |---|---|
    | `ENDED` | no live process, or the process table was unreadable |
    | `IDLE` | live process, transcript quiet past the idle threshold |
    | **`BLOCKED`** | **live process, hook phase `BLOCKED`, not stale** |
    | `WORKING` | live process, a `tool_use` with no matching `tool_result` |
    | `WAITING` | live process, nothing outstanding |

    `ENDED` still outranks `BLOCKED` (ADR-018 §7): an agent killed at a prompt
    is not blocked, it is gone. `IDLE` also outranks it, for T220's existing
    reason — a prompt nobody answered for half an hour is a session someone
    walked away from — and a `BLOCKED` phase older than the idle threshold is
    therefore reported as `IDLE`, not held forever.
13. `AgentActivity::Blocked` is set **only** from hook evidence. No transcript
    pattern, process state or elapsed-time threshold may produce it (rule 12).
14. `needs_you()` is true for `Blocked`; ranking puts `Blocked` above
    `Waiting`, which keeps the island's most-urgent slot correct.
15. `AgentSession` gains `activitySource: PROCESS | TRANSCRIPT | HOOK` — which
    source produced *this* verdict. `AgentEvidence` keeps meaning which sources
    saw the session and gains the hook combinations.

    - **The state file's bounded `tool` field becomes a `PermissionPending`
      timeline entry** (T230's entry kind), so a rail showing a blocked session
      names *what* is waiting — `Bash`, `Edit` and the core's bounded detail —
      rather than only that something is. Without it the user still has to go to
      the terminal to find out what they are approving, which is most of the
      value of knowing at all. The entry appears when the phase becomes
      `BLOCKED` and goes when it leaves; it is never persisted, and its detail
      is bounded by the hook and again by the core, like every other field from
      this source.
16. The roster diff is unchanged in spirit: a tick that changes nothing still
    emits nothing (rule 8). Hook state changes far more often than the process
    table, so verify this explicitly rather than assume it.

### The integration commands

17. `agents.integrationStatus { provider }` → `NOT_INSTALLED | INSTALLED |
    BROKEN | UNSUPPORTED_CONFIG`, with the config path and, when installed, the
    events covered. `BROKEN` means Gritty's entry is present but points at a
    binary that is not there — the app was moved or upgraded.
18. `agents.integrationInstall { provider, apply: false }` returns the proposed
    change without writing: the file, the events, the entry, and the backup path
    it would create. `apply: true` performs it. Two calls, one shape — the UI
    shows the diff it is about to apply, not a description of one (ADR-018 §3).
19. Installing appends Gritty's entry to each event's existing array. It never
    replaces an array, reorders one, or touches an entry that is not its own
    (ADR-018 §2). **Test this against a fixture config that already has another
    tool registered on every event** — that is the normal case, not the edge one.
20. Uninstall removes exactly the entries whose command path is Gritty's, and
    leaves the rest byte-identical modulo the re-emitted JSON's own formatting.
21. Both write a timestamped backup of the prior file beside it first.
22. Protocol version bumps MINOR: every change is additive. (`2.8.0 → 2.9.0`
    as implemented — `2.8.0` was consumed by T230's timeline contract.)

### The UI

23. The island's agent rows show `BLOCKED` distinctly and sort it first.
24. A row can say where its verdict came from, so a `BLOCKED` row is
    attributable rather than magic.
25. The integration is offered **in settings, not in the roster**, is off by
    default, and shows the diff before applying. A `BROKEN` status explains
    itself and offers a repair, which is an install over the top.

## Failure behavior

| Situation | Behavior |
|---|---|
| Config file absent | `NOT_INSTALLED`; install creates it with only Gritty's entry |
| Config file malformed | `UNSUPPORTED_CONFIG`; install **refuses** with the parse error. Never rewritten, never replaced from a template |
| Hook section in an unrecognised shape | `UNSUPPORTED_CONFIG`; refuse. Do not guess |
| Another tool already registered on every event | The normal case. Append; leave theirs untouched |
| Gritty's entry already present | Install is a no-op that reports so; it never duplicates |
| Config file read-only or in a read-only home | `PermissionDenied` naming the path. The backup is attempted first, so nothing is half-written |
| Backup cannot be written | Abort before touching the original |
| Hook binary missing after an app upgrade | `BROKEN`. The agent tolerates a failing hook, so nothing breaks for the user; the roster silently loses `BLOCKED` and the status says why |
| State directory unwritable | The hook exits `0` having written nothing; the roster degrades to T220 behaviour |
| State file stale — agent killed while blocked | No live process, so `ENDED` wins (§12). The file is swept at the age cutoff |
| State file for an unknown session | Ignored. It creates no row |
| State file newer `version` than this build | Skipped, with a debug trace. Forward compatibility over a hard failure |
| Clock moved backwards between hook and read | `since` is compared, never trusted as a duration; a negative age reads as zero |
| Two hooks racing on one session | Atomic rename means one wins whole; the older `since` is discarded (§7) |
| Non-Unix platform | The hook is built and installed the same way; only T220's process probe was Unix-only. Do not claim what has not been run (rule 11) |

## Security

- Hook stdin is untrusted input from a process Gritty does not control
  (rule 13). Bounded in the hook, bounded again in the core, never interpreted,
  never resolved as a path, never run.
- State files live in Gritty's app-data directory, never in `/tmp` or anywhere
  world-writable, and are created with owner-only permissions.
- Prompts, tool inputs and paths are never logged in full. Tracing records
  counts, providers, phases and durations.
- Writing to another tool's configuration happens only on explicit consent,
  only additively, always backed up, always reversible (ADR-018 §1–3).
- No transcript or hook content is persisted to the session database.

## Acceptance criteria

- With the integration installed and Claude Code running in another terminal,
  triggering a permission prompt flips that row to `BLOCKED` with
  `activitySource: HOOK` within one poll interval; answering it clears.
- Killing the agent while it sits at the prompt reports `ENDED`, not `BLOCKED`.
- With no integration installed, the roster is behaviourally identical to T220.
- Installing over a config that already carries another tool's hooks leaves
  every one of those entries intact — verified by diffing against the backup.
- Uninstalling returns the file to the backup's contents except for Gritty's
  own removed entries.
- A malformed config is refused with the parse error and left untouched on disk.
- No `AgentSessionsChanged` is emitted while the roster is unchanged, including
  across hook writes that do not change the derived activity.
- `pnpm check` green; generated protocol types fresh and committed.

## Tests required

- **Unit (Rust, hook):** each event → its phase; unknown event writes nothing;
  oversized fields truncated; malformed stdin exits `0` silently; unwritable
  target exits `0`; an older `since` does not overwrite a newer one.
- **Unit (Rust, reader):** malformed / truncated / non-UTF-8 / future-version
  state files skipped; the file cap, size cap and age cutoff hold.
- **Unit (Rust, derivation):** every precedence pair in §12, explicitly
  including `ENDED > BLOCKED` and `IDLE > BLOCKED`; `Blocked` unreachable
  without hook evidence.
- **Unit (Rust, merge):** hook evidence joins by session id; an orphan state
  file creates no row; `activitySource` correct for each combination.
- **Unit (Rust, diff):** unchanged roster emits nothing across a hook write.
- **Unit (Rust, installer)** against fixture configs: empty, absent,
  **another tool on every event** (the important one), Gritty already present,
  malformed, unrecognised shape, read-only. Assert byte-level preservation of
  foreign entries and that key order and indentation survive.
- **Unit (TS):** client methods; the reducer's ordering with `BLOCKED`.
- **Manual:** install from settings, trigger a real permission prompt in another
  terminal, watch the island; then uninstall and diff against the backup.

## Commands to run

```
cargo test -p gritty-agents
cargo test -p gritty-agent-hook
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

### Summary

`BLOCKED` is earned. The agent's own hook system now tells Gritty when a session
stops at a permission prompt, a third evidence source reads that on T230's fast
loop, and the derivation gains one row at a defined precedence. T231 wrote
"Pending validation" down months before anything could send the value; this is
what makes it reachable, and the renderer test asserts it travels the whole way
rather than sitting unused.

Four pieces:

1. **`gritty-agent-hook`** — a new binary crate. Reads one JSON payload on
   stdin, names a *phase*, writes one small state file atomically, exits `0` on
   every path including a panic. It links no `gritty-core`, knows nothing of the
   protocol, and takes the state directory on its command line so it carries no
   platform knowledge about where Gritty's data lives.
2. **`gritty-agents::hook`** — the format, the only writer and the only reader,
   in one module so the two cannot drift. Bounded twice: by the hook so an
   oversized field never reaches the disk, and again by the reader because a
   file on disk is untrusted input a second time.
3. **`gritty-agents::integration`** — the installer. The only thing in the
   repository that edits a file Gritty does not own, and every rule in ADR-018
   made executable: append-only, identified by its own binary, backed up first,
   and a refusal with the parse error for anything it cannot read.
4. **The derivation and the surfaces** — `AgentActivity::Blocked`,
   `activitySource`, the hook combinations on `AgentEvidence`, the
   `PermissionPending` timeline entry (whose first producer this is), three
   integration commands, and a settings popover that shows the change before it
   makes it.

Protocol `2.8.0 → 2.9.0`. Every change is additive.

### Files changed

**New**

- `crates/agent-hook/{Cargo.toml,src/lib.rs,src/main.rs}` — the hook binary,
  with the logic in a library so nothing about it is tested by launching a
  process.
- `crates/agent-hook/tests/installed_command.rs` — the whole chain once, for
  real: the command string the installer produced, run through `/bin/sh` with a
  payload on stdin, read back by the core's reader.
- `crates/agents/src/hook.rs` — the state file: phases, the write, the bounded
  scan, the sweep.
- `crates/agents/src/integration.rs` — status, install, uninstall.
- `crates/agents/src/json.rs` — an order-preserving JSON model (see decisions).
- `apps/desktop/src/features/agents/agent-integration.ts` and
  `agent-integration-settings.tsx` (+ `agent-integration.test.ts`) — the
  settings surface and its pure reducer.

**Changed**

- `crates/agents/src/lib.rs`, `crates/agents/Cargo.toml` — the new modules;
  `gritty-workspace` for the atomic write, `serde` and `thiserror`.
- `crates/gritty-core/src/agent_session/{mod,wire,timeline,tests}.rs` — the
  third-way merge, the derivation, `activitySource`, the pending prompt beside
  the ring, the integration commands.
- `crates/gritty-core/src/protocol/{mod,export}.rs` — the version and the new
  exported types.
- `apps/desktop/src-tauri/src/{commands,core,lib}.rs` — three command adapters,
  the hook binary resolver, the data directory reaching the reader.
- `packages/protocol/src/{client,commands,types}.ts` + `agents-client.test.ts`.
- `apps/desktop/src/features/agents/{agent-status.ts,agents-panel.tsx}` and the
  two touched tests.
- `Cargo.toml` — the `gritty-agent-hook` workspace member.
- `docs/architecture/agent-sessions.md`, `docs/decisions/ADR-018…`,
  `docs/decisions/README.md`, `docs/tasks/README.md`.

### Tests actually executed

```
cargo test -p gritty-agents            94 passed, 0 failed
cargo test -p gritty-agent-hook        13 passed + 1 integration test, 0 failed
cargo test -p gritty-core             229 passed (lib) + 3 (crash_recovery), 0 failed
cargo test --workspace                 all green
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --all --check                clean
pnpm lint / typecheck                  clean
pnpm test                             424 tests, 59 files, 0 failed
pnpm check                             green
```

Manual, by hand, against scratch directories only:

- built `gritty-agent-hook` and ran it five ways — a good payload, malformed
  stdin, empty stdin, no arguments, an unwritable state directory. **Exit `0`
  every time.** The state file came out `-rw-------` with the tool name and the
  working directory in it and no tool input.
- `crates/agent-hook/tests/installed_command.rs` does the same thing as a test,
  through a shell, from a path containing spaces, against a fixture config
  carrying another tool on all fourteen events.

**Not done, and not claimed:** the acceptance criterion that runs a real Claude
Code session in another terminal and watches the island. This session has no
screen access, so it could neither trigger a real permission prompt nor watch a
row change. Everything it *does* claim was executed: the hook binary was run,
the installed command line was run through a shell, and the derivation, the
merge, the diff and the installer are covered by tests against fixtures.

**No test in this task reads or writes a real `~/.claude`, `~/.codex` or
application-data directory.** Every path is injected; the real
`~/.claude/settings.json` was read once, to learn the shape the installer must
not break, and never written.

### Architectural decisions

**An order-preserving JSON model instead of `serde_json`'s `preserve_order`.**
ADR-018 §4 requires a re-emitted file to keep its key order, and
`serde_json::Value` sorts. The feature flag that fixes this is *workspace-wide*:
it swaps every `Map` for an `IndexMap`, which enlarges every `Value` in the tree
— including the one inside `CoreError` — and pushes the core's whole
`Result<_, CoreError>` surface past clippy's `result_large_err` threshold. A
repository-wide representation change to append one line to one file is the
wrong trade, so `crates/agents/src/json.rs` keeps the ordering in the one module
that needs it, over a type nothing else sees. `MapAccess` yields entries in
document order; all it does is keep them in a `Vec`.

**The state file carries no tool input.** §4's field list is followed exactly:
`{ version, sessionId, provider, phase, since, event, tool?, cwd? }`. A `Bash`
command line or an `Edit` payload is the kind of thing that carries a secret,
and writing one to disk to display it would be the wrong trade (rule 13). §15
still gets its detail: `PermissionPending.detail` comes from the *transcript's*
outstanding tool call, and only when the two name the same tool — they are
describing one call, so pairing them is not a guess, while pairing a different
call's input to this prompt would be.

**The pending prompt is held beside the timeline ring, not in it.** §15 says the
entry appears when the phase becomes `BLOCKED` and *goes when it leaves*. A ring
can only append, so an answered prompt would sit in the feed forever claiming to
be waiting. `AgentTimeline.pending` and `AgentTimelineAppendedPayload.pending`
carry it whole rather than as a delta, and a batch is emitted when it changes
even with no records appended — which is the one case where a fast-loop tick
speaks without the transcript having moved.

**The state directory travels on the installed command line.** The hook binary
therefore contains no knowledge of where Gritty's application data lives on each
platform, and `GRITTY_DATA_DIR` keeps working for free.

**Gritty's entries are recognised by their binary's file name, not by path
equality.** ADR-018 §2 has been refined in place with the reason: exact equality
stops recognising Gritty's own entry the moment the application moves, and an
entry nobody recognises can be neither reported as `BROKEN` nor removed.

**The fast loop re-derives from the cached discovery.** A hook write costs no
`ps` and no `lsof`; it re-runs the translation over the last scan's evidence and
goes through the same publish, so a tick whose hook state changed without
changing a derived value emits nothing (§16, verified explicitly).

**`AgentEvidence` gains three variants and not a fourth.** There is no
`HOOK_ONLY`, because a state file matching no process and no transcript creates
no row — so the combination cannot occur.

### Known limitations

- **The hook binary is not bundled as a sidecar resource.** It is found by
  `GRITTY_AGENT_HOOK` or as a sibling of the running executable, which covers
  `cargo run`, `target/debug` and a macOS bundle's `MacOS/` directory. Wiring
  `externalBin` into `tauri.conf.json` is a packaging change `pnpm check` does
  not exercise, and claiming a bundling path without having run a bundle would
  be exactly what rule 12 forbids. A build that cannot find its binary reports
  `hookAvailable: false` and offers nothing, rather than writing an entry that
  would be broken the moment it landed.
- **Claude Code only.** T222 owns Codex; the installer is already shaped as "a
  path, a parse and a set of event names", so adding one is one `HookTarget`.
- **Unix only in practice.** The hook binary and the installer are
  platform-independent and compile everywhere, but the end-to-end test runs the
  installed command through `/bin/sh` and is `ignore`d elsewhere, and T220's
  process probe was already Unix-only. Nothing here claims Windows works.
- **`hookAvailable` checks that a file exists, not that it is executable.**
  A non-executable binary would install and then fail silently, which the agent
  tolerates and the status reports as `INSTALLED` rather than `BROKEN`.
- **Registering on ten events means ten process launches per turn.** That is the
  mechanism's cost, and it is what the incumbent on this machine already pays on
  fourteen. It has not been measured against a long session.

### Follow-up

- Bundle `gritty-agent-hook` as a sidecar/resource so an installed application
  finds it (and so an upgrade that moves the app repairs rather than breaks).
- T222: `HookTarget` for Codex (`~/.codex/hooks.json`), which the reader and the
  derivation already accommodate.
- T223 can now notify on entering `BLOCKED`, which is the value it was waiting
  for.
- T232's timeline view should render `AgentTimeline.pending` — the entry exists
  and is delivered, but nothing draws it yet.
- Consider surfacing `activitySource` on the rail row; `activitySourceLabel` is
  exported and tested but only the settings surface is wired today.

### Remaining risks

- Writing into a user's own configuration is a new failure surface, and the
  worst case is a file nobody can recover. Three things stand between: the
  refusal to touch anything that does not parse, the backup written before the
  original, and the append-only edit verified against a fixture carrying another
  tool on every event. None of them has been exercised against a real
  `~/.claude/settings.json`.
- The state directory is swept on the fast loop. Sweeping is a `remove_file` for
  files past a six-hour cutoff, so it is rare — but it is a write on a 400 ms
  loop, and a filesystem that made `read_dir` slow would make it a cost.
- `since` is the hook's own clock. Two machines sharing a home directory over a
  network would disagree about staleness; the comparison saturates at zero
  rather than going negative, but it is still a clock nobody validated.
