# Platform matrix

Record observed behavior per platform. "Untested" is a valid entry; "works" is
only valid after a manual check on that platform. Add rows as features land.

Windows is not a supported target before Phase 8 (ADR-002); the column exists
because the development machine moved to Windows 11 during T020, so
observations from there are recorded without claiming support.

Legend: ✔ works · ✘ defect (link an issue/note) · ~ partial · – untested · (a) automated test only

| Area | Check | macOS (WKWebView) | Linux (WebKitGTK) | Windows (WebView2) | Notes |
|---|---|---|---|---|---|
| Shell | app launches, window sizing, min size | – | – | ~ | T000; T020: `pnpm dev` launched on Windows 11 (2026-09-03), sizing not checked |
| Shell | health round-trip under strict CSP | – | – | ~ | T000; T020: health request from the WebView observed in the app log on Windows 11 |
| Input | keyboard shortcuts (Cmd/Ctrl combos) | – | – | – | T040: Monaco 0.56 shell; Cmd+S bound inside Monaco and on the window; Cmd+W left to the native menu; keystroke checks pending the human WKWebView pass |
| Input | multi-cursor, selection, find/replace | – | – | – | T040: full Monaco entry (find widget, multi-cursor, folding, bracket matching registered); pending the human pass |
| Input | clipboard copy/cut/paste in editor and inputs | – | – | – | T040: pending the human pass |
| Input | IME composition, dead keys, international layouts | – | – | – | T040: pending the human pass |
| Input | drag and drop of text and files | – | – | – | T040: tab reorder uses HTML5 drag and drop (Tauri `dragDropEnabled` may interfere); pending the human pass |
| Rendering | font rendering, high-DPI, zoom/scaling | – | – | – | T040: codicon font emitted as an asset (`font-src 'self'`); pending the human pass |
| Rendering | light/dark theme switch | – | – | – | T000+; T040 follows `prefers-color-scheme` (`vs` / `vs-dark`), pending the human pass |
| Files | native directory picker opens a workspace | – | – | – | T020; picker runs on the Rust side |
| Files | Unicode filenames, spaces, symlinks, broken symlinks | – | – | (a) | T020: `crates/workspace/tests/listing.rs` on Windows 11 (symlinks via Developer Mode); explorer check pending |
| Files | case-insensitive vs case-sensitive path identity | – | – | (a) | T020: canonicalization returns on-disk spelling on NTFS |
| Files | 10k+ file repository opens without a recursive scan | – | – | (a) | T020: generated `large-repo`, root listed in < 100 ms, one `read_dir` per call |
| Files | directory with > 5000 entries pages with a cursor | – | – | (a) | T020 |
| Files | root removed while open → `NotFound` + `WorkspaceClosed` | – | – | (a) | T020: core and adapter tests |
| Files | permission errors surfaced, not crashed | (a) | – | – | T030: `crates/workspace/tests/documents.rs` (mode-000 file → `PermissionDenied`, no document created) on macOS 26 (2026-09-03) |
| Files | external modification detection | (a) | – | – | T030: core and adapter tests (`sed`-style edit while dirty → `CONFLICTED`, save refused, force/reload paths) on macOS 26; UI banner check pending |
| Files | open / edit / save round trip in the running app | – | – | – | T040: Monaco surface; `GRITTY_LOG=debug pnpm dev` on macOS 26 (2026-09-03) loads the Monaco bundle in WKWebView 21624.4.5 and completes the health round trip; edit/save in the app pending the human pass |
| Files | atomic save leaves no temp file; injected crash keeps destination | (a) | – | – | T030: `crates/workspace/tests/documents.rs` |
| Files | binary / invalid UTF-8 / BOM / CRLF / large (12 MiB) open correctly | (a) | – | – | T030: staged `weird-files` fixture; large file read-only until enabled. T040: placeholders and large-file degradation in the pane, pending the human pass |
| Editor | model ↔ document binding: lazy reads, buffer generation, conflict refetch, close/orphans | (a) | – | – | T040: `packages/editor` (fake model) and desktop reducer/scheduler tests, macOS 26 (2026-09-03) |
| Editor | CRLF round trip and mixed-EOL rewrite in a real Monaco model | (a) | – | – | T040: jsdom test against monaco-editor 0.56.0 (`model-eol.dom.test.ts`); on-disk bytes after a save pending the human pass |
| Editor | workers (editor, JSON, CSS, HTML, TypeScript) load under the CSP | – | – | – | T040: chunks emitted by `pnpm --filter @gritty/desktop build`; runtime load in `pnpm dev` and in the packaged app pending the human pass (failures would show in the pane's worker warning) |
| Editor | typing in the 12 MiB fixture without perceptible frame drops | – | – | – | T040: explicit large-file degradation and lazy reads; measurement (Web Inspector timeline, frames > 16 ms) pending the human pass |
| Editor | dirty tab close asks save / discard / cancel; Cmd+S saves | – | – | – | T040: pending the human pass |
| Markdown | GFM rendering (headings, lists, task lists, tables, fences, quotes, links, images) | (a) | – | – | T050: `packages/markdown` file snapshot of `fixtures/markdown/gfm.md` under jsdom, macOS 26 (2026-09-03); visual check in WKWebView pending the human pass |
| Markdown | XSS fixture yields no script, handler, unsafe URL or forbidden element | (a) | – | – | T050: `sanitize.test.ts` on `fixtures/markdown/xss.md` (jsdom, DOMPurify 3.4.14); a look at the Web Inspector console in the app pending the human pass |
| Markdown | source / split / preview modes keep the editor state; preview follows edits within ~100 ms | – | – | – | T050: adaptive debounce and thresholds implemented; `gfm.md` parses in under 10 ms (markdown-it, node); timing in WKWebView pending the human pass |
| Markdown | long document (134 KB `long.md`) renders without freezing; huge documents pause / refuse | (a) | – | – | T050: `long.md` 366 ms under jsdom (markdown-it alone 15–20 ms, DOMPurify under jsdom is the rest); in-app pause banner and Refresh pending the human pass |
| Markdown | source → preview scroll sync | (a) | – | – | T050: interpolation unit-tested (`scroll-sync.test.ts`); feel in the app pending the human pass |
| Markdown | local link opens the target at `line:col`; anchor scrolls; missing target → inline notice, link marked | (a) | – | – | T050: resolver and link extraction tests; reveal flow pending the human pass |
| Markdown | external link opens the system browser exactly once through `shell.openExternal`; middle click and context menu never navigate | (a) | – | – | T050: core validation (`shell.rs`), adapter round trip with a recording opener, navigation-guard predicate (`lib.rs`); the browser actually opening and the guard in WKWebView pending the human pass |
| Markdown | README.md / AGENTS.md badges in explorer, tab and toolbar | – | – | – | T050: pending the human pass |
| Files | workspace close refused while documents are dirty | (a) | – | – | T030: core and adapter tests |
| Terminal | interactive shell (vim, less, top) | – | – | – | T090: `pnpm dev` boots with the xterm bundle loaded in WKWebView 21624.4.5 (macOS 26, 2026-09-04); running a full-screen program in the window pending the human pass |
| Terminal | shell starts, echoes, reports its exit code | (a) | – | – | T090: `crates/terminal/tests/pty.rs` against a real PTY (macOS 26, 2026-09-04); the tests pin a shell and generate its rc files, because a developer's own login shell rewrites input as it arrives |
| Terminal | resize reaches the child (`stty size` before and after) | (a) | – | – | T090: `resize_reaches_the_child`; a zero size is clamped, not rejected |
| Terminal | process-tree kill: background job dies with the session | (a) | – | – | T090: `kill_terminates_the_whole_process_tree` records the job's pid and asserts it is gone; verifying with `pgrep` after closing a tab in the window pending the human pass |
| Terminal | unexpected child death reports termination, renderer unaffected | (a) | – | – | T090: `killing_the_child_unexpectedly_reports_termination` (SIGKILL behind Gritty's back → one `TerminalExited` carrying `SIGKILL`, nothing after it); the renderer staying responsive on screen pending the human pass |
| Terminal | output storm (`yes`) stays bounded and reports what it dropped | (a) | – | – | T090: `an_output_storm_is_bounded_and_reports_what_it_dropped` (slow consumer, 1 MiB cap) and `output_delivery_is_rate_limited_and_does_not_starve_the_control_path` (~70/s ceiling, a resize completes during the flood), both green on macOS 26 and Linux CI; frame timing in the window pending the human pass |
| Terminal | Ctrl+C interrupts; keystrokes reach the shell verbatim | (a) | – | – | T090: `U+0003` travels unescaped through the client, the adapter and the core (protocol, adapter and core tests); interrupting a real program pending the human pass |
| Terminal | tabs keep scrollback; exit status shown in the tab | – | – | – | T090: views stay mounted while hidden; pending the human pass |
| Terminal | a shell cannot be started outside the workspace | (a) | – | – | T090: adapter test refuses `../..` and `/etc` with nothing spawned |
| Recovery | force-kill restart restores tabs/cursor/dirty buffers | (a) | – | – | T060: `crates/gritty-core/tests/crash_recovery.rs` drops the process state without close, flush or checkpoint and recovers in all three states; `cargo test -p gritty-desktop` restores tabs and recovers through the adapters after a "restart" on the same data dir (macOS 26, 2026-09-04); the real `kill -9 $(pgrep -f Gritty)` while typing, relaunch and prompt pending the human pass |
| Recovery | snapshot vs. disk: same → prompt (default Restore), changed → CONFLICTED, missing → DELETED_EXTERNALLY | (a) | – | – | T060: reconciliation matrix in `session::tests` and `crash_recovery.rs`; banners in the app pending the human pass |
| Recovery | corrupted database → moved aside, fresh start, notice | (a) | – | – | T060: garbage file and damaged page (`quick_check` → replaced at runtime) in `crates/persistence/tests/database.rs` and `session::tests`; the notice banner in the app pending the human pass |
| Recovery | renderer reload rehydrates from the core (§11.4) | (a) | – | – | T060: `session.restore` idempotence (core, adapter) and `restore-flow.test.ts` reload path; a Cmd+R in `pnpm dev` with dirty tabs pending the human pass |
| Recovery | quit / relaunch restores workspace, tabs, active tab, view modes, cursors, preferences | (a) | – | – | T060: adapter and Vitest coverage; in the app pending the human pass |
| System | window geometry restore (size, position, maximized) | ~ | – | – | T060: `pnpm dev` twice with `GRITTY_DATA_DIR` on macOS 26 (2026-09-04): the second launch logged `restoring window geometry width=2560 height=1600 position=Some((232, 142))` from the row the first launch recorded; off-monitor positions dropped (`fit_to_monitors` tests); Linux untested by design (Wayland ignores positions, GTK maximize-before-show) |
| System | sleep/wake, multi-monitor | – | – | – | T060: pending the human pass |
| Build | `pnpm tauri build` produces a launchable bundle | – | – (CI: no-bundle) | – | T000 |

## Windows development notes (T020)

- Test binaries need the Common Controls v6 manifest that tauri-build only
  embeds into the app binary; `apps/desktop/src-tauri/build.rs` embeds
  `windows-test-manifest.xml` into test targets, otherwise every test binary
  linking the dialog plugin fails to load (`STATUS_ENTRYPOINT_NOT_FOUND`).
- The large-repo generator writes through the canonical (verbatim `\\?\`) base
  path, so the 24-level chain is not subject to the legacy `MAX_PATH` limits;
  the tests canonicalize the repository root before joining fixture paths.
- Git for Windows checks symlinks out as text files unless `core.symlinks` is
  on; the committed `fixtures/small-repo/link-to-src` therefore appears as a
  file there, and the Rust tests create real symlinks in a temporary tree.
