# Phase 0 — Architecture and runtime spike

Executable checklist. Fill the result columns with what was actually observed
(date, machine, numbers). Nothing here is "done" until it was exercised in the
running application. Exit: ADR-001 go/no-go.

## A. Functional checklist (plan §16)

| # | Item | Task | Result |
|---|---|---|---|
| A1 | Minimal Tauri 2 + React + TypeScript application launches | T000 | 2026-09-04, macOS 26 arm64: launched both ways with `GRITTY_DATA_DIR` and the log captured. Dev (`pnpm dev`, debug Rust + Vite dev server): application built 61 ms, window shown 228 ms, first WebView health 1665 ms. Packaged debug bundle (`pnpm tauri build --debug --bundles app`, production CSP): 72 ms / 250 ms / **754 ms** — the dev server accounted for most of the difference. Timings come from the `startup milestone` log lines; release numbers belong to §C. A human looking at the window is Session 1 of `docs/testing/phase-0-manual-pass.md` |
| A2 | Monaco embedded; TypeScript, Rust, Markdown and JSON fixtures open | T040 | 2026-09-03, macOS 26 arm64: Monaco 0.56.0 embedded (`packages/editor`, ADR-012); the bundle loads in WKWebView 21624.4.5 and the health round trip completes (`pnpm dev`); opening the `small-repo` fixtures in the running app pending reviewer (the agent cannot see the WebView) |
| A3 | Keyboard shortcuts, multi-cursor, selection, find, clipboard, drag/drop, font rendering verified on WKWebView | T040 | pending reviewer; rows and steps in `docs/testing/platform-matrix.md` (T040 completion report lists the exact checks) |
| A4 | Repository with 10k+ files opens without eager content loading | T020 | 2026-09-04, macOS 26 arm64, release bundle: `fixtures/large-repo` (10 501 files, 103 directories, depth 25) restored as the workspace. `workspace.list` 0.29 ms median (0.88 ms max) and launch to first WebView round trip 502 ms — indistinguishable from an empty workspace, because only the root is listed and nothing is read. Expanding the >5000-entry directory in the explorer is manual pass step 1.10 |
| A5 | Rust commands for directory listing, safe read, atomic write | T020/T030 | 2026-09-03, macOS 26 arm64: `workspace.list` (T020) and `documents.open/save` (T030) through the real adapters in `cargo test -p gritty-desktop`; atomic write with injected failure/crash in `cargo test -p gritty-workspace`; UI pass pending reviewer |
| A6 | Filesystem watching; rapid changes do not lock the UI | T070 | 2026-09-06, macOS 26 arm64: `notify` 8.2.0 behind `gritty-workspace::watch` (ADR-025). Attaching a recursive FSEvents watch to `fixtures/large-repo` (10 501 files, 103 directories, depth 25) took **18–52 ms**; a change is coalesced on a 200 ms quiet period with a 1 s ceiling and capped at 64 directories, so the renderer receives at most ~5 events/s no matter what the disk does. Measured by `cargo test -p gritty-workspace --test watch -- --ignored --nocapture`; the storm itself is B5. UI responsiveness on screen is pending reviewer (the agent cannot see the WebView) |
| A7 | Markdown preview with sanitized HTML and fenced-code highlighting | T050 | 2026-09-03, macOS 26 arm64: markdown-it + DOMPurify allowlist (ADR-013), fenced code colorized by Monaco's tokenizers; XSS fixture asserted clean and GFM fixture snapshotted under jsdom; `long.md` (134 KB) renders in 366 ms under jsdom (markdown-it alone 15–20 ms); in-app rendering, scroll sync and link navigation pending reviewer (the agent cannot see the WebView) |
| A8 | PTY: spawn a shell, resize, stream output, terminate cleanly | T090 | 2026-09-04, macOS 26 arm64: `portable-pty` 0.9 behind `gritty-terminal` (ADR-015); `cargo test -p gritty-terminal` drives a real PTY through echo, exit codes, `TERM`, `stty size` before and after a resize, a `yes` flood, an unexpected `SIGKILL` and a process-tree kill verified by pid (15 tests, 1.6 s); `pnpm dev` boots on protocol 1.4.0 with the xterm bundle loaded in WKWebView 21624.4.5; driving a shell in the window pending reviewer (the agent cannot see the WebView) |
| A9 | Persist and restore workspace + tabs | T060 | 2026-09-04, macOS 26 arm64: SQLite session database (ADR-014); `session.restore` reopens the last root and returns tabs, view modes, cursors, preferences and layout; automated through the adapters (`cargo test -p gritty-desktop`) and the WebView restore flow (Vitest); `pnpm dev` twice restores the window geometry; a quit/relaunch with open tabs in the app pending reviewer |
| A10 | Cold/warm startup, idle RAM/CPU, editor RAM growth, interaction latency recorded | below | 2026-09-04: §C filled for everything measurable without a person — cold 496 ms, warm 504 ms, idle 0.97 % of one core, 607 MiB against Cursor's 1194 MiB, core-side command latencies. Typing responsiveness and every *perceived* latency remain with the human pass |
| A11 | Linux build via CI; platform-specific defects logged | T000 CI + platform matrix | 2026-09-04, CI run 33876904661 on `main` (4807b09), both jobs green: **Linux** (ubuntu-24.04, WebKitGTK 4.1) `pnpm check` + `pnpm check:generated` + `pnpm tauri build --no-bundle`, 10m10s; **macOS** (macos-15) the same plus `pnpm tauri build --bundles app`, 3m18s, artifact uploaded. The macOS job is gated only for pull requests — pushes to `main` always run it. Defects logged so far: the T090 output-storm test asserted a throughput this Mac provides and the runner does not (fixed by injecting the buffer cap). No WebKitGTK behavioural defect is known, because no human has driven Linux — recorded as untested, not as working |
| A12 | ADR-001 written after measurements | end | |

