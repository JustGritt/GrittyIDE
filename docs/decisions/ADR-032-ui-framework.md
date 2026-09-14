# ADR-032: The UI framework is React 19, and it renders chrome rather than content

- **Status:** Accepted
- **Date:** 2026-09-09
- **Accepted:** 2026-09-09 by Alexis, with the amendments below: React is
  recorded as the *current shell* rather than a permanent commitment,
  *Revisit when* is restated as explicit reopening criteria, and the
  measurement those criteria turn on is commissioned rather than only named.
- **Amends:** ADR-001, which asserted "a React + Monaco frontend" while its
  Context compared only Electron against Tauri. The runtime half of that
  sentence is argued there and still open; the frontend half was never argued
  at all, and is settled here.

## Context

This ADR exists because of a question that turned out to rest on a false
premise: *were we on React because that is what Monaco is built on?*

Monaco is not built on React. `monaco-editor@0.56.0` declares exactly two
dependencies, `marked` and `dompurify`, and no peer dependencies at all;
`@xterm/xterm@6.0.0` declares none. Both own their DOM and render themselves,
and VS Code — Monaco's parent — is not a React application either. React was
never buying editor compatibility, and dropping it would never have meant
fighting the editor.

The premise was believable because nothing in this repository contradicted it.
**React is named dozens of times across `docs/` as a given and never once as a
decision.** ADR-001 states "Use Tauri 2 with a Rust core and a React + Monaco
frontend" (`ADR-001-desktop-runtime.md:16-19`) inside a document whose Context
weighs Electron against Tauri and whose kill criteria are scoped to Monaco,
WebView and PTY behaviour — none of them to React. The Project Plan states it
just as flatly (`Gritty IDE Project Plan.md:24-26`), as does T000
(`docs/tasks/completed/T000-bootstrap-workspace.md:9-10`). No alternative UI framework —
Solid, Svelte, Preact, Lit, vanilla TypeScript, web components — is named
anywhere in the documentation tree, in comparison, in a rejected-alternatives
list, or in passing. React entered as an unexamined assumption on 2026-09-02 and
has been inherited ever since.

So two things needed settling: whether there was a reason, and whether the
assumption is costing measurable performance. The second is answerable from
evidence already in the tree.

### What React actually costs here

**Payload: 3.6%.** React 19.2.8's production build is ~540 KB raw / 97 KB
gzipped, against a 14.9 MB `dist`. The TypeScript language worker alone is
6.6 MB. The 4.6 MB main chunk is dominated by Monaco — 558 occurrences of
`monaco`, plus 87 language chunks — because `packages/editor` takes the barrel
import `import * as monaco from 'monaco-editor'`
(`packages/editor/src/monaco/editor-host.ts:2`), which pulls every language
contribution and editor feature. That single line is worth an order of magnitude
more payload than the entire framework. Payload is in any case close to free
here: a desktop app loads from local disk, not over a network.

**Hot paths: none.** Every surface in Gritty that updates at high frequency has
already been routed around React deliberately, and each one records why in its
own comment:

1. **Terminal bytes never enter React state.** `use-terminals.ts:53-59`
   intercepts `TerminalOutput` before the reducer and hands it to a plain
   `Map`-based pub/sub (`output-bus.ts:47-70`); `terminal-view.tsx:91-99`
   subscribes and calls `terminal.write(chunk.data)` on the xterm instance. The
   file says it outright: putting that through `useState` would re-render the
   tree at the same rate. Scrollback is xterm's own, virtualized inside its
   renderer; React never draws a line of it.
2. **Monaco lives in a detached surface that React only moves.** One editor is
   created once (`use-editor-binding.ts:124-128`) and `appendChild`-ed into
   whichever container is on screen (`:175`), so models, undo history and view
   state survive tab switches and pane remounts. Text enters only when
   `bufferGeneration` changes — that is, from confirmed core state — and
   keystrokes leave through an imperative read, never through a React-controlled
   value.
3. **The Markdown preview is an `innerHTML` island** (`markdown-preview.tsx:111`),
   with link handling by native event delegation rather than React `onClick`.

**Keystrokes do not re-render.** `documents-reducer.ts:286-295` returns the same
state object for keystrokes 2..N of a burst: once `tab.pending` is true the
update is identity-stable and the reducer bails out. This "compute the update; if
nothing changed, return the input object" idiom is house style across every
reducer in the app, not a local optimisation.

**Backpressure is solved in Rust, upstream of the WebView.** Hard rule 8 is
enforced at roughly twenty sites in the core with tests behind them;
`crates/workspace/tests/watch.rs:235` asserts that 400 file writes across 40
directories reach the renderer as fewer than 40 batches, with a cap of 8
directories per batch and an overflow flag instead of a partial list. What
crosses the IPC boundary is a converged summary. The decision that determines
whether a storm can overwhelm the UI was made in the core, and a change of
framework would not touch any of it.

