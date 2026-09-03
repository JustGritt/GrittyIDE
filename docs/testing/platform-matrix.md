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
| Files | workspace close refused while documents are dirty | (a) | – | – | T030: core and adapter tests |
| Terminal | interactive shell (vim, less, top) | – | – | – | T090 |
| Terminal | resize, process-tree kill, unexpected child death | – | – | – | T090 |
| Recovery | force-kill restart restores tabs/cursor/dirty buffers | – | – | – | T060 |
| System | sleep/wake, multi-monitor, window restore | – | – | – | T060 |
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