## B. Failure and recovery tests (addendum §11)

| # | Test | Steps | Pass condition | Result |
|---|---|---|---|---|
| B1 | Dirty-buffer crash | open workspace → edit without saving → `kill -9` Gritty → restart | unsaved content is recoverable; no silent loss | 2026-09-04, macOS 26 arm64: automated in `crates/gritty-core/tests/crash_recovery.rs` (snapshot written within the debounce, process state dropped without close/flush/checkpoint, relaunch offers `SAME_AS_DISK` and recovers the text; changed and deleted files recover as `CONFLICTED` / `DELETED_EXTERNALLY`) and through the adapters; the real `kill -9` of the app while typing pending reviewer |
| B2 | PTY failure | spawn PTY → produce output → kill the child unexpectedly | renderer stays responsive; terminal reports termination | 2026-09-04, macOS 26 arm64: `killing_the_child_unexpectedly_reports_termination` (`crates/terminal/tests/pty.rs`) SIGKILLs the shell behind Gritty's back; the session reports `signal: SIGKILL`, `code: None`, exactly one `TerminalExited`, and no output after it. Renderer containment is structural (output never enters React state, delivery capped at ~60 Hz) and is asserted in `output_delivery_is_rate_limited_and_does_not_starve_the_control_path`: a resize still completes while `yes` floods. The renderer staying responsive on screen is pending reviewer |
| B3 | Operation cancellation | start a long operation (search or fixture op) → receive progress → cancel | prompt cancellation; no stale successful completion | 2026-09-03, demo operation (T010): automated in `cargo test -p gritty-core` (registry) and `cargo test -p gritty-desktop` (mock runtime through the real adapters): progress observed → cancel → `CANCELLED` in < 1 s, no `OperationCompleted`, `seq` contiguous. Manual UI pass pending reviewer; re-run with search (T080) |
| B4 | Renderer reload | reload/recreate the WebView while core/workspace exist | durable state rehydrates without corruption | 2026-09-04: `documents.list` + `documents.open` rehydration kept (`useDocuments.rehydrate`, always run by the session flow); a second `session.restore` in the same process reports the open workspace without reopening (core and adapter tests); the WebView flow is unit-tested for the reload path; a Cmd+R in `pnpm dev` with dirty tabs pending reviewer |
| B5 | Filesystem storm | generate rapid file changes in `fixtures/large-repo` | UI responsive; events batched/bounded; final state converges | 2026-09-06, macOS 26 arm64: **590 filesystem operations across 75 directories in 65 ms produced exactly one `WorkspaceEntriesChanged`**, `overflowed: true` with an empty list — never a partial list presented as complete. A narrower storm (350 operations, 50 directories) produced one batch naming all 50. The next single change after the storm was reported on its own, so it converges rather than latching. Automated as `the_large_repo_storm_stays_batched_and_bounded` (`crates/workspace/tests/watch.rs`, `#[ignore]`d: it churns a 10k-file tree, and its numbers are a measurement, not a threshold). The renderer's half is bounded by construction — a batch re-lists only the directories the user expanded, and `overflowed` re-lists all of those — and is asserted in `explorer-reducer.test.ts`; the window staying responsive on screen is pending reviewer |

