# Task T222 — More providers: Codex first, then what can be verified

- **Phase:** 3
- **Status:** not yet started
- **Depends on:** T220 (agent session discovery), T221 (hook bridge) for the integration half

## Objective

T220 shipped one provider and wrote the seam for more: *"One variant on
`Provider`, one arm in `provider_of_command`, and a transcript reader for that
agent's format."* This task takes that seam and adds **Codex**, which is fully
characterised below, and then as many further providers as can be verified
against a real installation.

The comparison that motivates it: Herdr detects 15–21 agent CLIs. Gritty
detects one. The roster's value grows with coverage, and the cost per provider
is deliberately small — but only for providers whose format someone has
actually looked at.

## Dependencies

- **T220** for `gritty-agents`, the merge, `AgentProvider`,
  `provider_of_command`, the bounded transcript read and the ranking.
- **T221** for the hook installer, if this task also lands Codex's integration.
  It can land discovery-only without T221 and gain the integration later.

## Required reading

- `docs/architecture/agent-sessions.md` — "Adding a provider"
- `crates/agents/src/transcript.rs` — the bounded tail-read to generalise
- `crates/agents/src/probe.rs` — `provider_of_command` and why only the
  executable name may classify a process
- `crates/gritty-core/src/agent_session/wire.rs` — `AgentProvider`
- `docs/decisions/ADR-018-agent-integration-bridge.md` — if adding hooks

## Allowed scope

- `crates/agents/**`
- `crates/gritty-core/src/agent_session/**` (the `AgentProvider` variant only)
- `crates/gritty-core/src/protocol/{mod.rs,export.rs}` — version bump
- `packages/protocol/src/{types.ts}` + tests
- `apps/desktop/src/features/agents/**` — provider labels only
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No provider trait.** T220 chose a match arm over an abstraction with one
  implementation, on the grounds that one arm is cheaper to read. Two or three
  arms do not change that. Introduce a trait only if a fourth provider makes
  the duplication real, and say so in the completion report rather than doing it
  pre-emptively (hard rule 9).
- **No provider whose format was not read on a real installation.** See
  "Required behavior" §4. A guessed path that silently matches nothing is worse
  than an absent provider, because it looks like coverage.
- **No control, no orchestration, no run lifecycle.** Same boundary as T220.
- **No transcript viewer.** Bounded previews only.
- **No new dependencies.** Every format below is JSONL; `serde_json` is already
  in the tree.

## Required behavior

### Codex — fully specified

Verified against a real installation (macOS, `codex` 0.145.0) while this task
was written.

1. **Sessions live at**
   `~/.codex/sessions/YYYY/MM/DD/rollout-<ISO-timestamp>-<uuid>.jsonl`.
   The date-partitioned layout means the age cutoff can be applied to
   *directories* before any file is opened — cheaper than T220's flat scan.

2. **The tail-only read does not work here, and this is the load-bearing
   detail of the task.** Codex writes a `session_meta` record carrying
   `session_id`, `cwd`, `originator`, `cli_version` and `model_provider` as the
   **first line of the file**, not the last. Claude Code repeats its identity
   on nearly every record, which is what made T220's tail sufficient.

   So the reader needs a bounded **head** read *and* the existing bounded tail:

   ```
   head 8 KiB   → session_meta: session_id, cwd, model, cli_version
   tail 128 KiB → event_msg / response_item: activity, last prompt, last reply
   ```

   Both bounds are constant, so the O(1)-in-file-size property T220 bought is
   preserved. **This must be tested against a large file**; losing that property
   is the main regression risk in this task.

   The `session_meta` payload also embeds the full base instructions — several
   kilobytes of prompt — so the head read must be bounded generously enough to
   reach the fields but must never assume the record fits, and must tolerate a
   first line longer than the head window by falling back to a streamed scan of
   just that line's leading fields.

3. **`~/.codex/session_index.jsonl`** carries `{id, thread_name, updated_at}`
   per session — a cheap equivalent of Claude Code's `ai-title`. Read it once
   per scan to get the task line for every session without opening a rollout
   file at all, and treat a session id absent from it as merely untitled.
   Note that the index can hold **several rows for one id** (observed:
   two `thread_name`s for the same session). Last write wins by `updated_at`.

4. **Tail record types are `event_msg` and `response_item`**, not Claude Code's
   `type: "assistant" / "user"` shape, so the outstanding-tool-call fold is its
   own function. Derive the same four things — task, last prompt, tool in
   flight, last reply — or leave them absent. An absent field is correct; an
   invented one is not.

