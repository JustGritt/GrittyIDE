# Task T047 — Naming terminal sessions

- **Phase:** 1A
- **Status:** done
- **Depends on:** T090, T095

## Objective

Let the user label a terminal session so the tab says what is in it. Sessions
increasingly hold something worth telling apart — a coding agent, a dev server,
a build — and a strip of tabs all reading `zsh` identifies none of them.

## Required reading

- `docs/architecture/terminal.md`, ADR-015
- `docs/architecture/security.md` (a name is user input drawn into the UI)

## Allowed scope

- `crates/gritty-core/src/terminal/**` (`terminal.rename`, `Terminal.name`)
- `packages/protocol` (command, type, client method, version bump)
- `apps/desktop/src-tauri` (one adapter)
- `apps/desktop/src/features/terminal/**`, `app.css`

## Explicit non-goals

- Automatic titles from what the session is running; that is shell integration
  (OSC 133, process title) and a task of its own.
- Persisting names across restarts — sessions do not survive app exit.
- Renaming from a context menu; there is no menu infrastructure.

## Required behavior

1. `terminal.rename` sets or clears a session's `name` and returns the updated
   snapshot. No event: nothing else subscribes to a label.
2. The name is trimmed. Blank clears it, so the tab falls back to the shell's
   own name rather than showing nothing.
3. Control characters are refused; the length is bounded in characters, not
   bytes.
4. An ended session can still be renamed.
5. The name survives a renderer reload, because `terminal.list` returns it.
6. A tab is renamed by double-clicking it, or by pressing F2 while it is
   focused. Enter commits, Escape cancels, blur commits.

## Failure behavior

- An unknown session is `NotFound`; an unusable name is `InvalidInput`.
- The field refuses a name the core would refuse, using the same rules, so the
  common mistakes never make a round trip.

## Acceptance criteria

- Double-click a tab, type `claude`, press Enter: the tab says `claude` and the
  tooltip still reports the shell, directory and pid.
- F2 does the same from the keyboard; Escape leaves the old name.
- Clearing the field puts the shell's name back.
- The name is still there after reloading the renderer.

## Tests required

- Rust: set, trim, clear, reject control characters and over-length, count
  characters not bytes, rename an ended session, unknown session.
- Tauri: rename through the real command wiring, and the name coming back from
  `terminal_list`.
- Vitest: the label fallback and the shared normalisation rules.
- Manual: the double-click, F2, Enter, Escape and blur paths.

## Commands to run

`pnpm check`, `pnpm dev`.

## Completion report

### Summary

`terminal.rename` labels a session; the tab shows the label instead of the
shell name, and is renamed by double-click or F2. Protocol 2.0.0 -> 2.1.0
(additive: one command, one optional field).

The decision worth recording is **where the name lives**. It is core state, not
renderer state, because `terminal.list` is what rehydrates the panel after a
renderer reload — a name kept in the renderer would vanish while the session it
described kept running. `rename` returns the updated snapshot like `resize` and
`kill`, and emits no event, because nothing else subscribes to a label.

### Files changed

- `crates/gritty-core/src/terminal/wire.rs`: `Terminal.name`,
  `RenameTerminalRequest`. `mod.rs`: `rename`, `normalize_name`,
  `MAX_TERMINAL_NAME_CHARS`, four tests. `protocol/export.rs`,
  `PROTOCOL_VERSION` 2.0.0 -> 2.1.0.
- `apps/desktop/src-tauri`: `terminal_rename` adapter, registration, one test.
- `packages/protocol`: `commands.ts`, `client.ts`, `types.ts`, generated
  `RenameTerminalRequest.ts`, `terminal-client.test.ts`.
- `apps/desktop/src/features/terminal/terminal-naming.ts` (new): `shellName`
  (moved from `terminal-profiles.ts`), `terminalLabel`,
  `normalizeTerminalName`; ten tests.
- `use-terminals.ts`: `rename`. `terminal-panel.tsx`: the inline field.
- `app.css`: the field, sized to the tab so the strip does not jump.

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test --workspace` | 284 pass, 0 fail (5 new) |
| `pnpm exec vitest run` | 275 pass across 44 files (6 new) |
| `pnpm check` | green once the generated files are committed |

Not executed: `pnpm dev`. Double-click, F2, Enter, Escape and blur are all
pointer and keyboard paths the agent cannot drive.

### Architectural decisions

- **The name is core state.** See above; the reload case decides it.
- **No event.** `resize` and `kill` already return a snapshot without emitting
  one, and adding an event for a label would be an event nobody subscribes to.
- **Blank clears rather than stores an empty label.** A tab with no text cannot
  be clicked, and emptying a field is how a user says "remove this".
- **Bounded in characters, not bytes.** The bound exists so a tab stays a tab,
  and a tab's width is what is drawn — an emoji is one character however many
  UTF-16 units or UTF-8 bytes it costs. Both sides count the same way.
- **An ended session can be renamed.** Refusing would be a rule with no purpose
  behind it, and naming a session after it failed is the useful case.
- **Validation is duplicated deliberately.** `normalizeTerminalName` mirrors
  the core's rules so the field answers immediately; the core still checks,
  because the renderer is not trusted to.
- **Focus is set imperatively, not with `autoFocus`.** A linter cannot tell
  `autoFocus` on an intentional rename apart from stealing focus on load. An
  effect keyed on which tab is being renamed says what is meant, and `select()`
  lets the old name be typed straight over.
- **Control characters are checked by code point, not by a regex** that would
  have to contain the characters it rejects — which the linter rightly flags.

### Known limitations

- **Manual checks are owed** for every interaction path.
- **Names do not survive a restart**, because sessions do not either.
- **No automatic titles.** A tab cannot notice that `claude` is running in it;
  that needs shell integration.
- **One rename at a time**, and starting a second cancels nothing — the first
  commits on blur, which is the intended behaviour but is worth seeing.
- **A refused name is dropped silently** on commit rather than explained. The
  tab strip is not a place for a validation message, and the field enforces
  `maxLength` already, so only a pasted control character reaches it.

### Follow-up tasks

- Shell integration (OSC 133) so a session can name itself after what it runs,
  which is what makes this useful without typing. Also a T300 prerequisite.
- A context menu on a tab, once there is menu infrastructure, so rename and
  close are discoverable without knowing F2.
- Persist names if sessions ever outlive the application.

### Remaining risks

- The inline field is 10rem wide regardless of the name; a long name scrolls
  inside it rather than growing the tab. That is deliberate but unseen.
