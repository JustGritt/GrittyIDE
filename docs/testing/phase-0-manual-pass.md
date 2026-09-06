# Phase 0 manual pass (macOS)

The Phase 0 spike cannot be closed by tests. ADR-001's kill criteria are
*observations*: recurring editor/input defects, PTY behaviour that feels
unreliable, WebView divergence that costs maintenance. None of that can be
concluded from `pnpm check`, and the agent cannot see or click the WebView on
this machine.

This is every outstanding human check in one ordered session. It replaces the
"Not executed (human WKWebView pass)" lists spread across the T040, T041, T045,
T048, T049, T050, T051, T060, T090, T096, T098, T210, T220, T221, T225, T231,
T232 and T325 completion reports.

That list used to name nine tasks and the sentence above it claimed to be
complete anyway; `docs/testing/review-findings.md` finding 6 is what caught it.
Sessions 1B (T051), 8 and 9 are what closed the gap, and every step in them
traces to a claim listed there.

- **Time:** about 170 minutes, in nine sessions you can run separately — the
  sum of the per-session estimates below, which the old "105" had stopped being.
- **The split that matters:** **Sessions 1–7 decide ADR-001** (~130 min).
  Sessions 8 and 9 close the review queue — T210, T220, T221, T231, T232 and
  T325 — and neither half blocks the other. If you have one evening, run 1–7.
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

# 0.1 — the agent hook, for Sessions 6 and 9 (T221, T225)
#   In dev, Gritty finds the hook beside its own binary in target/debug. A
#   clean checkout has never built it, and then the whole hook surface reads
#   "unavailable" for a reason that is not a defect.
cargo build -p gritty-agent-hook

# The packaged app for Session 6 (reuses the debug build; a few minutes)
# `pnpm sidecar` runs inside this and puts the hook in the bundle (T225).
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

## Session 1B — Startup screen, T048/T049/T051 (~15 min)

**Nothing in this session has ever been performed.** T048 and T049 were both
built without a screen: verified by `pnpm check`, by rendering the frontend in
Chrome against a vite dev server, and (T049) by one core-connected run whose
log proved the launcher mounted. No pointer has touched either. Be picky here
for the same reason Session 2's input steps are picky — this is the first
contact.

Two steps need a **fresh data directory**, which is also the only way to see a
true first run:

```
GRITTY_DATA_DIR=$(mktemp -d) pnpm dev
```

**First run and the ways in**

| # | Do | Expect |
|---|---|---|
| 1B.1 | Launch with a fresh `GRITTY_DATA_DIR` | Centred launcher: wordmark, one-line description, **two** tiles (Open folder…, Open path…), no recents list, the dashed drop hint. Top bar says "No workspace open"; status bar ends with `no workspace` |
| 1B.2 | Click **Open folder…** | The native macOS picker, same as step 1.6. Pick `fixtures/small-repo`: the launcher is replaced by the sidebar and editor |
| 1B.3 | Explorer `⋯` → **Close workspace** | Back to the launcher, now with one recent row and a **third** tile, "Reopen small-repo" |
| 1B.4 | Read the recent row | `small-repo`, a relative time ("just now"), and on the right its **parent** directory reading left to right and starting with `~` — not `/~Desktop/…`, which was a real defect and may recur |
| 1B.5 | Click the row | Opens that workspace. Its tabs do **not** come back — that is known and intended for now, not a bug to report |
| 1B.6 | Close again, click the **Reopen** tile | Same workspace, same result |
| 1B.7 | Click **Open path…** | A field appears **already focused**. Type an absolute path → opens. Try a relative one (`../secrets`) → the core's `InvalidInput` envelope, no crash. Escape closes the field |

**The resume rail, T051** — these continue from 1B.3's state and have never
been performed either. 1B.5 above says tabs do not come back when you click a
recents row; that is still true and still intended. The rail is the other
control, and it is the one that brings them back.

