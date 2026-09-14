# Task T120-R — `gritty-settings`: the registry, the file, and the failure policy

- **Phase:** 2
- **Status:** done
- **Depends on:** T120-C (the contract), ADR-031

## Objective

Build the service behind the contract: a new `gritty-settings` crate holding
the schema registry, a validating loader and a never-destructive writer; wire
it into the composition root; re-read the file when the window regains focus;
and make the agent notification switch durable without asking the operating
system for anything at startup, which is the ticket's headline fix.

## Motivation

ADR-031 §1 draws the line this crate implements: user configuration is not
application metadata, and it lives in a file so it stays recoverable when the
application will not start. Everything else here follows from that — the
strict "never rewrite a file that failed to parse" rule, the preserved unknown
keys, the atomic write.

T223's notification switch is the first real customer and the one that proves
the whole thing: it is a boolean the *core* acts on, on its own initiative,
which is exactly what no existing mechanism could store.

**Land together with T120-C.**

## Dependencies

`gritty-core` for the error envelope and the event bus. `serde_json` is already in
`[workspace.dependencies]` and the atomic write is reused from
`gritty-workspace`; **no new third-party dependency is needed**, and `members = ["crates/*"]` picks the directory up.

## Required reading

- `AGENTS.md` hard rules 9, 10, 11, 12, 13, 14
- `docs/decisions/ADR-031-settings-storage.md` — especially §5 (failure policy)
  and §7 (settings vs layout)
- `docs/decisions/ADR-014-session-persistence.md` — the failure policy this one
  is modelled on, and where it deliberately differs
- `crates/persistence/src/db.rs` — `DB_FILE_NAME`, and the corrupt-file
  handling this must **not** copy for settings
- `apps/desktop/src-tauri/src/core.rs:340-358` — `resolve_data_dir()`
- `apps/desktop/src-tauri/src/notifications.rs:100-170` — `permission()` versus
  `request_permission()`
- `docs/tasks/completed/T223-agent-attention.md` §2 and its follow-ups — why restoring
  "enabled" must not re-arm

## Allowed scope

- `crates/settings/` (new crate)
- `Cargo.toml` — one `[workspace.dependencies]` line (shared magnet: claim for
  the minute, release immediately)
- `crates/gritty-core/src/settings/mod.rs` — the handlers behind T120-C's wire
- `apps/desktop/src-tauri/src/{commands.rs,lib.rs,core.rs}` — adapters, registration
- `apps/desktop/src-tauri/src/notifications.rs` — reading the durable setting

## Explicit non-goals

- Any UI (T120-U), and the theme plumbing (T120-T).
- **The one-time seeding from the old blobs.** ADR-031 §8 puts it in the
  WebView: `preferences` and `layout` are UI-owned and opaque to the core
  (ADR-014), so the core interpreting one would invert the very ownership this
  work is drawing. It belongs to T120-U.
- A file watcher — see required behaviour 4.
- Workspace scope, and a `.gritty/` directory — forbidden by Refinements §14.
- A raw-JSON editing command. (`settings.reveal` belongs to T120-C.)
- Turning any currently hardcoded value into a setting beyond the migrated set
  in ADR-031 §7 plus `workbench.theme`. Record the rest as follow-ups.

## Required behavior

1. **The registry.** One declaration per setting — key, type, default,
   description, enum values with labels, `scope`, `applies`. It both validates
   writes in Rust and is the source `settings.describe` serializes. A setting is
   declared once.
2. **Load** is defaults, then the file overlaid. Unknown keys in the file are
   **kept** and written back verbatim.
3. **Write** reuses `gritty_workspace::write_file_atomic`
   (`crates/workspace/src/file.rs:217`) rather than growing a second atomic
   writer — it takes a plain absolute path, is already called for a file
   outside any workspace by `crates/agents/src/integration.rs:812`, and is
   already crash-tested through its `AtomicWriteFault` hooks (hard rule 2).
   Re-read immediately before writing so an external edit merges rather than
   being clobbered, under one lock held across the whole read-modify-write.
   **No debounce in Rust**: a settings write is a deliberate human act, and the
   only burst source is a number input firing per keystroke, which is the
   widget's problem to solve by committing on blur.
