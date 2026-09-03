# Fixtures

Deterministic repositories for tests (architecture addendum §10). Tests must
never depend on a developer's real project.

| Fixture | Purpose | Source |
|---|---|---|
| `small-repo/` | fast integration and explorer tests: TypeScript, Rust, JSON, Markdown, nested directories, a hidden file and directory, a file name with spaces, a symlink | committed |
| `large-repo/` | performance and cancellation tests: 10,500 files, a 24-level deep chain, `bulk/` with 5,200 entries (more than one listing page), ignored directories (`.git`, `node_modules`, `target`, `dist`) | generated, git-ignored |

## Generating `large-repo`

```
cargo run -p gritty-workspace --example generate_large_repo            # writes fixtures/large-repo
cargo run -p gritty-workspace --example generate_large_repo -- <dir>   # elsewhere
```

The generator (`crates/workspace/src/fixtures.rs`) is deterministic: same
layout and content on every run. `cargo test -p gritty-workspace` generates its
own copy under `target/fixtures/large-repo` and reuses it while the layout
version in `.gritty-fixture` matches.

## Symlinks on Windows

`small-repo/link-to-src` is committed as a symlink (mode `120000`). On a
checkout with `core.symlinks=false` (the Git for Windows default) it appears
as a plain file containing `src`; the Rust tests create real symlinks in a
temporary copy so their assertions do not depend on the checkout mode.
Creating symlinks on Windows needs Developer Mode or elevation.

Planned (addendum §10.3, §10.4): `weird-files/`, Markdown and Git fixtures
arrive with the tasks that need them (T030+, T100).
