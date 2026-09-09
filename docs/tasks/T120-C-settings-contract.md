# Task T120-C — The settings contract: `settings.*` and `SettingsChanged`

- **Phase:** 2
- **Status:** review
- **Depends on:** ADR-031 (the storage decision), T010 (the protocol envelope)

## Objective

Put the settings domain on the wire, as contract only: five commands
(`settings.describe`, `settings.get`, `settings.set`, `settings.reset`,
`settings.reveal`), the schema and value types they carry, and one
`CoreEvent::SettingsChanged`. Spends `PROTOCOL_VERSION` **2.25.0**, claimed in `docs/merge-queue.md`.

## Motivation

Nothing the *core* acts on can be configured today. T223 recorded the gap in
its own follow-ups — "a boolean the core acts on has nowhere to live" — and its
notification switch is off at every launch because of it. T046, T090 and T095
each recorded the same dependency, and T097 requires a settings toggle before
it can start.

ADR-031 decided where such a value lives. This ticket is the wire it reaches
the UI through, and it is deliberately separate from the service behind it so
the contract can be reviewed on its own.

**Land this together with T120-R.** The board's note on T102-C applies verbatim:
a registered command name with no handler behind it is not a state worth
landing.

## Dependencies

The protocol envelope, the error envelope and the event bus from T010; nothing
about the operation model is extended, because every settings command is a
bounded read or a small write and none of them is long-running.

## Required reading

- `AGENTS.md` hard rules 1, 2, 8, 9, 13
- `docs/decisions/ADR-031-settings-storage.md` — this ticket's decision
- `docs/decisions/ADR-009-core-protocol-conventions.md` — the versioning rule,
  and that additive changes bump MINOR
- `docs/merge-queue.md` — rule 2, the 2.25.0 claim already written there, and
  the `CoreEvent` seam warning
- `crates/gritty-core/src/git/wire.rs` — the house shape for a domain's wire types
- `crates/gritty-core/src/session/wire.rs:1-6` — why `preferences`/`layout` are
  opaque, and why this domain is not

## Allowed scope

- `crates/gritty-core/src/settings/` (new: `mod.rs`, `wire.rs`)
- `crates/gritty-core/src/protocol/{mod.rs,export.rs}` — the version, the rows
- `crates/gritty-core/src/event/mod.rs` — the variant, `name()`, `operation_id()`
- `crates/gritty-core/src/session/tap.rs` — the exhaustive match
- `packages/protocol/src/{commands.ts,client.ts}` and the generated directory
- `docs/merge-queue.md` (record 2.25.0 as spent)

## Explicit non-goals

- **The service behind it** — that is T120-R. This ticket may land a handler
  that reports the layer unavailable, but it does not read or write a file.
- **`settings.readRaw` / `settings.writeRaw`.** ADR-031 defers the raw editing
  surface; adding the commands before the surface exists is the state T101-C
  warns against.
- Workspace scope. The `scope` field exists and only ever reads `user`.
- Any UI.

## Required behavior

1. **`settings.describe`** returns the schema: an ordered list of sections,
   each with an ordered list of settings carrying key, type, default,
   description, optional enum values with labels, `scope`, and `applies`
   (`live | newTerminals`). This is what the UI renders a form from,
   so it must be complete enough to do that without a second lookup.
2. **`settings.get`** returns the effective values, the set of keys that differ
   from their default, any diagnostics, the absolute path of the file, and
   whether it is writable.
3. **`settings.set`** takes `{ values: { key: JsonValue } }` — a **patch**, not
   a whole document, so two panels editing different keys cannot clobber each
   other. Returns the new effective values and diagnostics.
4. **`settings.reset`** takes `{ keys?: string[] }`; omitting `keys` resets
   everything. Returns the same shape as `set`.
5. **`settings.reveal`** shows the file in the system file manager. It needs
   its own command: `shell.revealPath` takes a `workspaceId` and a
   `WorkspacePath` and resolves through the workspace root, so a data-directory
   path is `PermissionDenied` by construction. This one hands the core-derived
   path straight to `ExternalOpener::reveal` — and **rewrite that trait
   method's doc-comment** (`crates/gritty-core/src/shell.rs:36-39`), which
   currently promises the path was resolved against a workspace root. The real
   invariant is *core-derived, never renderer-supplied*.
6. **`CoreEvent::SettingsChanged`** carries the **whole effective set plus the
   changed keys**. Not a hard rule 8 violation: that rule is about volume and
   frequency, this fires at human frequency with a payload bounded by a static
   registry, and it is what every other converging event here already does —
   `OwnershipChanged`, `AgentSessionsChanged`, `WorkstreamChanged` — so a
   consumer that missed one still converges. Emitted **only when the effective
   values actually differ**.
7. Command naming follows ADR-009: `settings.set` in TypeScript,
   `settings_set` in Rust, one `request` object per command deserialized by the
   adapter so a malformed request is `InvalidInput`.
8. `PROTOCOL_VERSION` becomes `2.25.0`.

## Failure behavior

- A malformed request is `InvalidInput` from the adapter, before any handler runs.
- An unknown key in `set` or `reset` is `InvalidInput` naming the key — not a
  silent no-op, and not a write that stores something nothing will ever read.
  (This is distinct from an unknown key already *in the file*, which ADR-031
  says is preserved; a client asking for one is a bug, a file containing one is
  a downgrade.)
- A value of the wrong type for a known key is `InvalidInput` naming the key
  and the expected type.
- With no service behind it (before T120-R, or with no data directory), every
  command reports the layer unavailable in the envelope's terms and nothing
  crash-loops — the same shape `session.*` uses when persistence is
  unavailable.

## Acceptance criteria

- The five commands and the event exist, are typed, and are reachable from the
  generated client.
- `PROTOCOL_VERSION` is `2.25.0` and `packages/protocol/src/generated/` is
  regenerated by `cargo test -p gritty-core` and committed.
- `pnpm check` green, including `check:generated`.
- `docs/merge-queue.md` records 2.25.0 as spent rather than claimed.

## Tests required

- Rust: the wire types round-trip; an unknown key is refused by name; a
  wrong-typed value is refused by name and the expected type is in the message.
- The export test that regenerates the TypeScript types passes and its output
  is committed.
- TypeScript: the client exposes all five methods and the event type narrows.

## Commands to run

```
cargo test -p gritty-core     # also regenerates packages/protocol/src/generated
pnpm check
```

## Completion report

*(filled on completion)*