## C. Performance baseline (plan §13 targets)

Measure on the primary development Mac with `fixtures/large-repo` and a real
repository. Record the command/method used.

**Every number here is an observation on one machine, never an assertion.**
Nothing in this section becomes a CI test. The T090 output-storm test is the
warning: a threshold calibrated on this Mac passed here and failed on a Linux
runner, twice. Record the machine with every row; where a bound must be
checked, force the condition instead of trusting the hardware to produce it.

Two instruments make these numbers cheap to collect (added with this section):

- `startup milestone` log lines — `application built`, `window shown`,
  `first webview health` — each with `elapsed_ms` since the process began.
  `first webview health` is the WebView's first round trip through the
  protocol, which is the closest honest proxy for "the shell is usable".
- `command completed … elapsed_us` on every command, from the single `guard`
  wrapper in `apps/desktop/src-tauri/src/commands.rs`. This means the human
  pass produces core-side timings for everything it touches as a byproduct;
  `docs/testing/phase-0-manual-pass.md` captures the log for exactly that.

The milestones are `INFO`, so launch timings are collected at the default log
filter and the measurement does not pay for the debug logging it is measuring;
the per-command line is `DEBUG` and needs `GRITTY_LOG=debug`. Both go to
stderr. Perceived latency still needs a person either way: a core-side
`documents.open` of 400 µs says nothing about what the user felt three frames
later.

