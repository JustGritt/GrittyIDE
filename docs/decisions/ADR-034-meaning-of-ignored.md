# ADR-034: "Ignored" means what Git means when the root is a repository, and it is one predicate every surface is handed

- **Status:** Accepted (not yet implemented; IGNORE-RULES, IGNORE-SEAM,
  IGNORE-WIRE and IGNORE-UI build it)
- **Date:** 2026-09-14
- **Accepted:** 2026-09-14 by Alexis, as drafted: option C. When the workspace
  root is a repository, "ignored" means untracked and matched by Git's exclude
  rules or the eleven names. One gix-backed `IgnoreRules` in `gritty-git` is
  handed by `gritty-core` to the listing, the watcher, search and quick-open.
  Nothing is hidden, and `ignoredHint` covers files. Until IGNORE-SEAM lands,
  the two disagreements under "What ADR-022 did not know" are still live.
- **Decides:** ADR-022 decision 3, the decision it deliberately left unmade
- **Motivated by:** the limitation T080 (`docs/tasks/completed/T080-search.md:499-501`),
  T130-R (`docs/tasks/completed/T130-R-path-ranking.md:324-326`) and T130-C
  (`docs/tasks/completed/T130-C-quick-open-contract.md:284-286`) each recorded
  and each deferred here

## Context

ADR-022 kept "ignored" narrow so that it would stay *one* thing across the
explorer, the watcher and search, and said real gitignore support would change
all three together. This ADR is that change. What follows was read from the
tree on 2026-09-14, not from ADR-022's 2026-09-06 description of it, and two of
the facts below contradict that description.

### What "ignored" is today, per surface

**The definition** is eleven names, `IGNORED_DIRECTORY_NAMES`
(`crates/workspace/src/listing.rs:27-39`), and `is_ignored(path)` is true when
*any* segment of a path is one of them (`listing.rs:50-53`).

| Surface | How it asks | What the user gets |
|---|---|---|
| Explorer listing | `ignored_hint = kind == Dir && name in list` (`listing.rs:341-343`) — by name, directories only, not through `is_ignored` | a text badge, `ignored`, beside the name (`apps/desktop/src/features/explorer/explorer-tree.tsx:858-860`, rendered as `.tree__badge` at `:539`/`:694`). Nothing is dimmed, deferred or hidden; security.md says "Nothing is filtered" (`docs/architecture/security.md:62-64`) |
| Watcher | `is_ignored` on each event's directory (`crates/workspace/src/watch.rs:501`) and on a newly created directory (`:370`); the Linux registration walk checks the *name* again (`:443-447`) | changes inside an ignored subtree never refresh the explorer, even when the user has expanded it |
| Search (`search.query`) | `is_ignored` injected as `IgnorePredicate` (`crates/search/src/query.rs:64-70`) into `build_walker`'s `filter_entry` (`crates/search/src/walk.rs:217-262`), every `ignore` gitignore source off (`walk.rs:228-233`) | skipped unless `includeIgnored` (`query.rs:84-87`, wire `crates/gritty-core/src/search/wire.rs:59-67`) |
| Quick-open (`workspace.findFiles`) | the same `build_walker` (`crates/search/src/rank.rs:410`) | skipped, **with no way to ask otherwise**: `FindFilesRequest` has no `includeIgnored` (`crates/gritty-core/src/workspace/find.rs:68-85`) |

### What ADR-022 did not know

