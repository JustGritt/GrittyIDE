# ADR-031: Settings are a core-owned JSON file, and user configuration is not application metadata

- **Status:** Proposed
- **Date:** 2026-09-09

## Context

"Settings" has been Phase 2 scope since the project plan was written, in both
top-level documents and `docs/plans/roadmap.md`, and it was never given a
ticket. What exists instead is three unrelated `<details>` popovers — editor
preferences in the editor pane, shells and fonts behind the terminal's `⋯`, and
the agent hook control — plus a notification switch that resets at every
launch.

The repository does persist user-editable values. `session.update` stores two
opaque JSON blobs, `preferences` and `layout`, in the singleton `session` row
(ADR-014), and both parsers validate every field independently against its own
default. That mechanism is sound and is not being replaced. But it is
**UI-owned by contract** — `crates/gritty-core/src/session/wire.rs:1-6` says so
— and it therefore works only under one condition: *the core never acts on the
value by itself*. The terminal shell path satisfies it, because the WebView
passes the path to `terminal.spawn` as an argument. Editor preferences satisfy
it trivially, never leaving the WebView.

T223 hit the case that does not satisfy it, and recorded the failure precisely:

> **A durable settings surface**, which the notification switch is the first
> real customer for. Today the only user-facing settings that survive a restart
> are the editor preferences and layout blobs, which are UI-owned; a boolean the
> *core* acts on has nowhere to live.

Its notification switch is off at every launch, not by preference but because
storing "enabled" would have meant inventing settings persistence inside a
ticket that was not allowed to (rule 14). T046, T090 and T095 each recorded the
same dependency, and T097 — unstarted — already *requires* a settings toggle.

So the question this ADR answers is not "should there be a settings page". It
is **where a value the core acts on is allowed to live**, given that ADR-004
already said "SQLite for application metadata".

## Decision

### 1. User configuration is a different category from application metadata

ADR-004 governs **state the application maintains about itself** — open tabs,
window geometry, recovery snapshots, recent workspaces, durable operations.
That state is the application's own bookkeeping: the user never authors it,
never reads it, and would not know what to do with it.

**User configuration is authored by the user and belongs to the user.** It
follows them between machines, it is worth putting in a dotfile repository, and
— the deciding property — when a value in it is wrong, the user must be able to
fix it *without the application*. ADR-004 is not amended, because it never
claimed this category. The split is the same one VS Code draws between
`state.vscdb` and `settings.json`; Zed, Sublime and Neovim all land the same
way, and for the same reason.

### 2. Settings live in `settings.json` in the application data directory

`<data dir>/settings.json`, beside `gritty.sqlite`, resolved by the existing
`resolve_data_dir()` (`apps/desktop/src-tauri/src/core.rs:342`), which already
honours `GRITTY_DATA_DIR` for development and returns `Option` — no data
directory means defaults only, read-only, and a reported status, exactly as
persistence already behaves.

A flat, dotted key space (`editor.fontSize`, `terminal.shell`,
`workbench.theme`, `agents.notifications`), which is what makes a
schema-generated form and a search box possible.

The deciding argument is Gritty's own priority order,
`correctness → recoverability → inspectability → …`. A settings file you can
open in any editor when a bad value stops the application starting is strictly
more recoverable than a row in a database, and legible when nothing is running
at all. That is worth a second persistence mechanism; nothing else here would
have been.

### 3. A schema registry in Rust is the single source of truth

One table in `gritty-settings` declares every setting: key, type, default,
description, enum values with labels, `scope`, and `applies`
(`live | newTerminals | restart`). The same registry **validates writes in
Rust** and is **shipped to the UI** by `settings.describe`, which renders the
form from it.

A setting is therefore declared once. The alternative — hand-written forms —
means maintaining validation, search, reset-to-default and the JSON view twice,
and they drift. This is the one place a small amount of machinery buys more
than it costs; it is not a plugin system and not a framework layer, so hard
rule 9 is satisfied.

### 4. Scope is user-global, and `scope` exists anyway

v1 has exactly one scope. A repo-local `.gritty/` directory is **explicitly
forbidden** by Architecture Refinements §14 — *"Do not introduce this directory
until its ownership and Git behavior are defined"* — and is a stated non-goal
of T020.

The registry still carries a `scope` field from the first commit, so workspace
scope is additive rather than a migration. When it is specified it will need a
trust boundary before it needs a file format: repository content is untrusted
input (hard rule 13), and a workspace-scoped setting that can name an
executable — a shell path, an agent binary — is arbitrary code execution on
clone. That is the decision §14 is waiting for, and this ADR does not pre-empt
it beyond reserving the field.

### 5. Failure policy

Modelled on ADR-014's, and stricter in one place.

- **Missing file** → defaults, and **nothing is written**. An absent file is
  how "everything is default" is spelled; writing one on first launch would
  turn every future default change into a silent no-op for existing users.