4. **Re-read on window-focus gain, not with a watcher.** ADR-031 §6 explains
   why: the data directory holds `gritty.sqlite-wal`, which ADR-014's writer
   touches every one to two seconds while any buffer is dirty, so watching it
   reproduces the noise ADR-025 exists to filter — plus an echo loop on
   Gritty's own writes. `on_focus_event`
   (`apps/desktop/src-tauri/src/lib.rs:275`) already exists. Re-read on the
   blocking pool, guarded by `(mtime, len)`, under the same lock as a write, and
   emit only on an actual difference with `Actor::SYSTEM`.
5. **`agents.notifications` becomes durable, without a startup prompt.** Add a
   separate arming function; do **not** route the restore through
   `AgentSessionManager::set_notifications`, which calls `request_permission()`
   (`crates/gritty-core/src/agent_session/mod.rs:1064`) and would therefore ask
   the operating system at application startup — the thing T223 §2 forbids in
   as many words. Arming touches nothing platform-facing. Granted → it works;
   revoked → the setting reads enabled-but-blocked and says so; the prompt
   arrives only from the user touching the control.
6. **Mark T223 decision 6 superseded** in `docs/tasks/completed/T223-agent-attention.md`,
   with a line saying why. Without it the next reader implements the forbidden
   thing believing it was the decision.

## Failure behavior

Per ADR-031 §5, and this is the part most worth testing:

- **Missing file** → defaults, and nothing is written.
- **Unparseable JSON** → defaults, a diagnostic with the parse position, and
  **the file is never rewritten**. Deliberately unlike the database, which is
  renamed aside and replaced: nobody hand-writes a database, and overwriting a
  file a human was editing destroys both their work and the property that made
  a file worth choosing.
- **Known key, invalid value** → that key alone falls back to its default with
  a diagnostic; its neighbours still apply.
- **No data directory** (`resolve_data_dir()` returned `None`) → defaults,
  read-only, reported. The editor still works.
- **Unwritable directory or permission denied** → the value applies in memory
  for the session, the write is reported as failed, and the failure is visible
  rather than silent.
- **A write that dies midway** leaves the previous file intact — that is what
  temp+rename buys, and hard rule 12 says prove it rather than claim it.

## Acceptance criteria

- Every ADR-031 §5 rule holds, each with a test.
- The notification setting survives a restart and does not re-prompt.
- An external edit to `settings.json` applies on the next focus gain.
- `pnpm check` green, no new warnings.

## Tests required

- Unknown keys survive a round trip through a write.
- A bad value for a known key falls back alone and does not poison neighbours.
- An unparseable file is **not** rewritten (assert the bytes are unchanged).
- A missing file is not an error and writes nothing.
- A write interrupted midway leaves the previous file intact — **forced, not
  reasoned about** (hard rule 12).
- Concurrent external edit + in-app write merges rather than clobbering.
- Arming `agents.notifications = true` from a stored value **never** calls
  `request_permission()` — assert against T223's existing fake notifier. This
  is the test that keeps §2 honest.
- A focus re-read during a write does not land a stale snapshot over a fresh
  one.

## Commands to run

```
cargo test -p gritty-settings
cargo test -p gritty-core
pnpm check
```

## Progress

**The crate is built and committed (`84ea4d2`); the wiring is not.**

Done: `crates/settings/` — the schema registry (ten settings), the document
(defaults overlay, per-key validation, preserved unknown keys, diagnostics),
the file layer (bounded read, the four file states, atomic write through
`gritty_workspace::write_file_atomic`), and the store (one lock across each
read-modify-write, all-or-nothing patches, `reload_if_changed` for the focus
path). 36 tests; `cargo clippy -p gritty-settings --all-targets -- -D warnings`
and `cargo fmt --check` both clean.

Not done, and blocked on `T120-C`: the handlers behind the wire, the
composition-root wiring, the focus-gain call site, `arm_notifications`, and
marking T223 decision 6 superseded. The crate is inert until then — nothing
depends on it, so it adds no command, event or type and spends no version.

The contract itself is waiting on the source-control session, which holds
2.22.0–2.24.0 and shares three of the four protocol seams; by agreement they
land first and this follows.

## Completion report

*(filled on completion)*