**1. A second meaning of the word is already on the wire.** `git.status` has
its own `includeIgnored`, and there it means *what `.gitignore` excludes*
(`crates/gritty-core/src/git/wire.rs:44-48`): `gritty-git` asks `gix`'s
directory walk to emit ignored entries (`crates/git/src/status.rs`,
`read_with`'s `emit_ignored`), and `FileState::Ignored` is a domain value
(`crates/git/src/model.rs:96`). So the protocol carries two request fields with
one name and two definitions, and the Changes panel already applies real
gitignore rules while the explorer beside it applies eleven names. The outcome
ADR-022 decision 2 existed to prevent arrived through T100, not through search.

Concretely, in a repository today:

- A gitignored `.next/`, `coverage/` or `vendor/` is absent from the Changes
  panel, unbadged in the explorer, and searched.
- A **tracked** `dist/` or `build/` (a library that commits its build output)
  shows its modifications in the Changes panel, is badged `ignored` in the
  explorer, and is **invisible to search and quick-open** — the user can see an
  edited file in one panel that the other two say is not there.

**2. The three surfaces already disagree with each other.** `is_ignored`
matches any segment, including the last, and `filter_entry` applies it to
files as well as directories, so a *file* named `build`, `out`, `dist` or
`target` (a shell script, say) is dropped by search and quick-open — while the
explorer, which checks directories only, shows it without a badge. No tracked
file in this repository has one of those names today, which is why nothing has
noticed.

### What each engine offers

**`gix`, already compiled in.** `Cargo.toml`'s `status` feature pulls
`dirwalk`, which pulls `attributes` and `excludes`, so
`gix::Repository::excludes(index, overrides, source)` is available with no new
feature. It returns an `AttributeStack` whose `at_path(relative, mode)` yields a
platform with `is_excluded()`; the stack reads nested `.gitignore` files
lazily per directory, applies negation and anchoring as Git does, and adds
`$GIT_DIR/info/exclude` (`gix-ignore-0.22.1/src/search.rs:53-78`). It is the
engine `git.status` already uses, and it takes the **index**, which is what can
say whether a path is tracked.

**`ignore`, already compiled in.** `ignore::gitignore::GitignoreBuilder` builds
one matcher per ignore file; `Gitignore::matched` and
`matched_path_or_any_parents` answer a path against that one file. Composing
nested files, deepest first with whitelists, is left to the caller — the
walker does it with a private type (`ignore::dir::Ignore`) that is not usable
from a one-level listing or a watcher event. `Gitignore::global()` reads
`$HOME/.gitconfig` itself to find `core.excludesFile`. It has no index, so a
tracked file that matches a pattern is ignored — ripgrep's semantics, not Git's.

**What `gix` is allowed to read here.** `crates/git/src/open.rs:103-104` opens
every repository `Options::isolated()` with a deny-all config-section filter.
Reading `gix` 0.87.1: `excludes_file()` goes through `trusted_file_path` with
that filter (`gix-0.87.1/src/config/cache/access.rs:257-258`), and the XDG
fallback (`~/.config/git/ignore`, `access.rs:406-409`) is refused because
isolated permissions deny `home` and `xdg_config_home`
(`gix-0.87.1/src/open/permissions.rs:159-170`). So **`git.status` today honours
in-tree `.gitignore` files and `.git/info/exclude`, and not the user's global
excludes file.** This was established by reading the source; no test asserts
it.

## Options

**A. Keep the eleven names (status quo).** Costs nothing now. Leaves both
disagreements above in place, and leaves quick-open with generated directories
it cannot filter and tracked files it cannot find.

**B. Honour ignore files everywhere, and hide what they exclude.** What an
editor with `files.exclude`-style behaviour does. Contradicts security.md's
"nothing is filtered": a user who wants to open `target/debug/build/…/output`
or a gitignored `.env` must still be able to see it in the tree.

**C. One predicate with Git's meaning, handed to every surface; the explorer
marks, search and quick-open skip.** Today's stance (listed-and-badged,
skipped-unless-asked, pruned from the watch) with a correct definition behind
it.

**D. A user setting (`files.exclude`-like globs, ADR-031).** Useful, and not an
answer to this question: a setting is a third source of rules, not a meaning.
It can be added later as one more input to C's predicate.

**E. Turn `ignore`'s gitignore sources on in search only.** The fastest route to
honouring `.gitignore` in search, and precisely the "taken by accident, in the
crate that happens to need a walker first" ADR-022 rejected: a third definition,
with ripgrep's pattern-only semantics, beside `gix`'s in the Changes panel.

## Decision (recommended)

**Option C.**