- **Unparseable JSON** → defaults, a diagnostic carrying the parse position,
  and **the file is never rewritten**. This is the strict part and it is
  deliberate: a corrupt *database* is renamed aside and replaced, because
  nobody hand-writes a database, but a settings file that fails to parse almost
  always failed because a human was editing it. Overwriting it would destroy
  the work in progress and the very thing that makes a file recoverable. The
  settings page renders read-only over defaults and names the path.
- **Known key, invalid value** → that key alone falls back to its default and
  earns a diagnostic. One bad value never poisons its neighbours.
- **Unknown keys are preserved verbatim across a write.** An older build, or
  one whose feature was removed, must not eat a key it does not recognise.
- **Writes are atomic**, and reuse `gritty_workspace::write_file_atomic`
  (`crates/workspace/src/file.rs:217`) rather than growing a second one. It
  takes a plain absolute path — `crates/agents/src/integration.rs:812` already
  calls it for a file outside any workspace — and it is already crash-tested
  through its `AtomicWriteFault` hooks. Hard rule 2: an existing contract owns
  this responsibility.
- The file is **re-read immediately before a write** so an external edit is
  merged rather than clobbered; on a genuine conflict the last write wins,
  which is the right answer for single-user configuration.

### 6. Changes are an event, and the file is re-read on focus

An external edit applies without a restart — that is most of the reason the
format was chosen. It is **not** delivered by a watcher.

A watcher over the data directory would be a self-inflicted storm: that
directory holds `gritty.sqlite-wal`, which ADR-014's snapshot writer touches
every one to two seconds while any buffer is dirty. Watching it would
reproduce exactly the noise ADR-025 had to filter out for Gritty's own atomic
saves, and would additionally fire on Gritty's own settings write, which needs
an echo suppressor or it loops. The workspace watcher cannot serve this anyway
— it takes a `WorkspaceRoot`, and fabricating one over the user's application
data directory violates every path-boundary assumption in the tree.

Instead the file is re-read **on window-focus gain**, which is the shape of the
real workflow: edit in another editor, come back. `on_focus_event`
(`apps/desktop/src-tauri/src/lib.rs:275`) already exists and already pushes
focus into the core. The re-read runs on the blocking pool, is guarded by
`(mtime, len)` so a focus that changes nothing costs one `stat`, and takes the
same lock as a write so a stale read cannot land on top of a fresh one. A real
watcher, if it is ever wanted, belongs beside `watch()` inside
`gritty-workspace` as a file-grained `watch_file`, keeping `notify` contained
where ADR-025 put it.

One new `CoreEvent::SettingsChanged` carries **the whole effective set plus the
list of changed keys**. Carrying the whole set is not a hard rule 8 violation:
rule 8 is about volume and frequency, and this fires at human frequency with a
payload bounded by a static registry. It is also what every other converging
event in this codebase does, for a reason each of them states —
`OwnershipChanged(OwnershipMap)`, `AgentSessionsChanged(AgentRoster)`,
`WorkstreamChanged` — namely that a consumer which missed one event still
converges, where a delta-only event would leave it permanently wrong. The
`changed` list rides alongside so a consumer can cheaply ignore an event that
does not concern it. The event is emitted **only when the effective values
actually differ**, which is also what makes a theme applier unable to loop.

### 7. What stays in the UI-owned blobs

The rule, which exists so the next person does not have to guess: **is this
user intent that should follow the person, or is it the state of this window?**

Intent is a setting — editor preferences, terminal shell and fonts, theme,
notifications. Window state stays layout — pane sizes and the agents section's
fold. `layout` and `preferences` keep everything in the second category and are
not deprecated.

`explorer.showHidden` is a **setting by this rule** and is nonetheless deferred
out of v1 for scope reasons only: its wiring runs through files another ticket
is holding. It stays in the layout blob for now, and it is recorded here so
nobody has to re-litigate it the day a `files.exclude` setting arrives.

### 8. The seeding is the UI's job, not the core's

The one-time move of today's values into `settings.json` happens **in the
WebView**. ADR-014 makes `preferences` and `layout` UI-owned and opaque to the
core; having the core open the session row and interpret one would invert
exactly the ownership this ADR is drawing. The UI owns those blobs, so the UI
is what is entitled to translate them.

It needs no migration flag: a key that has already been written is reported as
explicitly set, so it is never seeded twice. A value equal to the schema
default is skipped rather than written, or every user who never chose a font
size would be pinned to today's default forever. One `set` is one atomic file
write, so the seeding either lands whole or not at all, and the blobs are left
intact either way — which is what makes retrying on the next launch safe.

The legacy fields keep being **written** for one release as a mirror, sourced
from the settings values and read by nothing but the seeder. Deleting them in
the same release that seeds would mean a seed that fails on a read-only data
directory loses the values permanently. Removing the mirrors is a follow-up,
gated on one release of `settings.json` being the source of truth.

## Consequences

- **There are now two places a user-editable value can live**, which is the
  real cost of this decision. §7's rule is the mitigation and it has to be
  applied deliberately; a settings-vs-layout mistake will not fail any test.
