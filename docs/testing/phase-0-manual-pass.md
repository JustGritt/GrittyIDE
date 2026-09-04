# Phase 0 manual pass (macOS)

The Phase 0 spike cannot be closed by tests. ADR-001's kill criteria are
*observations*: recurring editor/input defects, PTY behaviour that feels
unreliable, WebView divergence that costs maintenance. None of that can be
concluded from `pnpm check`, and the agent cannot see or click the WebView on
this machine.

This is every outstanding human check in one ordered session. It replaces the
"Not executed (human WKWebView pass)" lists spread across the T040, T050, T060
and T090 completion reports.

- **Time:** about 90 minutes, in six sessions you can run separately.
- **Fills:** the `–` rows in `docs/testing/platform-matrix.md`, the perceptual
  rows of `docs/plans/phase-0-spike.md` §C, and the §D go/no-go answers.
- **Record:** tick each step, note anything surprising, and keep the log file.
  The log is not optional: it is where the §C command timings come from.

Legend for the record: `✔` works · `✘` defect (say what happened) ·
`~` partial · `–` not run.

---

## Session 0 — Preparation

**Already done on this machine on 2026-09-04** — `fixtures/large-repo` (10 501
files, 103 directories, depth 25), `fixtures/weird-files/generated`
(`large-text.txt` 12 MiB, `denied.txt` mode 000) and
`target/debug/bundle/macos/Gritty.app` all exist. Re-run only if you clean them:

```sh
cd ~/Desktop/CurrentProjects/GrittyIDE
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

# Generated fixtures (git-ignored, safe to regenerate)
cargo run -p gritty-workspace --example generate_large_repo
cargo run -p gritty-workspace --example generate_weird_files

# The packaged app for Session 6 (reuses the debug build; a few minutes)
pnpm tauri build --debug --bundles app
```

Sessions 1–5 run the dev build, with the log captured:

```sh
rm -rf /tmp/gritty-manual
GRITTY_DATA_DIR=/tmp/gritty-manual GRITTY_LOG=debug pnpm dev 2>&1 \
  | tee /tmp/gritty-manual.log
```

Tracing writes to stderr, hence `2>&1`. Keep this terminal visible: the app
logs every command with its duration, so a slow interaction has a number
attached before you have to describe it.

Open the Web Inspector once and leave it open: right-click in the window →
*Inspect Element*. Watch the Console for CSP violations throughout — an
unexplained error there is a finding even if the feature looks fine.

**Between sessions,** `rm -rf /tmp/gritty-manual` for a clean start, except in
Session 5, which depends on the state Session 2 left behind.

---

## Session 1 — Launch, shell and explorer (~10 min)

| # | Do | Expect |
|---|---|---|
| 1.1 | Watch the window appear | Opens at 1280×800; no flash of an empty white window before content |
| 1.2 | Drag the window edge inward | Refuses to go below 800×600 |
| 1.3 | Check the terminal log | `startup milestone … "window shown"` and `… "first webview health"`; note both numbers |
| 1.4 | System Settings → Appearance → switch Light/Dark, watch the window | Editor and chrome follow without a reload |
| 1.5 | Zoom the display (or use a second monitor at a different scale) | Text stays crisp, no blurred bitmap scaling |
| 1.6 | Open a workspace → pick `fixtures/small-repo` | Native macOS picker, not an HTML dialog |
| 1.7 | Look at the explorer | `file with spaces.txt`, `link-to-src` (symlink) and any accented names listed correctly |
| 1.8 | Open the same workspace again via a differently-cased path (`FIXTURES/small-repo`) | Same workspace, path shown in its on-disk spelling — not a second one |
| 1.9 | Open `fixtures/large-repo` | Root listing appears immediately; no beachball; no long freeze while it enumerates |
| 1.10 | Expand the directory with more than 5000 entries | Pages progressively with a cursor, no hang |
| 1.11 | With `large-repo` open, `mv fixtures/large-repo /tmp/gone` in another terminal | Workspace closes with a clear message rather than an error dialog or a crash (`mv` it back after) |

Rows filled: Shell (both), Rendering (both), Files (picker, unicode/symlinks,
case identity, 10k+ repo, >5000 entries, root removed).