**1. The meaning.** When the workspace root *is* a repository (opened at the
root, never discovered above it, exactly as `git.status` does — T100 decision
3), an entry is ignored when it is **not tracked** and either Git's exclude
rules match it or one of its segments is in `IGNORED_DIRECTORY_NAMES`. `.git`
is always ignored. When the root is not a repository, the eleven names are the
whole rule, as today. The tracked clause is Git's own (`git check-ignore`
without `--no-index`), and it is the part that matters most: search must never
be blind to a file the Changes panel says the user modified.

**2. One engine, `gix`, owned by `gritty-git`.** `gritty-git` exposes an
`IgnoreRules` value built from the repository, the index snapshot and the
in-tree ignore files, with a `gix`-free API
(`is_ignored(&WorkspacePath, is_dir) -> bool`), `Send + Sync`. `gix` types do
not cross it (ADR-023 decision 3). `ignore`'s gitignore module stays unused;
ADR-022 decision 2's walker configuration is unchanged.

**3. The predicate is injected, not called.** `gritty-git` depends on
`gritty-workspace` (`crates/git/Cargo.toml:20`), so the workspace crate cannot
reach `IgnoreRules`. `gritty-core`, which already composes both, builds the
predicate when a workspace opens and hands it to `list_directory`, the watch
and both walks — search's existing `IgnorePredicate` (`query.rs:64`) moves to
`gritty-workspace`, gains `is_dir`, and `is_ignored` remains as the
non-repository predicate. Two producers of one closure type, not a trait with
one implementation.

**4. `ignoredHint` means, afterwards:** *this entry is ignored by the
workspace's rules (decision 1).* It is set on files as well as directories,
from the same predicate as everything else, so the explorer can no longer
disagree with search about a file named `build`. Nothing is hidden. Its wire
doc changes (regenerating `packages/protocol/src/generated`). ADR-009 bumps
MINOR for additive changes and says nothing about a changed meaning on an
unchanged field; decision 5's new request option is additive, so the release
that carries both takes one MINOR `PROTOCOL_VERSION` either way.

**5. The two `includeIgnored` fields keep separate, documented meanings.**
`git.status`'s stays Git's exact meaning — it is Git's status. `search.query`'s
means decision 1. They differ only by the eleven names on an untracked,
un-gitignored path, and each field's doc says which one it is.
`workspace.findFiles` gains the same option, since a quick-open that cannot
reach an ignored file is a dead end.

**6. Freshness without a new event.** The rules are a snapshot held by the open
workspace. It is rebuilt when a watcher batch reports a directory whose
`.gitignore` changed, or when a consumer finds `.git/index` or
`.git/info/exclude` with a different mtime than the snapshot recorded (two
`stat`s per call, not per entry — both files live inside `.git`, whose events
the watcher drops). A batch in which the rules changed is delivered as
`overflowed: true`, which already makes the renderer re-list every directory it
has loaded (ADR-025): badges converge through the existing mechanism, bounded by
what the user expanded.

**7. Not in scope:** the user's global excludes file (`core.excludesFile`,
`~/.config/git/ignore`). `git.status` does not read it (ADR-023's isolation),
and honouring it in the explorer alone would reopen disagreement 1. It is
reading user configuration, not running a program, so it is a legitimate later
change — made to `gritty-git`'s open options once, for both surfaces.

## Consequences

- **Users will see different results, in both directions.** Search and
  quick-open stop returning gitignored `.next/`, `coverage/`, `vendor/`, `*.log`
  and the like, and **start** returning tracked `dist/`/`build/` content they
  could not find before. The explorer shows more `ignored` badges, including on
  files. That is the change being asked for, and it should be announced in the
  completion report of the ticket that ships it, not discovered.
- **Gritty still disagrees with the user's terminal** where their global
  excludes file hides something, exactly as `git.status` already does (ADR-023's
  accepted consequence). A `.DS_Store` in a global ignore is badged nowhere and
  is searched. Explorer, Changes panel and search agree with *each other*.
- **Watcher cost stays inside ADR-025's bounds.** On macOS (`Recursive`) the
  predicate replaces a name comparison per event on the pump thread; the batch
  cap, debounce and ceiling are unchanged. On Linux (`PerDirectory`) the
  registration walk prunes by the predicate, which *removes* registrations for
  gitignored build output that `MAX_WATCHED_DIRECTORIES` (8,192,
  `watch.rs:58`) is spent on today. The cost is staleness: a rule change that
  un-ignores a directory leaves it unregistered until a change is next reported
  in its parent — documented, not hidden.
- **Listing cost moves.** `workspace.list` answers inline within
  `DEFAULT_LIST_BUDGET` (100 ms, `crates/gritty-core/src/workspace/mod.rs:44`).
  With a warm snapshot a lookup is per entry and cheap; building the snapshot
  reads the index once per workspace open or index change, and a large index
  can cost tens of milliseconds. It must be built off the listing's path (at
  open, on the blocking pool), and measured against a real repository — the
  generated `fixtures/large-repo` has a decoy `.git` and would exercise only
  the non-repository branch.
