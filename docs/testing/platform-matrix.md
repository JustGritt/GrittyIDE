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
| Input | keyboard shortcuts (Cmd/Ctrl combos) | – | – | – | T040 |
| Input | multi-cursor, selection, find/replace | – | – | – | T040 |
| Input | clipboard copy/cut/paste in editor and inputs | – | – | – | T040 |
| Input | IME composition, dead keys, international layouts | – | – | – | T040 |
| Input | drag and drop of text and files | – | – | – | T040 |
| Rendering | font rendering, high-DPI, zoom/scaling | – | – | – | T040 |
| Rendering | light/dark theme switch | – | – | – | T000+ |
| Files | native directory picker opens a workspace | – | – | – | T020; picker runs on the Rust side |
| Files | Unicode filenames, spaces, symlinks, broken symlinks | – | – | (a) | T020: `crates/workspace/tests/listing.rs` on Windows 11 (symlinks via Developer Mode); explorer check pending |
| Files | case-insensitive vs case-sensitive path identity | – | – | (a) | T020: canonicalization returns on-disk spelling on NTFS |
| Files | 10k+ file repository opens without a recursive scan | – | – | (a) | T020: generated `large-repo`, root listed in < 100 ms, one `read_dir` per call |
| Files | directory with > 5000 entries pages with a cursor | – | – | (a) | T020 |
| Files | root removed while open → `NotFound` + `WorkspaceClosed` | – | – | (a) | T020: core and adapter tests |
| Files | permission errors surfaced, not crashed | – | – | – | T030 |
| Files | external modification detection | – | – | – | T030/T070 |
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