---

## Session 2 — Editor, T040 (~25 min)

Workspace: `fixtures/small-repo`.

**Basic editing**

| # | Do | Expect |
|---|---|---|
| 2.1 | Open a `.ts`, a `.rs`, a `.json` and a `.md` file | Syntax colouring, no worker warning in the pane |
| 2.2 | Type in the `.ts` file | Characters appear with no lag; a dirty dot shows on the tab |
| 2.3 | Cmd+S, then `cat` the file in a terminal | The change is on disk; the dirty dot clears |
| 2.4 | Cmd+Z / Cmd+Shift+Z | Undo and redo behave |
| 2.5 | Cmd+F, then Cmd+Option+F | Find and replace widgets open and work |
| 2.6 | Cmd+D on a repeated word; Option+click in three places | Multi-cursor: all carets type together |
| 2.7 | Select a block, Cmd+C, Cmd+V, Cmd+X | Clipboard round-trips inside the editor **and** to another app (paste into Notes) |
| 2.8 | Fold a block; put the caret on a `{` | Folding works; the matching brace highlights |
| 2.9 | Break the syntax of the `.ts` and the `.json` | Squiggles appear (that is the worker running); Console shows no CSP violation |

**Input edge cases — this is a kill-criterion area, be picky**

| # | Do | Expect |
|---|---|---|
| 2.10 | Type accented characters: press-and-hold `e` → `é`, and dead keys if you use a layout that has them | The composition popup appears over the caret, in the right place, and commits once |
| 2.11 | Ctrl+Cmd+Space, insert an emoji | Inserted once, at the caret, not duplicated |
| 2.12 | Select text in the editor and drag it a few lines down | Text moves; no ghost, no duplicate |
| 2.13 | Drag a file from Finder onto the window | A dashed drop target appears while the drag is over the window, and disappears when it leaves |
| 2.14 | Drop `fixtures/small-repo/src/index.ts` (a file in the open workspace) | Opens as a tab. Dropping three files opens three tabs |
| 2.15 | Drop a file from `~/Downloads` (outside the workspace) | A notice offering to open its *folder* — the workspace is not swapped and no tab appears until you accept |
| 2.16 | Drop the folder `fixtures/markdown` | Becomes the workspace. With a dirty tab open it asks first, offering "Discard changes and open" |
| 2.17 | Drop two folders at once, and a folder together with a file | One folder opens; the notice names what it skipped and why |
| 2.18 | Drag a tab to reorder it | Reorders. OS file drops are enabled (`dragDropEnabled: true`), so if the tab drag stopped working, that is the T040 interaction risk — say so, it decides whether these two can coexist |

**Files that misbehave** (`fixtures/weird-files/generated`, opened as a workspace)

| # | Do | Expect |
|---|---|---|
| 2.19 | Open `crlf.txt`, edit, save, then `file crlf.txt` and check bytes | CRLF preserved, not silently converted |
| 2.20 | Open `mixed-eol.txt` | Read-only normalized view with an explicit "Normalize to LF / CRLF" action |
| 2.21 | Open `utf8-bom.txt`, save without editing | BOM still there |
| 2.22 | Open a binary file | Placeholder, not garbage, not a hang |
| 2.23 | Open `one-line-100k.txt` | Opens; scrolling stays usable |
| 2.24 | Open `large-text.txt` (12 MiB) | Read-only degraded view with "Enable editing" |
| 2.25 | **§C typing responsiveness:** Enable editing on `large-text.txt`, start a Web Inspector *Timelines* recording, type continuously for 30 s, stop | Report the number of frames over 16 ms and the worst frame. Repeat briefly in `small-repo/src/*.ts` for comparison |

**Conflict and close**

| # | Do | Expect |
|---|---|---|
| 2.26 | With a dirty tab, run `sed -i '' 's/a/b/' <that file>` elsewhere, then click back into the window | CONFLICTED banner offering reload / keep / overwrite; nothing resolved silently |
| 2.27 | Close a dirty tab | Dialog: save / discard / cancel — all three do what they say |
| 2.28 | Close the workspace with a dirty tab open | Refused, with the dirty documents named; after discarding, orphans are shown |
| 2.29 | **§C open latency:** open a normal source file and judge it | Instant, or a visible delay? The log line `command completed command="documents.open"` gives the core-side number to compare against what you felt |