| Metric | Target | Measured (2026-09-04, MacBook M-series, 16 GB, macOS 26 arm64, release bundle) | Method |
|---|---|---|---|
| Cold launch to usable shell | ≤ 2 s | **496 ms** median of 5 (483–556). Decomposed: application built 42 ms, window shown 181 ms, first WebView round trip 496 ms | Release bundle, `GRITTY_DATA_DIR` deleted before each run, default log filter; `startup milestone … "first webview health"`. `scripts/measure-phase-0.sh launch cold <dir> 5` |
| Warm launch | ≤ 1 s | **504 ms** median of 5 (495–508) with a session database and window geometry; **504 ms** median of 3 when also restoring a workspace and three tabs | Same, keeping the data directory (`… launch warm <dir> 5 keep`). Warm is not faster than cold because startup does almost nothing either way — the restore work is sub-millisecond and does not gate the window |
| Idle CPU (no watchers/tasks) | ~0 % | **0.064 % of one core** — 0.86 s of CPU across all four processes over 22.5 minutes (Gritty 0.10 s, WebContent 0.66 s, GPU 0.07 s, Networking 0.03 s). Target met | Cumulative CPU-time deltas (`ps -o time=`), not `%cpu`, which is an average over the process lifetime. **Measure over tens of minutes, not one:** the same method over the 60 s *after launch* reads 0.97 %, which is the app still settling, not idle |
| Typing responsiveness | no perceptible frame drops | pending the human pass | Manual pass step 2.25 (Web Inspector *Timelines*, frames over 16 ms, 12 MiB fixture). No core-side proxy exists: typing is debounced before it reaches a command |
| Open a normal source file | ≤ 100 ms perceived | core side **0.23 ms** median (`documents.open`, n=9, max 0.55 ms); perceived pending the human pass | Restore of three tabs in `fixtures/small-repo` under `GRITTY_LOG=debug`. The core is three orders of magnitude inside the target; whether the *screen* is, is step 2.29 |
| Workspace restore | UI immediately, expensive work incremental | **the window appears at the same time with or without a workspace to restore** (504 ms vs 504 ms). `session.restore` 0.45 ms, `workspace.list` 0.35 ms, three `documents.open` 0.23 ms each | Session seeded directly into SQLite (`… seed <dir> <root> <tabs…>`) so the real restore path runs without anyone clicking |
| Repository enumeration | never blocks the editor; progressive | `fixtures/large-repo` (10 501 files, 103 directories, depth 25): `workspace.list` **0.29 ms** median, 0.88 ms max; launch **502 ms**, indistinguishable from an empty workspace | Same seeding, root `fixtures/large-repo`. Only the root is listed: one `read_dir` per expanded directory, nothing recursive. Paging a >5000-entry directory is manual pass step 1.10 |
| Baseline memory (shell only) | track; compare with an Electron baseline | Two minutes after opening a workspace with three tabs: **607 MiB RSS** (WebContent 432, Gritty 122, GPU 37, Networking 16). **Cursor, same workspace, extensions disabled, same moment: 1194 MiB.** After 22 minutes idle Gritty settles to **228 MB physical footprint** (WebContent 190, Gritty 37; peak footprint 461 MB) | RSS summed over the app and the WebKit helpers it spawned, found by diffing the machine's WebKit processes across launch. Gritty and Cursor were measured identically at the same point in their lifetime, which is what makes the comparison fair — the absolutes are upper bounds, since RSS counts shared framework pages |
| Memory growth after 30 min | track | **No growth.** Over 22.5 minutes idle the app allocated nothing measurable (0.86 s CPU total) and resident memory *fell* from 607 to ~230 MiB as macOS reclaimed untouched pages | Idle, not editing — the agent cannot type into the window. **RSS is the wrong metric here:** it falls while nothing is allocated, so it cannot tell "no leak" from "reclaimed". `Physical footprint` (`vmmap --summary`, what Activity Monitor calls Memory) is the metric, and the harness now records it. A real editing-growth number needs a person at the keyboard |
| Agent/LSP/indexer crash | no renderer crash or freeze | not applicable yet | No agent, language server or indexer exists (T300, Phase 6). The only supervised child today is a PTY, covered by B2. Recorded as out of scope rather than as passing |

### How to read these

- **The machine:** MacBook, Apple silicon, 16 GB, macOS 26 arm64, release bundle
  built from `phase-0-measurement`. Nothing else running; a stale `pnpm dev`
  from an earlier session was killed first because it moved every number.
- **"Cold" means a fresh data directory, not a cold page cache.** The binary had
  been built and run minutes earlier, so the OS had it cached. A genuine
  first-launch-after-boot is slower than 496 ms; this is a floor, not a ceiling.
- **RSS on macOS counts shared framework pages**, so 607 MiB and 1194 MiB are
  both upper bounds. They were measured the same way, so the ratio — Gritty at
  **51 % of Cursor's memory and 29 % of its idle CPU** — is the fair number, not
  the absolutes.
- **Cursor is a full IDE, not an empty Electron shell.** It is the honest
  comparison for "what a user would otherwise run", not a measurement of
  Electron itself. A bare Electron window would be far smaller than 1194 MiB.
- **Command timings are core side.** `documents.open` at 0.23 ms says the core
  is not the bottleneck; it says nothing about what the user sees three frames
  later. Every perceived-latency row still needs the human pass.
- **Two measurement traps met here, both worth inheriting.** A 30-minute soak
  run while the agent was compiling recorded memory *falling* by 336 MiB —
  macOS compressing the WebView's pages under the pressure the build created.
  Nothing else may run during a memory measurement. And once that was fixed,
  RSS still fell, because macOS reclaims an idle app's untouched pages: the
  metric, not the machine, was wrong. Physical footprint is the one that holds
  still.
- The harness is `scripts/measure-phase-0.sh`, so these can be re-run and
  compared rather than argued about.


## D. Go/no-go questions (addendum §19)

Answer each explicitly with evidence.

**Editor.** Does Monaco behave correctly in WKWebView? Are shortcuts, clipboard,
multi-cursor and text input dependable? Any unresolved IME/input defects?

**Runtime.** Do filesystem operations stay outside the renderer cleanly? Can
high-frequency events be delivered without UI degradation? Does the typed
protocol stay understandable rather than becoming Tauri glue?