- **Settings are plaintext in the application data directory.** Nothing secret
  belongs in them — no tokens, no keys — and `docs/architecture/security.md`
  records it alongside the recovery snapshots, which are already plaintext
  there for the same reason.
- **`settings.json` cannot be opened as a document.** Every document path must
  resolve inside the workspace root (`crates/workspace/src/root.rs:93-100`), so
  the "Open Settings (JSON)" tab every comparable editor has is not free here.
  A raw editor means either a hole in a frozen security boundary or a Monaco
  surface over a core-served string; the second is the acceptable one, and it
  is a follow-up. `shell.openExternal` cannot stand in — it validates http and
  https only.
- **Revealing the file needs its own command.** `shell.revealPath` cannot be
  reused: its request takes a `workspaceId` and a `WorkspacePath` and resolves
  through the workspace root, so a data-directory path is `PermissionDenied` by
  construction. `settings.reveal` therefore hands the core-derived path
  straight to `ExternalOpener::reveal`, alongside the path being shown as
  selectable text. This **falsifies that trait method's doc-comment**, which
  today promises "the path has already been resolved against the workspace root
  and proven to be inside it" (`crates/gritty-core/src/shell.rs:36-39`). The
  comment must be rewritten to the real invariant — *core-derived, never
  renderer-supplied* — in the same change, or the next reviewer will correctly
  read `settings.reveal` as a boundary violation.
- **A user-selectable theme overrides T048's non-goal** ("the OS preference
  remains the only signal") and does the work ADR-016 costed. ADR-016's own
  "Revisit when" is *"a user-selectable theme is specified"*; T120-T amends it.
  Worth stating plainly: **the light palette is derived, not designed** —
  artboard 2a is dark-only — so offering Light exposes a surface nobody has
  drawn.
- **No `SCHEMA_VERSION` and no migration are spent**, deliberately. 5 and
  `0005` stay free.
- **T223 decision 6 is superseded**, and its task file must say so. Restoring a
  stored "on" must **not** run through `AgentSessionManager::set_notifications`,
  which calls `request_permission()`
  (`crates/gritty-core/src/agent_session/mod.rs:1064`) and would therefore ask
  the operating system for notification permission at application startup —
  the precise thing T223 §2 forbids. Arming from a stored value is a separate
  function that touches nothing platform-facing, with a test asserting
  `request_permission` is never called on that path. Without that note in
  T223, the next reader implements the forbidden thing believing it was the
  decision.
- **Four TypeScript constants become duplicates the day this lands** —
  `MAX_CUSTOM_SHELLS`, `TERMINAL_FONT_SIZE_RANGE`, `MIN_FONT_SIZE`/
  `MAX_FONT_SIZE`, `TAB_SIZES`. They must be **deleted and read from the
  schema**, not kept in step by hand. A bound that disagrees with Rust by one
  lets the UI offer a value the core then refuses, failing the whole patch with
  an error naming a field the user did not touch.
- **A per-setting `applies` hint is needed and is not automatic.** A terminal
  shell change reaches the next session, not the running one. Saying so in the
  UI is cheaper and more honest than pretending every setting is live.

## Alternatives rejected

- **A typed SQLite table** (migration `0005`). Reuses the persistence crate's
  corruption, backup and failure policy, and reads ADR-004 most literally. It
  loses the one property that decided this — a settings store you can repair
  when the application will not start — and gains nothing the file lacks for a
  single-user, low-frequency, small document.
- **Extending the existing UI-owned blobs.** By far the cheapest: no protocol
  bump, no migration, and the 500 ms coalescing writer already exists. It does
  not close the gap. The core still cannot read a value it must act on, so
  T223's switch would still reset at every launch — and having the core read a
  blob the contract calls UI-owned inverts the ownership rather than answering
  the question.
- **A dotfile in `$HOME`** (`~/.grittyrc`). Familiar, but it puts the file
  outside the directory `GRITTY_DATA_DIR` redirects, which would make crash
  tests and development instances share one user's configuration.
- **Workspace-scoped settings now.** Forbidden by Refinements §14 until
  ownership and git behaviour are defined, and it needs a trust boundary first
  (§4).
- **A module inside `gritty-core` rather than a crate.** Defensible, and it is
  what ADR-016 argued for `tokens.css`: one consumer does not justify a
  package. It lost narrowly to the `gritty-persistence` precedent — a storage
  domain that "knows nothing about the core's types", takes and returns plain
  values, and can therefore be tested without constructing a core. The settings
  document has exactly that shape, and keeping it outside means the validation
  and failure-policy tests never need the event bus. If it ever grows a
  dependency on core types, it should move rather than reach.
- **A `version` field in the file.** Both existing blob parsers already write
  one and never read it. Per-key validation plus preserved unknown keys is
  strictly stronger than a version nobody checks, so this file has none —
  recorded here so it is not added back out of habit.

## Revisit when

Workspace scope is specified — which needs a trust decision, not a format —
or a raw-JSON editing surface is wanted, or a setting appears that the schema
registry cannot describe.