Rows filled: Input (all five), Editor (CRLF bytes, workers at runtime, typing
frame drops, dirty close), Files (open/edit/save, weird files, external
modification, close refused).

---

## Session 3 — Markdown, T050 (~15 min)

Workspace: `fixtures/markdown`.

| # | Do | Expect |
|---|---|---|
| 3.1 | Open `gfm.md`, cycle Source → Split → Preview | Headings, lists, task lists, tables, fences, quotes, links and images all render; the cursor survives each switch |
| 3.2 | Type in Split view | Preview follows within roughly 100 ms |
| 3.3 | Scroll the editor in Split view | The preview follows and reaches the bottom when the editor does |
| 3.4 | Open `long.md` (134 KB) | Renders without freezing the window; note how long it feels |
| 3.5 | Open a multi-megabyte `.md` (copy `long.md` onto itself 30× if needed) | Pause banner with a Refresh button, not a frozen window |
| 3.6 | Click `docs/target.ts:3:5` in `links.md` | Opens the file at Ln 3 Col 5 |
| 3.7 | Click `deep.md#section` and an in-page anchor | Both scroll to the heading |
| 3.8 | Click `docs/missing.md` | Inline notice; the link is marked as broken |
| 3.9 | Click a `mailto:` link and `../escape.md` | Inert, with a tooltip explaining why. **The window must not navigate** |
| 3.10 | Click `https://example.com` | Default browser opens **once**; the Gritty window stays exactly where it was |
| 3.11 | Middle-click that link, then right-click → "Open Link" | Neither navigates the WebView (Console shows the navigation guard if it fires) |
| 3.12 | Open `xss.md` | Escaped text; **no alert**, no CSP violation in the Console |
| 3.13 | Look at `README.md` and `AGENTS.md` in explorer, tab and toolbar | Instruction-file badge visible |
| 3.14 | Check the image placeholder and the `data:image/png` in the fixtures | Placeholder for the missing one; the data URI renders |

Rows filled: all seven Markdown rows.

---

## Session 4 — Terminal, T090 (~15 min)

Any workspace; open the Terminal dock at the bottom (collapsed by default).

| # | Do | Expect |
|---|---|---|
| 4.1 | "New terminal" | Your login shell, in the workspace root. `echo $PATH` and the prompt look like your own terminal |
| 4.2 | `stty size`, then resize the window, then `stty size` again | The second matches the new drawn size |
| 4.3 | `vim` a file, `:q`; then `less` a long file, `q`; then `top`, `q` | All three draw, redraw on resize, and exit cleanly |
| 4.4 | Run `ping example.com`, press Ctrl+C | Interrupted immediately |
| 4.5 | `yes` | UI stays responsive; a yellow `[gritty] N bytes of output dropped` marker appears. Ctrl+C to stop |
| 4.6 | Open three terminals, type something distinct in each, switch tabs | Scrollback survives every switch |
| 4.7 | In a tab run `sleep 300 &`, note the pid, close the tab, then `pgrep -f "sleep 300"` | No output — the job died with the session |
| 4.8 | Type `exit` in a tab | The tab stays, showing its exit status; it does not vanish |
| 4.9 | `pgrep -f zsh \| wc -l`, quit Gritty (Cmd+Q), `pgrep -f zsh \| wc -l` again | The same number — no shell survives the quit |

Rows filled: interactive shell, tabs/scrollback/exit status, and the human
halves of resize, process-tree kill, unexpected death and output storm.

---

## Session 5 — Session and recovery, T060 (~20 min)

Keep the same `GRITTY_DATA_DIR=/tmp/gritty-manual`; these steps build on each
other. Killing the app also ends `tauri dev`, so re-run the launch command from
Session 0 after every force-kill — that is the relaunch.