| # | Do | Expect |
|---|---|---|
| 1B.3a | At 1B.3's launcher — you closed a workspace that had no tabs open — look for the rail | There is none. A workspace with nothing open offers nothing to resume, and a rail that appeared here would teach you to ignore it |
| 1B.3b | Reopen `small-repo`, open two files, click somewhere in the middle of the second, type a character into the first **without saving**, then Explorer `⋯` → **Close workspace** | The launcher carries a resume rail naming `small-repo`, its tab count, and the unsaved buffer **by name** — not just a count |
| 1B.3c | Press **Restore** | The same two tabs, in the same order, with the same one active and the cursor where you left it. The edited tab is **still dirty**, and the file on disk is unchanged — restoring never writes |
| 1B.3d | With the workspace open, look for the rail; then close the workspace again and press **Open without them** | The rail is gone while a workspace is open, and a fresh one appears on closing. "Open without them" opens the folder with no tabs |
| 1B.3e | Close once more, press **Dismiss**, then reopen from the recents row and close again | Dismiss hides the rail and leaves the launcher alone. Reaching the launcher again offers the same tabs — **Dismiss hides, it never discards** |

**Keyboard**

| # | Do | Expect |
|---|---|---|
| 1B.8 | On the launcher press **⌘O**, then **⌘⇧O** | Picker; then the path field opens and takes focus |
| 1B.9 | With a workspace open press **⌘O** | Picker again — it is global. **⌘⇧O** does nothing, which is deliberate: only the launcher has a field |
| 1B.10 | Open the terminal dock, click into the shell, press **Ctrl+O**, then **⌘O** | Ctrl+O reaches the shell untouched (Gritty claims ⌘ on macOS, not Ctrl); ⌘O still opens the picker. Then type in Monaco and repeat — the editor must not lose a keystroke to either |

**Drop, and the empty-state surfaces**

| # | Do | Expect |
|---|---|---|
| 1B.11 | Drag a folder from Finder over the launcher, without releasing | The dashed drop target appears. **If it does not, stop and say so** — that separates "the OS event never arrived" from "the app did nothing with it", and is the first thing to know |
| 1B.12 | Release it | Becomes the workspace. Same rules as steps 2.13–2.17, which are worth running here too now that there is a second screen to drop onto |
| 1B.13 | On the launcher, open the terminal dock | A shell starts in your **home directory** (T095) and says so. Open a workspace afterwards: the scrollback survives |
| 1B.14 | Hover a recent row, then Tab to it | **Forget** appears on hover *and* on keyboard focus — it must not be mouse-only |
| 1B.15 | Click **Forget** on a row | The row goes immediately. Re-open that folder later and it comes back — Forget drops the history entry, never the folder |

**The missing root, end to end**

| # | Do | Expect |
|---|---|---|
| 1B.16 | `mkdir /tmp/gritty-doomed`, open it as the workspace, quit the app, `rm -rf /tmp/gritty-doomed`, relaunch | A session notice, "Last workspace not reopened"; the launcher lists `gritty-doomed` **struck through** with `root missing` in red, and it is not clickable |
| 1B.16a | Repeat 1B.16, but `echo hi > /tmp/gritty-doomed/a.txt` and open that file before quitting (T051) | The rail names `gritty-doomed` and offers **no Restore button**, saying why. An offer to reopen tabs under a root that is gone is worse than no offer |
| 1B.17 | Click its **Forget**, quit, relaunch | The row is gone **and the notice is gone** — this is the whole point of Forget clearing the last-workspace pointer. If the notice comes back, that is a defect |

**Long lists, themes and geometry**

| # | Do | Expect |
|---|---|---|
| 1B.18 | Once six or more workspaces have been opened, look at the list | Five rows and a **View all (n)** link; clicking it shows the rest and becomes **Show fewer** |
| 1B.19 | Switch macOS to Light with the launcher showing | The whole launcher follows: no dark-on-dark, no light-on-light. **The light ramp was derived from the dark one and never designed — this is the first time a person has seen it.** Say what looks wrong even if it is only "flat" or "washed out" |
| 1B.20 | Shrink the window to its 800×600 minimum with the list expanded | The panel **scrolls**; the wordmark at the top stays reachable and is never clipped |
| 1B.21 | With a workspace open, quit and relaunch | The workspace comes back by itself and **the launcher never flashes** on the way. A blink of the empty state here is a defect |

