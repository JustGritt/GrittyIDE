# ADR-017: Command blocks are a renderer-side view of xterm's scrollback

- **Status:** Proposed
- **Date:** 2026-09-05

## Context

Design artboard 8a asks the terminal to group its output per command, with the
command, its duration and its exit code on one line and the output under it. A
byte stream does not say where a command starts, so this needs semantic prompt
markers — OSC 133 `A`/`B`/`C`/`D;<exit>`, the FinalTerm sequences iTerm2, VS Code
and WezTerm all implement — emitted by hooks Gritty installs into the shell it
starts.

Two questions come with that, and only the second is genuinely open.

The first is **how the markers get emitted**, and it has one answer: point the
shell at a script through the environment of the session Gritty spawns
(`ZDOTDIR` for zsh, `--rcfile` for bash), never by editing the user's dotfiles.
That is T097's scope and needs no ADR.

The second is **who owns the resulting command history**. AGENTS.md hard rule 4
says raw provider output is never Gritty domain state; adapters translate it.
Read strictly, that argues for a `TerminalCommand` registry in
`gritty_core::terminal`, emitting `TerminalCommandStarted`/`Finished` events the
way T047 put session *names* in the core rather than in the renderer.

## Decision

**A command block is a renderer-side view of xterm's scrollback.** The core
builds the environment that makes the markers appear and is otherwise unchanged:
it gains no command registry, no new events, and `TerminalOutput` keeps its
shape.

Three reasons, in order of weight.

1. **A block is a range of rows, and the rows only exist in xterm.** ADR-015
   already decided that scrollback is not core state and is never persisted —
   terminal content is user data that routinely contains secrets. A core-side
   registry would own the semantics of a block while owning none of its extent,
   and the renderer would still have to find the rows itself.

2. **The join between the two would be the failure.** Output reaches the
   renderer as coalesced ~60 Hz batches from a 1 MiB drop-oldest buffer. To
   place a core-side command against a row, the renderer has to know *where* in
   the byte stream the marker fell — which means either splitting `TerminalOutput`
   at marker boundaries (a shape change on the hottest event in the protocol) or
   an ordinal join that silently drifts the first time a view mounts late and
   the bounded pre-mount buffer has already dropped a marker. Correctness is the
   first priority in this repository; a correlation that fails quietly is the
   wrong trade for a decoration.

3. **Hard rule 4 is about not letting an agent's output become the domain.**
   Nothing downstream consumes command blocks. When T300 needs to know what an
   agent ran, it gets that from the agent adapter, which is a contract, not from
   parsing a shell's escape sequences.

The markers reach the renderer because they already do: xterm ignores an OSC it
has no handler for, so `parser.registerOscHandler(133, …)` sees them exactly
where they occurred, and `registerMarker()` pins the row.

## Consequences

- Blocks do not survive a renderer reload. Neither does the scrollback they
  describe, so nothing is lost that was previously kept.
- 8b's session-rail subtitle (`cargo test · exit 101`) is available, because the
  panel holds the blocks; it is renderer state, and a second window would not
  see it. There is one window.
- T047's deferred automatic titles (a tab naming itself after what it runs)
  become possible, but they would be *core* state set from the renderer through
  `terminal.rename` — the renderer observing and the core remembering, which is
  the same split T047 already chose.
- If a later task genuinely needs command history in the core — a run log, an
  agent that reports what it executed — this is superseded rather than extended.
  The marker parsing would move to the core and `TerminalOutput` would carry the
  boundary. Recording that now would be the speculative layer hard rule 9 warns
  against.
- Injecting into shell startup is the real risk, and it is a T097 risk, not this
  one: a configured prompt (powerlevel10k's instant prompt in particular) is the
  surface where T095 already found a defect.

## Revisit when

Something outside the renderer needs to know what a terminal ran — a run log,
persistence, an agent adapter that reports commands — or when a second window
exists.
