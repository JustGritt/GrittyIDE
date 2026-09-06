# ADR-022: Search runs in-process on ripgrep's own crates, and "ignored" keeps its current, narrower meaning

- **Status:** Proposed
- **Date:** 2026-09-06

## Context

`docs/Gritty IDE Project Plan.md:799` leaves the search implementation open:
"ripgrep process, Rust library, or hybrid". Nothing searches today. T080
specifies the command (`search.query`, streamed, cancellable, no index) and
recommends the in-process option, then says the choice "needs its own ADR
before any crate code is written" (`docs/tasks/T080-search.md:126`).

The three candidates are not equally weighted by this repository's priority
order (`correctness → recoverability → inspectability → responsiveness`), and
two of the hard rules decide most of it before performance is discussed at all.

**Spawning `rg`.** Hard rule 10 requires long-running core work to be
cancellable through the operation model. A spawned process cancels by dying,
and the only process-tree termination logic in this tree is ADR-015's
PTY-shaped signal sequence, built for a shell and its children — there is no
bare process-supervisor crate to reuse, so search would either grow a second
kill path or misuse the terminal's. Hard rule 4 requires raw external output to
pass through an adapter before it becomes domain state: `rg --json` is exactly
the raw provider output that needs translating, and getting a match position
wrong is silent. Hard rule 11 forbids claiming cross-platform support from
compilation alone; a spawned binary has to be located, bundled or found on
`PATH`, on a project that is one accepted OS deep (ADR-002) and has no Linux CI
until October. Three rules, three costs, and the only thing bought is a
process boundary nothing here asked for.

**`git grep` or any other external tool.** Same objections, plus it would only
search tracked files, which is not what an editor's find-in-files means.

**In-process, on the crates ripgrep itself is built from.** `grep` 0.4.1 is a
facade over `grep-searcher` 0.1.17, `grep-matcher`, `grep-regex` 0.1.14 and
`grep-printer`; `ignore` 0.4.33 is the parallel directory walker with
gitignore semantics. All are `Unlicense OR MIT`, and `ignore`'s MSRV (1.88) is
under this workspace's pinned 1.98.0 toolchain. Cancellation is free: a
`Searcher` drives a `Sink` we own, so `OperationContext`'s token is checked in
our code between files and inside one, the way `list_directory` already takes a
`cancelled: &dyn Fn() -> bool` closure. There is no output to translate, because
there is no output — the sink *is* the adapter.

### The part neither spec noticed

**Gritty has no gitignore support, and `ignore`'s main feature is gitignore.**
What the explorer calls "ignored" today is a hardcoded list of eleven directory
names — `.git`, `node_modules`, `target`, `dist`, … —
(`crates/workspace/src/listing.rs:27-39`), surfaced as `ignoredHint`. T070 keeps
that shape, proposing `is_ignored(path)` beside the same constant
(`docs/tasks/T070-file-watcher.md:146-148`), and T080 plans to call it
(`docs/tasks/T080-search.md:152`). None of the three reads a `.gitignore` file.

So adopting `ignore` for its walker quietly offers a second, different and much
larger definition of the word — nested ignore files, negation, `.git/info/exclude`,
`core.excludesFile`, and a repository's real rules — across three surfaces that
currently agree with each other. Taking it by accident, in the crate that
happens to need a walker first, is how a product ends up telling the user two
things.

## Decision

**1. Search is in-process, built on `grep-searcher` + `grep-regex` + `ignore`.**
No `rg` process, now or as a fallback. Each crate is one exact-pinned
`[workspace.dependencies]` entry with the dependency-policy justification; they
overlap nothing in the tree.

**2. `ignore` is taken for the walk only, with its gitignore behaviour turned
off.** The walker is configured to consult no ignore file at all
(`git_ignore`, `git_global`, `git_exclude`, `ignore`, `parents`, `hidden` — all
off), and filtering stays T070's `is_ignored`. What Gritty means by "ignored" is
unchanged by this ADR, and stays one thing in three surfaces.

**3. Real gitignore support is a separate, later decision** — one that changes
the explorer, the watcher and search together, and that has to answer what
`ignoredHint` means afterwards. It is not a side effect of choosing a search
engine, and this ADR deliberately does not make it.

**4. The `grep` facade crate is not taken.** It re-exports `grep-cli` and
`grep-printer`, which exist to build a terminal program and are dead weight
here. Depending on the two crates actually used keeps the surface honest.

## Consequences

- Cancellation, progress and errors all arrive through the existing operation
  model, and search needs no event family of its own beyond T080's
  `SearchMatchesFound`. Nothing new can leak a process.
- The pre-1.0 versions are the standing risk: `grep-searcher` is 0.1.17 and
  these crates are versioned for ripgrep's benefit, not for external consumers.
  A minor bump may break compilation. It cannot break behaviour silently — the
  sink is ours and the tests are ours — so the failure mode is a build error
  during a deliberate upgrade, which is the acceptable kind.
- Binary detection stays Gritty's own (`security.md:71-79`: invalid UTF-8, or a
  NUL byte), not `grep-searcher`'s. Two definitions of "binary" in one product
  would be the same mistake as two definitions of "ignored", and T030 got there
  first.
- Search will walk a generated directory whose name is not one of the eleven —
  `.next`, `vendor`, `coverage` — and will not honour a `.gitignore` the user
  wrote. That is a real limitation and users will meet it. It is the explorer's
  behaviour today, so search is consistent with the rest of the product rather
  than worse than it, which is the whole point of decision 2.
- No search-and-replace, no index, no cross-workspace search: unchanged by this
  ADR, all still T080 non-goals.

## Revisit when

Gritty needs real gitignore semantics anywhere — the explorer, the watcher, or a
search that a user expects to match `rg`'s results — at which point the decision
is about the *meaning of ignored* across all three, and `ignore` is already in
the tree to implement it. Or when a measured walk over `fixtures/large-repo`
(10,500 files) is too slow with the token checked at
`CANCEL_CHECK_INTERVAL`, which would argue for `ignore`'s parallel walker and
its own cancellation, not for a process.
