# ADR-020: A hook may wait for Gritty's verdict on a permission prompt, bounded and fail-open

- **Status:** Accepted (implemented by T335)
- **Date:** 2026-09-06
- **Amends:** ADR-018 §5 and its "does not open the door to control" consequence

## Context

ADR-018 gave Gritty the one value poll-only discovery cannot earn: `BLOCKED`,
the fact that an agent is stopped at a permission prompt. It did so through a
hook binary constrained to be inert — §5 "The hook never blocks or breaks the
agent", §6 "The hook knows nothing of the Gritty protocol", §7 "Evidence, not
authority" — and it closed with a consequence that reads as a wall:

> **This does not open the door to control.** A hook is a notification the agent
> emits, not a channel into it. Answering a permission prompt, cancelling a turn
> or injecting a keystroke are none of them consequences of this decision; they
> belong to runs Gritty owns (T300).

That wall was right for T221 and is the wrong shape for what the product is now
being asked to do. Design turn 12 draws the answer, not the notification: the
agent you are watching asks in place, the ones you are not ask through a queue.
Today Gritty can tell you `t090` is waiting and nothing else — you still walk to
another terminal to answer, which is the exact failure the island exists to
remove. A cockpit that can only report that a decision is needed, and never take
it, is a dashboard.

The wall also rested on an assumption worth re-examining. ADR-018 argued the
hook must not block because "Gritty is usually **not running** when a hook
fires", and a socket would "stall the agent while it timed out". Both are
statements about *unbounded* or *unconditional* waiting. Neither rules out a
wait that is bounded, conditional on Gritty already being alive, and discarded
safely when it overruns.

## What was measured

Everything below was executed against the Claude Code build in use here —
`2.1.263`, at `~/.local/share/claude/versions/2.1.263` — on 2026-09-06, in an
isolated project directory with the hook supplied through `--settings` so that
neither `~/.claude/settings.json` nor any real project was touched. Rule 12: none
of this is inferred from documentation.

**1. `PermissionRequest` fires exactly when a prompt would be shown, and carries
the subject.** A `Write` in `default` mode produced:

```json
{"session_id":"aa1ade04-…","transcript_path":"…jsonl",
 "cwd":"/private/tmp/gritty-manual-home/work","permission_mode":"default",
 "hook_event_name":"PermissionRequest","tool_name":"Write",
 "tool_input":{"file_path":"…/probe.txt","content":"hello\n"},
 "permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}]}
```

`session_id` is the key T221 already joins on. `tool_input` is the verb's
subject — the path and the content a diff needs, or the command for `Bash`.
`permission_suggestions` is the agent's own account of the scopes on offer,
which is where "Allow … for this run" comes from rather than being invented.

**2. A verdict on stdout is honoured.** Returning

```json
{"hookSpecificOutput":{"hookEventName":"PermissionRequest",
                       "decision":{"behavior":"allow"}}}
```

created the file with no keystroke, and the transcript recorded
`⎿ Allowed by PermissionRequest hook`. The deny form,
`{"behavior":"deny","message":"…"}`, refused the write and delivered `message`
to the model, which quoted it back and reasoned about it. Denial is therefore
feedback, not a dead end — Gritty's reason reaches the agent.

Note the shapes differ by event, and the binary's own error text is explicit
about it: `PreToolUse` takes `hookSpecificOutput.permissionDecision`
(`allow|deny|ask|defer`), `PermissionRequest` takes
`hookSpecificOutput.decision: {"behavior": …}`.

**3. Overrunning the hook's timeout is safe.** With `"timeout": 5` and a hook
that slept 25 s, Claude Code discarded the output — "hook timed out … output
discarded. Raise the hook's `timeout` to allow more time" — left the prompt
live, and the human answered it normally. The per-hook `timeout` is honoured
verbatim in seconds with no clamp, so the ceiling is ours to choose.

**4. The prompt is displayed while the hook deliberates.** The TUI rendered
"Do you want to create probe.txt?" 6.4 s before a hook that answered at 20 s.
Both answer paths are live at once and the first one wins.

## Decision

`gritty-agent-hook` may, **for `PermissionRequest` alone**, wait for a verdict
Gritty writes, and return it on stdout. Every other event keeps ADR-018 §5
exactly: write one file, exit 0, say nothing.

