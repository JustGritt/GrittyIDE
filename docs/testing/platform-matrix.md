# Platform matrix

Record observed behavior per platform. "Untested" is a valid entry; "works" is
only valid after a manual check on that platform. Add rows as features land.

Legend: ✔ works · ✘ defect (link an issue/note) · ~ partial · – untested

| Area | Check | macOS (WKWebView) | Linux (WebKitGTK) | Notes |
|---|---|---|---|---|
| Shell | app launches, window sizing, min size | – | – | T000 |
| Shell | health round-trip under strict CSP | – | – | T000 |
| Input | keyboard shortcuts (Cmd/Ctrl combos) | – | – | T040 |
| Input | multi-cursor, selection, find/replace | – | – | T040 |
| Input | clipboard copy/cut/paste in editor and inputs | – | – | T040 |
| Input | IME composition, dead keys, international layouts | – | – | T040 |
| Input | drag and drop of text and files | – | – | T040 |
| Rendering | font rendering, high-DPI, zoom/scaling | – | – | T040 |
| Rendering | light/dark theme switch | – | – | T000+ |
| Files | Unicode filenames, spaces, symlinks, broken symlinks | – | – | T020/T030 |
| Files | case-insensitive vs case-sensitive path identity | – | – | T020 |
| Files | permission errors surfaced, not crashed | – | – | T030 |
| Files | external modification detection | – | – | T030/T070 |
| Terminal | interactive shell (vim, less, top) | – | – | T090 |
| Terminal | resize, process-tree kill, unexpected child death | – | – | T090 |
| Recovery | force-kill restart restores tabs/cursor/dirty buffers | – | – | T060 |
| System | sleep/wake, multi-monitor, window restore | – | – | T060 |
| Build | `pnpm tauri build` produces a launchable bundle | – | – (CI: no-bundle) | T000 |