**And the shell is measurably light already.** Phase 0 §C, measured on the
primary Mac against a release bundle (`docs/plans/phase-0-spike.md:63-74`): cold
launch 496 ms against a 2 s budget, warm launch 504 ms, idle CPU 0.064% of one
core, `documents.open` 0.23 ms median, `workspace.list` 0.29 ms median over a
10,501-file fixture, 607 MiB RSS against Cursor's 1194 MiB on the same
workspace — "Gritty at 51% of Cursor's memory and 29% of its idle CPU"
(`:86`).

### What is not measured

One metric could implicate React, and it is the one that has never been taken.
Typing responsiveness — frames over 16 ms during sustained editing — is recorded
as "pending the human pass" (`docs/plans/phase-0-spike.md:68`), has a method but
no number (`docs/testing/phase-0-manual-pass.md:241`), and is already flagged
stale because it would be measured against a UI that has since been rebuilt
(`docs/tasks/completed/T048-ui-revamp.md:169`). This ADR is decided without it, and names
it as the trigger that would reopen the question.

Two further facts belong on the record rather than in a footnote. There is no
`React.memo` and no list virtualization anywhere in the tree: what keeps
`rows.map(...)` cheap today is ring caps (`MAX_TIMELINE_ENTRIES = 500`,
`MAX_RUN_EVENTS = 512`) and server-side pagination in the explorer, not bounded
render cost. And `use-runs.ts:68-71` subscribes to *every* event of *every* kind
and dispatches each into `runsReducer`. Neither is a framework problem — both
would be identical under Solid or Svelte — but both are the places where growth
would first hurt.

## Decision

**1. React 19 stays.** The measured cost is 3.6% of a payload that is never
downloaded, on none of the hot paths, in a shell already running at half of a
comparable Electron editor's memory. No alternative framework addresses a cost
this application is actually paying.

**2. React renders chrome, not content.** The three imperative islands above are
not workarounds; they are the pattern. Any new surface whose updates arrive
faster than a user can read them — streamed search results, a live diff, a log
tail — takes the same route: the bytes go to a DOM node or a self-rendering
engine through a ref, and React state carries only lifecycle and identity. A
surface that cannot be built that way is evidence for *Revisit when*, not a
reason to widen React's job.

**3. The alternatives, weighed and rejected on the record**, so that this is a
decision rather than an inheritance:

- **Preact** (`preact/compat`) is the only option with a plausibly cheap
  migration, and it saves the cost that matters least: roughly 35 KB of a
  14.9 MB bundle. Its reconciler is not meaningfully faster than React 19's, and
  compat aliasing carries real risk against the React 19 semantics the app leans
  on — notably the `useReducer` identity bail-out that decision 2 depends on.
  Rejected: it buys bundle size and spends compatibility.
- **Solid** offers a genuine architectural win — fine-grained reactivity, no
  VDOM diff — but it optimises exactly the traffic Gritty already keeps out of
  the framework. Its JSX also looks far more portable than it is: props are
  getters, destructuring breaks reactivity, and there are no hook rules to port
  mechanically. Rejected: a rewrite of the React-coupled surface to speed up a
  path that carries almost nothing.
- **Svelte 5** is the same trade with additional tooling churn: a new file
  format changes the Vitest, Testing Library and Biome setup that
  `pnpm check` is built on. Rejected for the same reason as Solid, plus cost.
  **SvelteKit is rejected separately, and more firmly than Svelte itself.** It
  is a meta-framework whose value is SSR, filesystem routing, server endpoints
  and form actions; a Tauri application runs it under `adapter-static` with
  `ssr: false`, which is adopting a server framework in order to switch its
  server off. Gritty also has nothing for the routing half to describe — there
  is no router in the tree, and an IDE's state is panes, tabs and workstreams,
  not URLs. If this decision reopens it reopens on **Svelte 5 + Vite**, which
  is the only form of the candidate worth weighing.
- **Vanilla TypeScript over the DOM** is what VS Code does, and it is the honest
  answer to "what if we were starting over" — the highest ceiling and the fullest
  control, at the price of rebuilding a component and lifecycle model. Gritty is
  not starting over. Rejected on cost, not on merit.
- **Lit / web components** is actively the worst fit here: shadow DOM
  encapsulation works against ADR-016, whose tokens are deliberately read back
  off the document root by Monaco's theme and xterm's `ITheme`. Rejected on
  architectural conflict.

## Consequences

- The question is closed with an argument and a number attached, so it does not
  have to be re-litigated from intuition. Where it reopens, it reopens on
  evidence.
- Decision 2 is a constraint on future work, and the cheapest kind: it is what
  the terminal, editor and preview already do.
- **The exit stays cheap, and that is not an accident.** `packages/protocol`,
  `packages/editor` and `packages/markdown` contain no React at all — the Monaco
  wrapper in `packages/editor/src/monaco/editor-host.ts` is plain imperative
  TypeScript over the DOM — and across the whole workspace only 59 of 486
  TS/TSX files import React, 13,579 lines against 28,769 that are
  framework-free. Every feature follows the same trio of a pure
  `<feature>-reducer.ts`, a thin `use-<feature>.ts`, and a component. That
  discipline is why a migration would be a UI-shell rewrite rather than an
  application rewrite, and it is worth keeping deliberately: framework-coupled
  code stays out of `packages/*`, and state logic stays in plain reducers.