The wait is guarded by a chain whose every link fails open:

1. Gritty, while running, refreshes a liveness marker on the fast loop it
   already runs at 400 ms.
2. The hook writes its state file first, exactly as today — so `BLOCKED` is
   earned whether or not anyone is listening — then stats the marker. Missing or
   stale ⇒ return immediately, byte for byte ADR-018's behaviour. **This is
   ADR-018 §6's argument preserved: Gritty is usually not running, and the hook
   still does not care.**
3. Marker fresh ⇒ poll for a verdict file, re-arming only while the marker stays
   fresh, so killing Gritty releases the agent within one marker interval rather
   than at the deadline.
4. Verdict found ⇒ emit it. Timeout, unreadable file, malformed JSON, or any
   error ⇒ emit nothing and exit 0, and the human answers in the terminal as
   measurement 3 showed they can.

A file, still, and not a socket: ADR-018 §6's reasoning survives intact, and a
poll of a directory Gritty owns needs nothing listening and has no connect
failure to reason about.

## Consequences

- **ADR-018 §5 is narrowed, not deleted.** "The hook never blocks" becomes "the
  hook never blocks *unless Gritty is alive and the agent was about to block on
  a human anyway*". The wait replaces a wait; it does not add one. An agent is
  no slower for having Gritty installed in any case where it was going to run
  unattended.
- **The installed hook entry gains a `timeout`.** Answering takes as long as a
  person takes, so the `PermissionRequest` entry is installed with a generous
  one. That value is the true ceiling on how long an agent can be held, and it
  is visible in the user's own settings file rather than buried in a binary.
- **Two answer paths are live at once, and the first wins.** The user may answer
  in the terminal while Gritty is asking, or the reverse. Gritty must therefore
  treat a prompt that vanishes without its verdict as normal, not as an error,
  and must never resurrect an ask whose session has moved on.
- **`tool_input` becomes a new and sharper security surface** (hard rule 13).
  It is the file content, or the command. It is bounded on write and again on
  read, never logged, and omitted entirely for targets whose content is
  presumed sensitive. A permission prompt that leaked the secret it was asking
  about would be worse than no prompt.
- **A verdict is authority, and it is the user's.** §7 said hook-sourced state is
  evidence, never authority, and that still holds for everything the hook
  reports. What travels the other way is not a derivation Gritty made; it is a
  decision a person took, in a UI that named the verb, the target and the blast
  radius. The core never synthesises one.
- **This still does not open the door to control.** Answering a question the
  agent asked is not driving the agent. Cancelling a turn, injecting a
  keystroke, or writing to stdin remain out, and remain T300's.
- **Enforcement is still not this.** A denied ask is a decision the agent
  honours because it asked. Stopping a write nobody asked about is T330, and it
  now has the mechanism it was waiting for.
- **One mechanism, several providers.** The event name and the reply shape are
  provider-specific and live in the adapter; the verdict file, the marker and
  the bounded request are not. Codex is a parse and a set of names (T222).

## Alternatives rejected

**Answer at `PreToolUse` instead.** It also accepts a decision, and it fires
for *every* tool call — including the overwhelming majority Claude Code would
have allowed without asking anyone. Gritty would have to re-derive which calls
deserved a question, duplicating a policy the agent already applies, and would
pay a hook round trip on every tool use. `PermissionRequest` fires precisely
when a human was about to be asked, which is precisely when Gritty wants to be.

**Wait unconditionally, without the liveness marker.** Simpler, and it hangs
every agent on a machine where Gritty is not running — which ADR-018 correctly
identified as the common case.

**A socket.** Rejected by ADR-018 §6 for reasons this decision does not change:
it requires Gritty to be running and puts failure handling inside a binary that
must not have opinions.

**Wait only until some short fixed deadline, then deny.** Denying on a timeout
turns Gritty's absence into a refusal the user never made, and trains people to
distrust it. Falling through to the agent's own prompt is the only answer that
is honest about who decided.

**Do nothing, and wait for T300.** Correct by the letter of ADR-018, and it
leaves the product's central question answerable only by walking to another
terminal — for sessions Gritty already knows about, already reports on, and
already tells you are waiting.