5. **Hooks are configured in `~/.codex/hooks.json`**, with the same event names
   and the same `{hooks: {Event: [{matcher?, hooks: [{type, command, timeout}]}]}}`
   shape as Claude Code's `settings.json`. T221's installer therefore
   generalises with a per-provider config path and event set, and no new
   mechanism. Verify the shape before relying on it; a separate file from
   `config.toml` is what makes this cheap, since no TOML editing is needed.

6. **Process classification** follows T220's rule exactly: only the executable
   name (`codex`) may classify a process, never a substring of the command line.

### Other providers — verify, then add

7. Add any further provider **only after reading its session format on a
   machine that has it installed**. At the time of writing, neither Cursor's
   agent CLI nor Gemini CLI was installed on the development machine, and
   `~/.cursor/agents` was empty — so no path, format or record shape for either
   is asserted anywhere in this task, and none may be copied from a blog post.

8. For each candidate, the completion report records: the session directory,
   the record shape, whether identity is at the head or repeated, whether a
   session index exists, the hook config path and event names, and the
   executable name. A candidate that cannot be verified is listed as
   **not added, with the reason** — which is a complete outcome for this task,
   not a failure of it.

### Shared

9. Provider display names and any provider-specific parsing stay inside
   `gritty-agents`. `AgentProvider` on the wire gains variants and nothing else;
   no provider concept may reach a `Workstream` (hard rule 6).
10. Session ids stay unique across providers, as `AgentSession.id` already
    promises. Namespace by provider if two formats could ever collide.
11. Protocol version bumps MINOR: adding enum variants is additive. Confirm the
    TypeScript side treats an unknown `AgentProvider` as displayable rather than
    as a branch it must exhaust — the wire enum's doc comment already states
    this intent for older clients.
12. Ranking, the roster diff and the poll budget are unchanged. Scanning three
    providers must not triple the scan cost: the per-provider directory walk is
    cheap, and the file cap is **global**, not per provider.

## Failure behavior

| Situation | Behavior |
|---|---|
| A provider's directory is absent | That provider contributes nothing; no error. The normal case for most users |
| Codex `session_meta` missing, unparseable, or beyond the head window | The session is skipped rather than listed under a guessed cwd — T220's rule for a transcript with no recoverable id |
| `session_index.jsonl` absent or malformed | Sessions list untitled; the rollout files still parse |
| Duplicate ids in `session_index.jsonl` | Last write wins by `updated_at` |
| A rollout file being appended to while read | The final partial line is skipped, as T220 already does |
| Non-UTF-8 or truncated file | Skipped; every other session still lists |
| A provider's format changes | Fields go absent, the session still lists. Degradation, never a hard failure |
| Many providers installed at once | The global file cap and age cutoff hold the scan inside its budget; providers are not each given their own quota |

## Acceptance criteria

- With a Codex session running, the roster lists it with the correct working
  directory, task line, model and activity, alongside any Claude Code sessions,
  ranked by the same rules.
- A 400 MB Codex rollout file is read in the same bounded time as a small one,
  with no memory spike — the T220 property, re-verified for head + tail.
- With Codex installed but never run, the roster is empty and no error is
  raised.
- Scan duration with two providers stays within the same order as T220's
  measured ~300 ms; the figure measured is recorded in the completion report.
- Every provider claimed in the report was exercised against a real session.
- `pnpm check` green; generated protocol types fresh and committed.

## Tests required

- **Unit (Rust):** Codex head parse — `session_meta` present, absent,
  oversized, malformed, beyond the head window.
- **Unit (Rust):** Codex tail fold — task, last prompt, last reply, outstanding
  tool call, tool call closed by its result, malformed line skipped, truncated
  final line tolerated.
- **Unit (Rust):** `session_index.jsonl` — present, absent, malformed,
  duplicate ids resolved by `updated_at`.
- **Unit (Rust):** bounded cost — a synthetic large fixture proves head + tail
  reads a constant number of bytes.
- **Unit (Rust):** `provider_of_command` classifies `codex` and does not
  classify a shell whose command line merely mentions it.
- **Unit (Rust):** the merge and ranking with sessions from two providers,
  including id uniqueness across them.
- **Unit (TS):** an unknown `AgentProvider` renders rather than throwing.
- **Manual:** run Codex and Claude Code at once, watch both appear.

## Commands to run

```
cargo test -p gritty-agents
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_(filled in by the implementing session — including, per §8, the verified
format notes for every provider considered, and the reason for each one not
added)_
