# Task T120-A — Make the editor and terminal obey their settings, then delete the popovers

- **Phase:** 2
- **Status:** doing
- **Depends on:** T120-M (mount the settings tab)

Written after the fact: the ticket was specified in its board note and in the
"two things deliberately not done" of T120-M's completion report, and had no
task file until this one.

## Objective

After T120-U and T120-M the settings page edits eight `editor.*` and
`terminal.*` keys that nothing applies, so the three popovers were still the
only working controls. Make Monaco and the terminal dock read `settings.json`
(ADR-031), seed both session blobs into it once (ADR-031 §8), and delete the
popovers that edited copies of those settings.

## Allowed scope

`apps/desktop/src/App.tsx`, `app.css`, `features/terminal/**`,
`features/editor/editor-settings*`, `features/editor/editor-pane.tsx`,
`features/settings/**`, `features/session/use-session.ts`,
`features/session/shell-restore.dom.test.tsx`,
`features/agents/{agent-integration-settings,agents-panel}.tsx` (released by
the AGENT-TURN-END session for this change), this file.

## Explicit non-goals

- No protocol, registry or core change: the keys, their bounds and the
  `settings.*` commands exist (T120-C, T120-R).
- The session blobs are not deleted or migrated. They are restored and written
  back unchanged, as the seed's source; removing them is ADR-031 §8's later
  mirror removal.

## Required behavior

1. Monaco is configured from `editor.fontSize`, `editor.tabSize`,
   `editor.wordWrap` and `editor.minimap`; the editor bar's popover is replaced
   by an "Editor settings" button that opens the page on its Editor section.
2. The dock reads `terminal.shell`, `terminal.customShells`,
   `terminal.fontFamily` and `terminal.fontSize`; its `⋯` opens the page on its
   Terminal section. The dock's shell picker writes `terminal.shell`.
3. The first launch that has the file seeds both blobs in one `settings.set`.
4. `terminal-settings.tsx` and `agent-integration-settings.tsx` are deleted with
   their mount points.

## Completion report

### Summary

The editor half (1, and the preferences half of 3) was built in an earlier
session on 2026-09-14 and left uncommitted; this session finished the terminal
half.

- **The dock reads settings.** `terminal-settings.ts` (replacing the `.tsx`
  popover) projects a snapshot onto the `TerminalLayout` shape the dock already
  consumed, through `parseLayout` so the bounds are the ones T095 enforced —
  the same arrangement `editor-settings.ts` has with `parseEditorPreferences`.
  `App.tsx` passes it to `useTerminals` and `TerminalPanel`; `layout.terminal`
  is no longer read by anything.
- **The picker is a shortcut to `terminal.shell`**, not a copy. A refused write
  (read-only storage) opens the settings page on Terminal, where the refusal is
  shown, rather than leaving a picker that silently did not change.
- **The `⋯` stays in the bar** (artboard 8a) but is a button into the page's
  Terminal section.
- **The agent-integration popover is deleted** with its mount in the island's
  `agents-panel.tsx`. Its replacement was checked before deleting: the page's
  `settings/agent-integration-section.tsx` makes the same status / install /
  uninstall calls, and notifications are the page's `agents.notifications`
  field. With all three popovers gone, the whole `.prefs` CSS block had no user
  and is removed. Both agents files were in the AGENT-TURN-END lease; that
  session released them for this change.
- **Steering the page to a section moved into the tab reducer.** It was local
  state in `editor-pane.tsx`, which the dock cannot reach. `open` now takes an
  optional `section`; `section` is cleared when the page leaves the pane, and
  `sectionRequest` counts asks so the page (keyed on it) remounts on a repeat —
  the dock stays on screen beside the page, so its button can be pressed while
  the page already shows another section. An open with no section (⌘,) does not
  remount, so a search in progress survives.
- **The seed carries both blobs.** `useSession`'s `onPreferencesBlob` became
  `onRestoredBlobs`, called once with the raw `preferences` and `layout` straight
  from `session.restore`'s answer; `App` hands it to `useSettings` as the seed
  source. Seeding runs once per session, so handing it only one blob would have
  lost the other's keys for good.

### Files changed (this session)

- `apps/desktop/src/App.tsx` — terminal settings from the snapshot, the picker
  and `⋯` wiring, `showSettings(preview, section?)`, the seed from both blobs.
- `apps/desktop/src/app.css` — the terminal popover block and every `.prefs`
  rule removed, `.terminal-panel__settings` added.
- `features/agents/agent-integration-settings.tsx` — deleted.
- `features/agents/agents-panel.tsx` — its mount and import removed.
- `features/editor/editor-pane.tsx` — the section comes from the tab state; the
  strip's click is wrapped so its event is not taken for a section.
- `features/session/use-session.ts` — `onRestoredBlobs`.
- `features/settings/settings-tab.ts` + test — `section`, `sectionRequest`.
- `features/terminal/terminal-panel.tsx` — `onShellChange`, `onOpenSettings`,
  the button; popover import gone.
- `features/terminal/terminal-settings.tsx` — deleted.
- `features/terminal/terminal-settings.ts` + test — new.
- `features/terminal/terminal-panel.dom.test.tsx` — new.
- `features/session/shell-restore.dom.test.tsx` — the seed case.

### Tests actually executed

- `vitest run` over `features/{terminal,settings,session,editor}`: 26 files,
  256 tests, pass.
- `shell-restore.dom.test.tsx` "seeds settings.json from the preferences and the
  layout together" mounts the real `App` against a stubbed core. **Mutation
  checked:** with `layout: null` put back in `use-session.ts` it fails (1 failed,
  2 passed); restored, it passes.
- `pnpm check:ts` after the agents deletion: exit 0 (125 files, 1311 tests).
- `pnpm check`, full, before the agents deletion: exit 1 — decomposed rather than claimed: `check:ts` green
  (Biome, `tsc` across the workspace, vitest 125 files / 1311 tests).
  `check:rust` failed at `cargo fmt --check` on `crates/agents/src/transcript.rs`
  and `crates/gritty-core/src/agent_session/attention.rs`, both uncommitted work
  of the live AGENT-TURN-END session (the roster says Done while the model is
  still generating); clippy and `cargo test` therefore did not run. This ticket
  changes no Rust. `pnpm check:generated` run on its own: exit 0.
- Not run: the app. No screen access in this session, so the dock's `⋯`, the
  picker writing `terminal.shell`, and a live font change reaching an open
  terminal have not been seen on screen.

### Architectural decisions

- The dock keeps its `TerminalLayout` prop type rather than a new settings type:
  it is the shape `parseLayout` already validates, and the dock's code did not
  need to change to consume it.
- The section request lives in the singleton tab reducer, not in `App` state, so
  it stays pure and tested and is cleared by the same transitions that hand the
  pane back.

### Known limitations

- The dock's first auto-started shell can race the first settings snapshot and
  start the default shell. The same race existed against the session restore's
  layout before this change; it needs a dock opened within milliseconds of
  launch.
- `parseLayout` accepts only `/`-rooted custom shells, as the layout always did;
  the core's `pathList` validation also accepts Windows-absolute paths.
  Unreachable on macOS; relevant when Windows is claimed.

### Follow-up tasks

- `features/agents/agent-notification-setting.tsx` has no importer now (the
  deleted popover was its only one). It sits in the AGENT-TURN-END lease, so it
  was left; delete it once that lease allows.
- Delete the `preferences` blob and `layout.terminal` once ADR-031 §8's mirror
  removal is scheduled.