| # | Do | Expect |
|---|---|---|
| 5.1 | Open three tabs, put one Markdown tab in Preview, leave the caret deep in a third file, change a preference, quit and relaunch | Everything comes back: workspace, tabs, active tab, view modes, cursor position, preferences |
| 5.2 | **§C warm launch:** note `startup milestone` on that relaunch | Compare with the cold number from Session 1 |
| 5.3 | Type into a file without saving, then `kill -9 $(pgrep -f gritty-desktop)` from another terminal, and relaunch | Prompt offers to restore; Restore puts the text back, still dirty. **The file on disk is untouched** |
| 5.4 | Repeat 5.3 but edit the file externally before relaunching | CONFLICTED banner, not a silent overwrite |
| 5.5 | Repeat 5.3 but delete the file before relaunching | DELETED_EXTERNALLY, handled without a crash |
| 5.6 | With dirty tabs open, press Cmd+R | The renderer reloads and rehydrates: same tabs, same dirty state, nothing corrupted. If Cmd+R is not bound, say so — that is worth knowing |
| 5.7 | Quit, `echo garbage > /tmp/gritty-manual/gritty.sqlite`, relaunch | Clean start with a "database moved aside" notice, not a crash. A `gritty.sqlite.corrupt-<timestamp>` file should be sitting next to it afterwards |
| 5.8 | Move the window to a second monitor (or a corner), quit, disconnect that monitor if you can, relaunch | The window comes back on screen, never off-screen |
| 5.9 | Sleep the Mac with Gritty open, wake it | Window still responsive; terminals still alive; no runaway CPU in Activity Monitor |

Rows filled: all six Recovery rows, both System rows.

---

## Session 6 — Packaged app, Run B (~10 min)

The production CSP only applies to the packaged app; `pnpm dev` runs the
`devCsp`. This session is what makes the CSP rows real.

```sh
rm -rf /tmp/gritty-packaged
GRITTY_DATA_DIR=/tmp/gritty-packaged GRITTY_LOG=debug \
  ./target/debug/bundle/macos/Gritty.app/Contents/MacOS/gritty-desktop 2>&1 \
  | tee /tmp/gritty-packaged.log
```

| # | Do | Expect |
|---|---|---|
| 6.1 | Launch as above; keep the Console open | No CSP violation of any kind at startup |
| 6.2 | Open `small-repo`, open a `.ts` and a `.json`, break the syntax of each | Squiggles appear — the Monaco workers load under `worker-src 'self' blob:` |
| 6.3 | Open `fixtures/markdown/xss.md` | Escaped text, no alert, no CSP violation |
| 6.4 | Click an external link in `links.md` | Browser opens once; the packaged window does not navigate |
| 6.5 | Open a terminal in the dock, run something | Works the same as in dev |
| 6.6 | Quit; run *without* `GRITTY_DATA_DIR`, open a workspace, quit, relaunch by double-clicking the app in Finder | Restores from the real app-data directory (`~/Library/Application Support/io.github.justgritt.gritty`) |
| 6.7 | Note cold launch of the packaged app from the log | Compare with dev — this is the honest launch number |

Rows filled: Build (launchable bundle), the packaged-app halves of the worker,
XSS, external-link and data-directory rows.

---

## Then: the three questions that actually decide ADR-001

Row ticks are evidence, but the ADR turns on judgment. Answer these in your own
words — a paragraph each is enough, and "I am not sure yet" is a real answer.

1. **Editor.** After an hour in it, does text editing feel dependable? Not
   *feature-complete* — dependable. Did anything about typing, selection,
   clipboard or accented input misbehave more than once? A defect you saw twice
   matters far more than one you saw once.

2. **Terminal.** Did the PTY feel like a terminal, or like something imitating
   one? Any moment where you were unsure whether a process had really died?

3. **Maintenance.** Of everything that went wrong today, how much was
   *Gritty's* bug and how much was *the WebView's* behaviour that Gritty has to
   work around? The second kind is what the kill criterion is about.

---

## Recording

1. Paste back the numbered steps that were anything other than `✔`, with what
   you saw. Steps you do not mention are taken as `✔`.
2. Keep `/tmp/gritty-manual.log` and `/tmp/gritty-packaged.log` — hand over the
   paths and the §C command timings come out of them automatically.
3. Note the machine and date once: macOS version, chip, RAM.

The agent transcribes all of it into `docs/testing/platform-matrix.md` and
`docs/plans/phase-0-spike.md`, with your observations attributed to the pass
rather than to a test.