Rows filled: Shell (empty state), Files (picker again, drop onto the launcher),
Input (the two chords, and that neither steals from Monaco or the shell), and
Session (the resume rail's six claims, T051).

---

## Session 2 — Editor, T040 and T041 (~30 min)

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

**Preview tabs and the tab menu, T041** — the feature that landed with no ticket
and no steps until now (`review-findings.md` finding 4). None of it has ever
been performed; the reducer under it is unit-tested and the *gestures* are not.

| # | Do | Expect |
|---|---|---|
| 2.18a | Single-click three different files in the explorer, one after another | **One** tab, showing the third. Its name is in italics, and its tooltip says "preview, double-click to keep" |
| 2.18b | Single-click a file, type one character, then single-click another | **Two** tabs. The edited one is still open and still dirty — this is the rule the whole feature stands on |
| 2.18c | Double-click the previewed tab, then single-click another file in the explorer | The double-clicked tab stays; the new file opens beside it |
| 2.18d | Double-click a file in the explorer that is already the preview | It is kept, in one gesture. No second tab, no flicker |
| 2.18e | Single-click a file, then drag its tab one place left | It is kept: the next single click no longer replaces it |
| 2.18f | Open four files, edit the second and the third, right-click the first tab → "Close 3 tabs to the right" | Asks about the second, then the third, in that order. Cancel at the second: nothing behind it closes |
| 2.18g | Right-click the last tab | "Close tabs to the right" is present and disabled |
| 2.18h | With a preview open, quit and relaunch | It comes back as an ordinary tab — italics gone. Preview is not persisted |

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

Any workspace; open the Terminal dock at the bottom (collapsed by default —
click the `TERMINAL` label). Session 7 covers the bar itself.

| # | Do | Expect |
|---|---|---|
| 4.1 | "New terminal" | Your login shell, in the workspace root. `echo $PATH` and the prompt look like your own terminal |
| 4.2 | `stty size`, then resize the window, then `stty size` again | The second matches the new drawn size |
| 4.3 | `vim` a file, `:q`; then `less` a long file, `q`; then `top`, `q` | All three draw, redraw on resize, and exit cleanly |
| 4.4 | Run `ping example.com`, press Ctrl+C | Interrupted immediately |
| 4.5 | `yes` | UI stays responsive; a yellow `[gritty] N bytes of output dropped` marker appears. Ctrl+C to stop |
| 4.6 | Open three terminals, type something distinct in each, switch pills | Scrollback survives every switch |
| 4.7 | In a session run `sleep 300 &`, note the pid, press its ✕, then `pgrep -f "sleep 300"` | No output — the job died with the session |
| 4.8 | Type `exit` in a session | The pill stays, showing its exit status; it does not vanish |
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
| 6.8 | ⌥A, open the agents section, look at its header (T225) | The hook reads **available**, and the path it names is inside `Gritty.app/Contents/MacOS/`. Before T225 a packaged build always said unavailable; this step is the only positive check that the sidecar shipped |

Rows filled: Build (launchable bundle), the packaged-app halves of the worker,
XSS, external-link and data-directory rows, and the bundled sidecar (T225).

---

## Session 7 — Terminal dock, T096 / T098 (~15 min)

The chrome, not the PTY. Session 4 is the shell; this is the row above it. Open
a workspace, and leave a second window's worth of shells running.

| # | Do | Expect |
|---|---|---|
| 7.1 | Look at the dock collapsed, then expanded | One 32px row either way. Collapsed, the pills are still readable |
| 7.2 | With the dock collapsed, click a pill | The dock opens onto that session |
| 7.3 | Open a shell at the root, `cd crates/terminal` in another, and one with no folder open | The pills read `zsh GrittyIDE`, `zsh crates/terminal` and `zsh ~`. The `cd`-ed one keeps the directory it *started* in — the pill is the rooting, not the cwd |
| 7.4 | Click into a terminal so it has focus, press ⌥2 | Switches to the second session. Nothing is printed and nothing reaches the shell |
| 7.5 | Press ⌥7 with three sessions open | Nothing happens, and nothing is typed |
| 7.6 | Start a rename (F2), press ⌥1 while typing | The character goes into the field; no session switch |
| 7.7a | ⌥A, hover a roster row, press **Ask**, type a question, press **Add an option** twice and fill both, press **Ask** | The form folds away. The row shows "1 open question" and, if it was In progress, goes to Blocked — the core blocks running work on a question |
| 7.7b | Look at the top of the sheet | The question is in the most-urgent slot with both options, each carrying its ⌥ number |
| 7.7 | With the island open on that decision, press ⌥1 | It answers the decision. The dock does not switch |
| 7.7c | Type into the question field and press ⌥1 while typing | The character goes into the field; no decision is answered and no session switches |
| 7.8 | Press ✕ on a running pill | The shell dies, the pill stays, its dot turns red and it reads `exit N` |
| 7.9 | Press ✕ again on that pill | The pill goes |
| 7.10 | Right-click a running pill → Close | Gone in one step, no intermediate state |
| 7.11 | Right-click a pill → Rename, and double-click another | Both open the field; only one at a time; the row does not jump |
| 7.12 | Click the caret half of "New terminal" | Only `available` profiles are listed; picking one changes the caret's label and the next shell |
| 7.13 | Quit and relaunch | The picked profile is still selected |
| 7.14 | Open `⋯` | It opens *upward*, fully on screen. Adding a custom shell and changing the font still work, and the bar does not grow |
| 7.15 | Drag the dock's top edge to its smallest | The row survives at 120px; the terminal below it is still usable |
| 7.16 | Switch macOS to Light appearance | The bar, pills, dots and the split button all re-colour; nothing is unreadable |

Then T098, on the same shells:

| # | Do | Expect |
|---|---|---|
| 7.17 | Right-click a pill → Blue | A blue bar on the pill's left edge. The dot is unchanged and still says whether the shell is alive |
| 7.18 | Terminate that shell with ✕ | The bar stays blue; the dot turns red. Two facts, two channels |
| 7.19 | Reopen the menu on it | "Blue" is ticked; the others are not |
| 7.20 | → No colour | The bar goes; nothing takes its place |
| 7.21 | Colour three shells differently, then reload the window (⌘R in the dev build) | Every colour comes back with its session — it is core state, not the panel's |
| 7.22 | Open `crates/terminal/src/system.rs`, then "New terminal" | The shell starts in `crates/terminal/src`, and the pill says `crates/terminal/src` |
| 7.23 | Switch to a `README.md` tab, then "New terminal" | The workspace root, as before. Hover "New terminal" first — the tooltip says where it will land |
| 7.24 | Close every editor tab, then "New terminal" | The workspace root |
| 7.25 | Open a file, delete its folder in Finder, then "New terminal" | A shell at the workspace root, with no error — the fallback, not a failed spawn |
| 7.26 | With no folder open at all, "New terminal" | `~`, as before |
| 7.27 | In Light appearance, colour a pill each of the six | All six are visible against the bar; none disappears |

Rows filled: the T096 and T098 completion reports' "not yet exercised by a pointer".

---

## Session 8 — Workstreams, the queue and ownership, T210/T325 (~15 min)

**Not needed for ADR-001.** Sessions 1–7 decide the runtime; 8 and 9 close the
review queue. Run them when you want those six tasks off `review`, in either
order relative to the rest.

Nothing in either task has been seen in the running app. Open `small-repo`, and
keep 7.7a–7.7c in mind — they raise a question, which is the state several steps
below need and which nothing else in the runbook produces.

**The island itself**

| # | Do | Expect |
|---|---|---|
| 8.1 | Press **⌥A**, then **⌥A** again | The sheet opens and closes. It is a floating layer: **the tab strip must not move down by a pixel** and nothing below it reflows |
| 8.2 | Open it, press **Escape**; open it, click in the editor | Both close it. (T048's `⋯` and health popovers do *not* do this — that is a known defect of theirs, not of the island) |
| 8.3 | Type a title into **New workstream**, submit | A row appears reading **Draft**, with a dot |
| 8.4 | Hover the row, then reach it with **Tab** instead | The move buttons appear on hover **and** on keyboard focus. Mouse-only here is a defect |
| 8.5 | Walk it **Ready → Start → Review → Done**, entirely from the row | Each click moves the row and relabels the state. Only the moves the core allows are ever offered — the renderer keeps no copy of the state machine |
| 8.6 | On the **Done** row, look at what is offered | No **Block** and no **Abandon**: the row filters them deliberately. If a move you expect is missing, that is the core's answer, not a bug |

**A refusal, and the two surfaces agreeing**

| # | Do | Expect |
|---|---|---|
| 8.7 | Raise a question on a **In progress** workstream (7.7a) | The row goes to **Blocked** and reads "1 open question". The core blocks running work on a question; the renderer does not decide this |
| 8.8 | While it is Blocked, try to move it to **Review** | It is refused. **The row stays exactly where it was** and the core's message is what you read — nothing moves optimistically and springs back |
| 8.9 | Answer the question from the island's most-urgent slot | The row leaves Blocked and the work resumes **without a reload**. Watch the log: one event, not a refetch |
| 8.10 | Press **Open the queue**, then answer a question there instead | The queue replaces the sidebar and editor. The island and the queue must agree at every moment, in both directions — answer in one, look at the other |
| 8.11 | In the queue, click each filter: **All, Questions, Reviews, Conflicts** | Four filters, each with a count that matches the list. An empty filter says "Nothing under …", never a blank pane |
| 8.12 | Press **Back to the editor** | The editor comes back with the same tabs, unchanged |

**Ownership, T325**

| # | Do | Expect |
|---|---|---|
| 8.13 | In the island, find the **Ownership** section with nothing claimed | "Nothing is claimed", and a claim form below it |
| 8.14 | Claim a **folder** for one workstream: pick it, pick Folder, type `crates`, press **Claim** | A lease row appears reading **Held**, naming the target and the holding workstream. The roster row now carries **`owns 1`** |
| 8.15 | Claim `crates` again for a *different* workstream | Refused where you clicked: "… is already owned. The conflict is in the attention queue." Check the queue's **Conflicts** filter — the refusal is a row there, naming the holder |
| 8.16 | Take the holding workstream to **Done**, then look at its lease | It reads **"Held by finished work"** with a different dot. It still keeps others out; only you take it away |
| 8.17 | Press **Release** on it | The row goes and the ground is claimable again |
| 8.18 | **Read the section as a stranger would.** Does anything here make you believe a claimed file is *protected*? | The note says Gritty refuses a second claim and does **not** stop a write. T325's own report names this as its main risk: "The registry is believable, and it protects nothing." **This judgement is the single most useful thing you can record about this task** — if the wording oversells, say so in your own words |

Rows filled: T210's seven perceptual claims and T325's five, none of which had a
step before.

---

## Session 9 — Agents: the rail, the timeline and the hook, T220/T231/T232/T221 (~25 min)

**This session needs a real Claude Code session running in another terminal**,
because that is the only thing that produces the state four tasks are waiting
to be seen in. Have one ready before you start, working in a *different* folder
from the one Gritty has open.

Check Session 0 step 0.1 first: without `target/debug/gritty-agent-hook` the
whole hook surface reads unavailable, and that is not a defect.

**The roster and the rail (T220, T231)**

| # | Do | Expect |
|---|---|---|
| 9.1 | With a Claude Code session running elsewhere, press **⌥A** and look at the **Agents** section | The section renders at all — T220's report says this "has not been seen in the running app". The session is listed with its task line and the tool in flight |
| 9.2 | Press **⌥G** | The agent rail opens on the right, listing every session |
| 9.3 | Watch a session's marker as it works, blocks and finishes | The marker changes with it. **Look at the five shapes — filled disc, diamond, square, ring, bar — and say whether you can tell them apart at rail size with the colour ignored.** Colour alone would be a defect |
| 9.4 | Quit the Claude Code session | Its row flips to **Ended** within one poll interval, and stays listed |
| 9.5 | Drag the rail's gutter to resize; quit and relaunch | The width comes back |
| 9.6 | Collapse the rail | The needs-you count is **still visible** on the collapsed strip. A collapsed rail that hides the one number worth seeing is worthless |
| 9.7 | Shrink the window until something has to give | The rail shrinks toward its collapsed strip; the editor never goes below its minimum |
| 9.8 | **Click into a terminal so the shell has focus, then press ⌥G** | The rail toggles and **no `©` is typed into the shell**. This was reasoned from an xterm source read and never run — it is the step most likely to fail |
| 9.9 | With the island open (⌥A), press **⌥G**; then with a terminal focused press **⌥1** | ⌥G does nothing while the island is open, and the dock's ⌥1…⌥9 and the island's ⌥A never fight each other |

**The timeline (T232)**

| # | Do | Expect |
|---|---|---|
| 9.10 | Open a session's timeline and watch it while the agent works | The feed grows live |
| 9.11 | Scroll up a few entries and wait for new ones | **You stay where you put yourself.** A feed that yanks you back to the bottom while you are reading is the defect this pin exists to prevent |
| 9.12 | Find a file the agent just edited in **files touched** and click it | It opens with the **current** contents from disk. Open the same file first, edit it without saving, then click it again → the ordinary dirty-buffer conflict, never a silent overwrite |
| 9.13 | Look for a file the agent touched **outside** the workspace | Listed, and **not clickable** |
| 9.14 | Kill and restart the agent mid-timeline | The feed draws an explicit **gap** rather than pretending it is continuous |
| 9.15 | Let a long tool result arrive | The rail never scrolls sideways as a whole, and the files-touched list is still reachable |

**The hook, stage A — a scratch home, nothing real is touched (T221)**

The installer edits `~/.claude/settings.json`, which is the file every Claude
Code session on this machine reads. Gritty honours an absolute `HOME`, so this
half is genuinely safe: everything below happens inside a temporary directory
you can delete.

```sh
export FAKEHOME=/tmp/gritty-manual-home
rm -rf "$FAKEHOME" && mkdir -p "$FAKEHOME/.claude"
cat > "$FAKEHOME/.claude/settings.json" <<'JSON'
{ "hooks": { "PreToolUse": [ { "matcher": "Bash", "hooks": [
  { "type": "command", "command": "/usr/bin/true" } ] } ] } }
JSON
HOME=$FAKEHOME GRITTY_DATA_DIR=/tmp/gritty-manual-home-data GRITTY_LOG=debug pnpm dev
```

| # | Do | Expect |
|---|---|---|
| 9.16 | ⌥A → Agents section header → **Install…** | The proposed change is shown **before anything is applied**, and it names the backup path it will write |
| 9.17 | Press **Apply**, then read `$FAKEHOME/.claude/settings.json` | Gritty's entry is there **and the `/usr/bin/true` entry is byte-identical**. Key order and indentation survived |
| 9.18 | Press **Uninstall…** → **Remove**, then `diff` the file against the backup | No difference. This is the obligation ADR-018 puts on the installer, and the first time it has been checked against a file rather than a fixture |
| 9.19 | Note **where the control is** | §25 says "in settings, not in the roster". It renders in the island's agents-section header, which the code argues *is* settings. Worth an eye rather than an argument — say whether you would have found it |

**The hook, stage B — the real home, the only way to see `BLOCKED`**

Stage A cannot produce a `BLOCKED` row: the hook only fires for a Claude Code
session that reads the config Gritty wrote, and a real session reads the real
file. Take your own copy first — the installer writes its own backup too, and
both are worth having.

```sh
cp ~/.claude/settings.json ~/.claude/settings.json.manualpass
```

| # | Do | Expect |
|---|---|---|
| 9.20 | Relaunch Gritty normally (no `HOME` override) and install the hook | Same preview, same apply |
| 9.21 | In the other terminal, get a Claude Code session to a **permission prompt** | Within one interval the row reads **Blocked**, and the row says the verdict came from the hook — `activitySource: HOOK` (T221 §24). The label reads **"Pending validation"**, not "Blocked": that phrasing is the point of §24, and whether it lands is your call |
| 9.22 | Open that session's timeline while it sits at the prompt | The **permission prompt is the last row of the feed** (T232). It is a state, not an entry: it is replaced, never accumulated, and a new prompt re-pins the feed once |
| 9.23 | Answer the prompt | The row leaves Blocked and the feed row goes with it |
| 9.24 | Get back to a prompt, then **kill the agent while it waits there** | The row reads **Ended**, not Blocked. A dead agent that still claims to need you is the worst outcome this design has |
| 9.25 | Uninstall, then `diff ~/.claude/settings.json ~/.claude/settings.json.manualpass` | No difference. If there is one, **stop and keep both files** — that is the highest-stakes defect in the whole runbook |
| 9.26 | `rm -rf /tmp/gritty-manual-home /tmp/gritty-manual-home-data` and remove your copy once 9.25 passed | Clean |

Rows filled: T220's four claims, T231's eight, T232's six and T221's six —
thirty of the forty-three that `docs/testing/review-findings.md` lists as having
no step anywhere.

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
4. Steps 8.18 and 9.3 ask for a **judgement**, not a tick. Answer them in your
   own words even if everything worked — they are the two places where a
   passing row and a good design are different things.

The agent transcribes all of it into `docs/testing/platform-matrix.md` and
`docs/plans/phase-0-spike.md`, with your observations attributed to the pass
rather than to a test.

Sessions 8 and 9 have one more consequence: they are what lets T210, T220,
T221, T231, T232 and T325 leave `review`. An agent never sets `done`
(`AGENTS.md`), so those six flips are yours and they need this evidence first.
