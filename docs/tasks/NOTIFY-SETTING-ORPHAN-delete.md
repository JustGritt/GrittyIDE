# Task NOTIFY-SETTING-ORPHAN — Delete the notification switch nothing mounts

- **Phase:** maintenance (a follow-up of T120-A and AGENT-TURN-END)
- **Status:** review
- **Depends on:** T120-U (the settings page), T120-M (mounting it). Both are
  on `main`.

## Objective

`apps/desktop/src/features/agents/agent-notification-setting.tsx` has had no
importer since T120-A (make the editor and terminal obey their settings)
deleted the agents popover that mounted it. Delete it, or mount it.

## Decision: delete

The settings page already owns this switch, so mounting a second one would be
a duplicate:

- `features/settings/settings-fields.tsx` `NotificationField` renders
  `agents.notifications`, reached through `settings-model.ts` (`CustomControl`
  `'notifications'`).
- `features/settings/use-settings.ts` `setNotifications` calls
  `core.agents.setNotifications` first and then persists the answer to the
  settings file. The orphan only did the first half, so it could never have
  persisted the choice.
- `SettingsPage` is mounted from `features/editor/editor-pane.tsx`.

Its two CSS rules, `.agents__notifications` and `.agents__notifications-row`,
have no other user. `.agents__hint` and `.agents__note` stay, because
`agents-panel.tsx` uses both.

## Allowed scope

- `apps/desktop/src/features/agents/agent-notification-setting.tsx` (delete)
- `apps/desktop/src/app.css`: the two `.agents__notifications*` rules only. It
  is a shared file carrying other sessions' hunks, so stage hunks, not the file.
- this task file

## Explicit non-goals

- No change to the settings page's notification field or to the core commands.
- No edit to completed task files that name the component. They are history.

## Acceptance criteria

- No file in `apps/` or `packages/` names `AgentNotificationSetting`,
  `agent-notification-setting` or `agents__notifications`.
- `pnpm check` green.

## Completion report

### Summary

Deleted `agent-notification-setting.tsx` and its two orphaned CSS rules. The
settings page's `NotificationField` is the only notification switch, and it is
mounted.

### Files changed

- `apps/desktop/src/features/agents/agent-notification-setting.tsx` (deleted)
- `apps/desktop/src/app.css`: `.agents__notifications` and
  `.agents__notifications-row` removed. This hunk only; the file carried another
  session's uncommitted explorer hunk, which was left alone.
- this task file

### Tests actually executed, and results

- `grep` over `apps packages crates docs/architecture docs/decisions scripts`
  (excluding `node_modules`, `target`, `dist`) for the component, the file name
  and the class: no match.
- Before deleting, checked that the replacement is live: `SettingsPage` is
  imported and rendered by `features/editor/editor-pane.tsx`, `NotificationField`
  renders `agents.notifications`, and `use-settings.ts` calls
  `core.agents.setNotifications`.
- `pnpm check`: run once, on the tree with this ticket and LAST-PROMPT-SHELL
  (a `!` shell command is not a session's last prompt). Exit 0: Biome (345 files), `tsc`, vitest **1346 passed**,
  `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` **1253
  passed, 0 failed**, generated types fresh.

### Known limitations

None. Nothing imported the file.

### Follow-up tasks

None.
