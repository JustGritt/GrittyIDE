# Security model

Status: baseline written at T000; refined by T020 (paths), T090 (processes),
T300 (agent capabilities).

## Principles

- Repository content and agent output are untrusted input.
- The WebView gets a minimal command surface. No arbitrary filesystem, shell,
  process or path APIs are exposed; Tauri capabilities stay at the core
  app/event/window/webview defaults (`apps/desktop/src-tauri/capabilities/default.json`).
- Every path crossing into the core is canonicalized and validated against the
  allowed workspace roots. Traversal (`..`), symlink escapes and paths outside a
  root are rejected unless a capability explicitly allows `workspace.outsideRoot`.
- Command and process execution for agents runs under explicit allowlists or
  capability grants; destructive or out-of-scope actions are shown before
  execution unless a persistent permission was deliberately granted.
- Markdown preview never renders raw HTML from documents or agents; output is
  sanitized and links to `javascript:`/`data:` URLs are stripped.
- Secrets stay out of logs, events, SQLite and persisted run output. Provider
  tokens go to OS credential storage when direct API credentials appear.
- Content Security Policy is enabled in both dev and release
  (`app.security.csp` / `devCsp` in `tauri.conf.json`). Widen a directive only
  for a concrete need (Monaco workers, Markdown images) and record why.

## Process boundaries

Agents, language servers, terminals, indexers and external tools run outside the
renderer as supervised processes. Their failure must not freeze or crash the UI.
Process trees are terminated predictably on cancel and on application exit.

## Actor attribution

All mutations carry an actor (`USER`, `SYSTEM`, `AGENT(runId)`, `EXTERNAL`) so
the user can always see who changed what. See `overview.md`.