- **Untrusted input (hard rule 13).** A repository's `.gitignore` files are
  attacker-authored. They run no program (excludes name patterns, not
  executables), but a pathological glob costs matching time on every entry.
  An ignore file above a byte bound is treated as absent and counted, and a
  failure to build the rules degrades to the eleven names — the failure
  direction shows *more*, never hides more. A `.gitignore` that is a symlink
  resolving outside the root is not read (ADR-010); whether `gix`'s stack
  follows one is asserted by a test, not assumed. A repository-local
  `core.excludesFile` stays unread, because `open.rs`'s deny-all section filter
  already refuses it.
- **Tests are differential against Git itself.** `crates/git/src/fixtures.rs`
  may run the `git` binary with user configuration off. Each fixture — nested
  `.gitignore` with negation, an anchored pattern, a pattern ending in `/`,
  `.git/info/exclude`, a force-added tracked file under an ignored directory, a
  file named `build` — is compared path by path with `git check-ignore`'s
  answer on the same fixture. Beside that: the listing, the watcher and both
  walks are asserted to consult the injected predicate (the shape
  `crates/search/tests/walk.rs` already uses), and a rules change is asserted to
  produce an `overflowed` batch.
- **Settled by this ADR:** `ignore` remains a walker only; no second parser of
  `.gitignore` enters the tree.

## Alternatives rejected

- **A** — leaves two live disagreements, one of which hides tracked files from
  search.
- **B** — hiding contradicts security.md and the explorer's whole stance, and
  turns an ignored secret or log into something the user cannot open from the
  tree.
- **D as the decision** — a setting is an input, not a meaning; it can feed
  decision 1's predicate later without reopening this ADR.
- **E** — a third definition, pattern-only (tracked files hidden), confined to
  one surface: the mistake ADR-022 was written to avoid.
- **`ignore::gitignore` in `gritty-workspace`**, which would avoid the
  injection in decision 3: a second parser of the file format `gix` already
  parses for the Changes panel, no index so no tracked clause, and nesting
  re-implemented by hand because the walker's composition is private.

## Follow-up tickets this would create

Added to the board as `unspec` on acceptance, each waiting for its own task
file. They run in order: RULES, then SEAM, then WIRE, then UI.

- **IGNORE-RULES** — `IgnoreRules` in `gritty-git`: `gix` excludes stack plus
  index, bounded ignore-file reads, differential fixtures against
  `git check-ignore`.
- **IGNORE-SEAM** — move `IgnorePredicate` to `gritty-workspace` with `is_dir`,
  thread it through `list_directory`, the watch and both walks; `ignoredHint`
  on files; `gritty-core` composition and the snapshot's freshness rules.
- **IGNORE-WIRE** — `ignoredHint` and both `includeIgnored` docs, `includeIgnored`
  on `workspace.findFiles`, the MINOR protocol bump and regenerated types.
- **IGNORE-UI** — surface the quick-open/search toggle, and decide whether the
  explorer de-emphasises ignored rows beyond the text badge (no artboard draws
  it today).

## Revisit when

A user asks for their global excludes file to count (decision 7, changed once
in `gritty-git` for both surfaces); a workspace opened *inside* a repository
wants its rules (T100 follow-up 1 — today it is not a repository, so it gets
the eleven names); a measured listing or walk over a real repository misses its
budget because of the lookup; a settings-based exclude is requested (option D,
as an input); or `gix`'s excludes API changes shape across an upgrade.
