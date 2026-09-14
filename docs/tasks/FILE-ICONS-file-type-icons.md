# Task FILE-ICONS — file-type icons in the explorer tree and Changes list

- **Phase:** 2
- **Status:** doing
- **Depends on:** T042 (the editable tree), T100-U (the Changes tab),
  SIDEBAR-ICONS (sidebar section tabs as icons — the icon frame it introduced)

## Why

The tree marked every file with the same faint `·` and every folder with `▪`
beside a `▸`/`▾` text toggle, so a listing was a column of names with nothing
to scan by. Asked for by Alexis, with a screenshot of the editor they use today
(Cursor with the Seti file-icon theme) as the reference: a coloured type glyph
per file, and a chevron for a folder in the same column.

## Allowed scope

- `apps/desktop/src/components/icons.tsx` (new) — the shared 16px SVG frame and
  the chevron; `features/git/sidebar-icons.tsx` now draws in it
- `apps/desktop/src/components/file-icon.tsx` and its test (new)
- `apps/desktop/src/features/explorer/explorer-tree.tsx` and its DOM test
- `apps/desktop/src/features/git/changes-view.tsx`
- `apps/desktop/src/app.css` — the tree toggle/glyph rules, the new
  `.file-icon*` rules, and one alignment rule on the Changes row

## Acceptance criteria

- A file row draws a type icon decided from its name alone — no filesystem read.
- A folder row draws a chevron that turns down when open, in the same 16px
  column, so folder and file names line up.
- The Changes list draws the same icon beside each changed path.
- Colours come from tokens that exist in both themes; no new palette.
- No new dependency.

## Completion report

**Summary.** `fileIconKind(name)` maps a name to one of 22 kinds: extensions,
dotfiles by name (`.gitignore`, `.editorconfig`, `.nvmrc`), and README/LICENSE
ahead of their extension. `<FileIcon kind>` draws each kind as inline SVG
(letters for TS/JS/RS/PY, glyphs for the rest). The tree now renders either a
chevron toggle or an icon, not the old invisible disabled toggle plus a text
glyph. Symlinks and special files keep their own icons, and an expandable
symlink draws as a folder, as before.

**Decisions.**

- *Drawn, not vendored.* Seti's font and mapping are MIT, but vendoring them
  means a font asset and a large JSON mapping for a look that ~20 glyphs
  cover. The kinds were chosen from `git ls-files` over this repository
  (`.ts` 488, `.rs` 163, `.md` 146, `.tsx` 56, `.json` 22, `.toml` 18, …),
  not from a guessed list.
- *ANSI ramp for colour.* `tokens.css` has no icon tokens, and its ANSI
  colours are the only named hues it defines for both themes. Dedicated
  `--icon-*` tokens would need an ADR-016 amendment and edits to all three
  theme blocks, in a file another lane has uncommitted changes in.

**Tests executed.**

- `npx vitest run src/components src/features/explorer src/features/git
  src/features/session` — 20 files, 241 tests passed, including the new
  `file-icon.test.ts` (mapping, case, README/LICENSE precedence, dotfiles,
  fallback).
- `explorer-tree.dom.test.tsx`: new case asserting a folder row has a chevron
  and no file icon, a file row has no toggle, and `README.md` and `notes.txt`
  get the `readme` and `text` icons — 11 passed.
- `npx tsc --noEmit -p apps/desktop` — clean. `npx biome check` on the changed
  paths — clean.
- The real components were server-rendered through Vite into a scratch page
  that loads the real `app.css`, as a tree of this repository's root entries
  plus one file per kind, in dark and `data-theme="light"`. That check made the
  letter glyphs larger (7.5px → 9px); everything else read as drawn.

**Not done.** Not exercised in the running Tauri window. The Changes row's
icon alignment is not checked by eye. Full `pnpm check` not run: the working
tree carries other lanes' uncommitted work.

**Follow-ups** (each seen in the reference screenshot, none done here):

- Icons on editor tabs. `features/editor/tab-strip.tsx` has another lane's
  uncommitted edits, so it was left alone; `<FileIcon>` is ready for it.
- Dim git-ignored entries (`node_modules`, `target`) instead of only badging
  them `ignored`.
- Colour tree names and folders by git status, with the dot the reference
  draws on folders that contain changes.
- The tree's font: the reference uses the UI face; the tree is monospace.