- **The styling layer is already framework-portable, and deliberately plain.**
  ADR-016 puts the palette, the type scale and the geometry scale in
  `apps/desktop/src/styles/tokens.css` as CSS custom properties, consumed as
  `var(--…)`; there is no Tailwind, no PostCSS and no CSS-in-JS anywhere in the
  workspace. Tokens in CSS are what would actually survive a framework change,
  and they are load-bearing well beyond styling, because Monaco's theme and
  xterm's `ITheme` read the computed values back off the document root
  (ADR-016). A utility-class layer would add nothing to that portability and
  would work against those two surfaces, which need values they can read at
  runtime rather than classes they cannot. Keep the tokens in CSS.
- What this ADR does *not* claim: that rendering is fast. It claims that
  rendering is not where the cost is, on the evidence available, and that the
  measurement which would test that claim is named and still owed.

## The limit of this evidence

Decision 1 rests on a claim about *today's* traffic: the framework carries
almost nothing, because the three surfaces that move fastest were deliberately
routed around it. That claim has an expiry, and naming it belongs in the
decision rather than in whoever reopens it.

The application this ADR was written inside is an editor shell. The one it is
being built toward is an orchestration cockpit, in which workstreams, agent
statuses, run consoles, workspace ownership, decisions, task progress, command
streams, diffs, review states and an attention queue all update concurrently,
from many sources, with no user driving any of them. That is a materially more
reactive UI than an editor, and it is the load under which fine-grained
reactivity would first earn its cost. So *"Svelte optimises a path carrying
almost nothing"* is true on the evidence available and must not be quoted
forward as though it were durable.

Two things in the tree were where that pressure landed first, and both are
ordinary work rather than framework evidence: `RUNS-EVENT-FILTER` (`useRuns`
dispatched *every* domain event of *every* kind into `runsReducer`) and the
absence of any `React.memo` or list virtualization, held in check today only by
ring caps and server-side pagination. Neither is a framework problem — both
would be identical under Solid or Svelte — which is exactly why they have to be
fixed *before* any measurement here can be read as a framework signal rather
than as render pressure we chose.

*Corrected 2026-09-14, before this amendment was committed:* the first is fixed —
`RUNS-EVENT-FILTER` landed as `04ff0ca`, and `useRuns` now takes two typed
subscriptions, the pattern `use-agents.ts` already used. The second is still
true, and the 9 September text called it ticketed when it is not: virtualization
is recorded only as a known limitation of T210 (the workstream UI), and nothing
on the board owns it.

## Revisit when

React is the current shell, not a settled architectural commitment. The rule:

> **Keep React unless the framework itself is demonstrably contributing to
> Gritty's responsiveness problems, after the non-framework causes have been
> ruled out.**

Reopen on any of the following, in writing:

1. **Measured typing cost, attributed.** Phase 0 §C step 2.25 is taken against
   the current UI and records sustained editing outside frame budget, *and*
   profiling attributes a meaningful share of it to reconciliation rather than
   to Monaco, layout or the WebView. The attribution is the whole criterion:
   those three are the expected dominant terms and none of them is fixed by
   changing framework.
2. **Orchestration render pressure.** The workstream, agent and attention
   surfaces show measurable render pressure under realistic concurrent load —
   measured after `RUNS-EVENT-FILTER`, virtualization and rule-8 batching have
   each been applied, so that what remains is the framework and not a
   subscription we picked badly.
3. **Escalating workarounds.** Holding a surface inside budget starts to need
   increasingly elaborate machinery to route around React. Decision 2 is the
   cheap version of that pattern — a ref and a DOM node — and it stops being
   cheap when new surfaces need bespoke work instead.
4. **Containment lost.** Framework code reaches `packages/*`, or reducers stop
   being pure. The migration cost this ADR treats as bounded is bounded only by
   that discipline. It is measurable, so measure it: 2026-09-09, 70 of 557
   TS/TSX files import React, 16 860 React-coupled lines against 35 495
   framework-free — 32.2 %, against 32.1 % when this ADR was drafted the same
   day at 486 files. The tree grew about a quarter and the ratio did not move;
   that is the number to re-take, and a rising ratio is the signal.

**Criterion 1 is owed, not optional.** It is the one measurement that could
implicate React, it has never been taken, and until it exists both sides of
this question argue from priors. It belongs to the `Phase 0 tests` ticket
(`docs/plans/phase-0-spike.md` §C; manual pass step 2.25) and it needs a person
at the keyboard — the agent sessions cannot type into the window, which is why
that row has read "pending the human pass" since 2026-09-04. Step 2.25 has been
amended alongside this ADR to require the attribution criterion 1 turns on: as
written it asked only for frame counts, and a frame count cannot tell
reconciliation from layout.
