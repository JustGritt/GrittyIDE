# Recorded `claude` streams

`two-turns-denied.jsonl` is a **real** capture, not a hand-written stream. It is
what makes `tests/fixture.rs` an assertion about the CLI rather than about its
author's memory, and it is why CI can prove the translation with neither
`claude` on `PATH` nor an API key.

## Provenance

| | |
|---|---|
| Binary | `claude` 2.1.263 |
| Platform | darwin 25.6 (macOS, arm64) |
| Recorded | 2026-09-08 |
| Model | `claude-haiku-4-5-20251001` (`--model haiku`) |
| Argv | exactly what `ClaudeLaunch::argv` builds, plus `--model haiku --tools Bash --permission-mode manual` |

The three extra flags are recording conveniences and are **not** part of the
shipped launch line: `--model haiku` kept the capture cheap, `--tools Bash`
guaranteed a gateable call, and `--permission-mode manual` was carried over from
the task's own channel table before it was isolated and found to contribute
nothing (see `src/launch.rs`).

## What it contains

Two turns in one process:

1. the user asks for `touch probe-side-effect.txt`; the agent issues a `Bash`
   `tool_use`; a `control_request`/`can_use_tool` arrives; the driver answers
   `{"behavior":"deny"}`; the refusal comes back as a `tool_result` with
   `is_error: true`; the agent explains and the turn ends `success`;
2. the user asks what command was attempted, and the answer names it — which is
   the evidence that one process really does carry a conversation.

`probe-side-effect.txt` did **not** exist when the ask arrived, and did not
exist after the process exited. The tool never ran.

It also carries four line kinds this crate deliberately does not model —
`system/hook_started`, `system/hook_response`, `system/thinking_tokens` and
`rate_limit_event` — which is what keeps "an unrecognised type is recorded and
skipped" honest against a real release rather than against an invented one.

## What was redacted

The capture ran in a scratch directory on a developer's machine, so it was
rewritten before being committed. Only identifying values changed; no line was
added, removed or reordered, and no field this crate reads was altered except
the paths inside it.

- absolute paths → `/workspace/demo`
- every UUID except the session id → `00000000-0000-4000-8000-…`
- `toolu_…` ids → `toolu_0000…`
- the `init` line's `tools`, `mcp_servers`, `slash_commands`,
  `terminal_slash_commands`, `agents`, `skills`, `memory_paths` and
  `messaging_socket_path` → trimmed to the same shape with this machine's
  inventory removed
- the hook output, which named an unrelated third-party tool → a placeholder
- `thinking.signature` → `REDACTED`
- `total_cost_usd` → `0.0`, `modelUsage` dropped, `rate_limit_info` reduced to
  its shape

`tests/fixture.rs::the_fixture_carries_nothing_from_the_machine_that_recorded_it`
asserts the redaction held, so a future re-recording cannot quietly leak a home
directory or a key.

## Re-recording

Drive `claude` with the argv above from a scratch directory, answering the
`control_request` on stdin, then apply the redactions above. Update the
assertions in `tests/fixture.rs` to whatever the new session actually did —
they name a specific conversation on purpose, and a fixture whose test was
loosened to accept anything would prove nothing.