**Terminal.** Is PTY behavior reliable for daily interactive use? Can child
process trees be terminated predictably? Does unexpected child death stay contained?

*2026-09-04, macOS 26 arm64 (T090, ADR-015).* **Reliable enough to keep going,
with one caveat.** A real shell runs, echoes, reports its exit code, and
`stty size` follows a resize; `TERM=xterm-256color` reaches the child;
concurrent sessions do not cross. The caveat is not the PTY but the shell: an
interactive line editor is not ready the instant the PTY opens, and input
written before the first prompt is silently mangled or lost. A user types after
seeing a prompt, so this does not affect interactive use — but anything driving
a terminal programmatically (an agent, T300) must wait for readiness, and the
integration tests had to pin a shell and generate its rc files to be
deterministic at all. Running a developer's own login shell, oh-my-zsh asked
about updates and autosuggestions rewrote the input as it arrived.

**Process trees: yes, with a documented limit.** `killpg` over the shell's own
process group *and* the terminal's current foreground group, escalating
SIGHUP → SIGTERM → SIGKILL, re-reading the foreground group between signals.
`kill_terminates_the_whole_process_tree` starts a background job (`sleep 300 &`,
its own process group under job control), records its pid, kills the session
and asserts the pid is gone — so the SIGHUP-first design that relies on the
shell forwarding to its jobs is verified, not assumed. Dropping a session
handle kills it, and `Core::shutdown` kills every session before the session
writer flushes. A process that puts itself in a new session (`setsid`, a
daemon) survives, exactly as when any terminal closes; that is recorded rather
than worked around.

**Unexpected death is contained.** SIGKILL behind Gritty's back produces one
`TerminalExited` carrying `SIGKILL`, after the last output and with nothing
following it, and the registry marks the session `EXITED` and drops the pid.
The exit signal is normalized: `strsignal` says "Killed: 9" on macOS and
"Killed" on Linux, which is platform text, not a protocol value.

**Flooding does not reach the renderer.** A `yes` flood overflows the buffer:
the oldest bytes go, the count is reported as `truncatedBytes` and drawn as a
visible marker, and batches stay under the cap. Separately, delivery stays rate
limited against a measured ceiling of ~70/s and a resize completes during the
flood, so the control path is not starved. Output never passes through React
state at all.

Getting that test right took two attempts and the lesson is worth more than the
test. The first version asserted that the flood overflowed the production 1 MiB
buffer — which requires the shell to outrun a 60 Hz drain, true on the
development Mac and false on a Linux CI runner, where it failed. The second
version slowed the *consumer* down, which only widened the drain window from
16 ms to 66 ms: still a throughput assumption, and it failed on CI too. The
buffer cap is now injectable, the integration test uses a 16 KiB one (smaller
than a single read, so overflow is certain on any machine), and the size of the
production cap is pinned by the deterministic unit tests in `batch.rs`.

**Worth carrying into §C below.** The performance baseline is entirely
thresholds. A threshold calibrated on this Mac will pass here and fail in CI
exactly the way this did. Record the machine with every number, and where a
test must assert a bound, force the condition rather than relying on the
hardware to produce it.

Remaining for the reviewer: the same checks with a human at the window (`vim`,
`less`, `top`, Ctrl+C, tab close verified with `pgrep`, the flood on screen).

**Recovery.** Does workspace/session state survive force termination? Can unsaved
buffers be recovered? Do interrupted operations reconcile correctly?

**Performance.** Startup acceptable? Idle CPU effectively zero? Baseline memory
materially better than Electron? Does repository size degrade progressively?

**Platform.** Does Linux remain buildable? Are WebView differences manageable
behind adapters/tests?

**Maintenance.** Are Tauri/WebView workarounds isolated? Does the Rust/TypeScript
split still produce a clean architecture? Are shell-specific hacks accumulating
faster than product functionality?

## E. Kill criteria (ADR-001)

Recurring Monaco/input defects on macOS; unstable PTY/process behavior requiring
invasive workarounds; unacceptable maintenance from WebView divergence. If
triggered: keep the core protocol, evaluate Electron as the shell.
