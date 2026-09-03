# Documents and buffers

Status: fixed by T030 (ADR-011). The short reference for the document
contract; `gritty_core::document` is the implementation, `crates/workspace`
holds the file primitives.

## Wire shape

```
Document {
  id, workspaceId, path, encoding: "utf-8" | "utf-8-bom" | "binary",
  languageId, savedRevision, savedHash: "sha256:<hex>", currentVersion,
  dirty, readOnly, binary, largeFileMode,
  externalModificationState: UNCHANGED | MODIFIED_EXTERNALLY | CONFLICTED | DELETED_EXTERNALLY,
  size
}
DocumentContent { document, text? }      // text absent for binary documents
```

| Command | Request | Returns | Notes |
|---|---|---|---|
| `documents.open` | `{ workspaceId, path, largeFileEditing? }` | `DocumentContent` | idempotent per path: an open document returns its current buffer; `largeFileEditing` lifts read-only on a large document |
| `documents.close` | `{ documentId, force? }` | `Document` | `Conflict` for a dirty document without `force` |
| `documents.update` | `{ documentId, baseVersion, text }` | `Document` | `Conflict` when `baseVersion != currentVersion`; `PermissionDenied` when read-only; `InvalidInput` for binary or above 64 MiB |
| `documents.save` | `{ documentId, force? }` | `Document` | atomic write; `Conflict` when the disk differs from `savedHash` or the file is gone, unless `force` |
| `documents.reload` | `{ documentId }` | `DocumentContent` | replaces the buffer from disk, bumps `currentVersion` and `savedRevision`; `NotFound` (and `DELETED_EXTERNALLY`) when the file is gone |
| `documents.status` | `{ documentId? }` | `Document[]` | re-checks the disk for one or every open document |
| `documents.list` | – | `Document[]` | rehydration after a renderer reload |

Events: `DocumentOpened { document }`, `DocumentChanged { document, cause: EDIT
| RELOAD | READ_ONLY_CHANGED }`, `DocumentSaved { document }`,
`DocumentExternalChangeDetected { document, previousState }` (actor
`EXTERNAL`, transitions only), `DocumentClosed { documentId, workspaceId,
path, reason: USER | WORKSPACE_CLOSED, dirty }`. Events never carry text.

## State matrix

`dirty = sha256(encode(buffer)) != savedHash`. The disk state comes from the
last check (`status`, `save`, `reload`); it is not watched (T070).

| Disk since last sync | Buffer clean | Buffer dirty |
|---|---|---|
| same hash | `UNCHANGED` | `UNCHANGED` |
| different hash (or grown past the read bound) | `MODIFIED_EXTERNALLY` | `CONFLICTED` |
| missing | `DELETED_EXTERNALLY` | `DELETED_EXTERNALLY` |

What each state allows:

| State | `save` | `save { force }` | `reload` | `update` |
|---|---|---|---|---|
| `UNCHANGED` | writes when dirty, no-op when clean | same | replaces buffer | accepted |
| `MODIFIED_EXTERNALLY` | `Conflict` | overwrites the disk | replaces buffer (safe) | accepted → `CONFLICTED` |
| `CONFLICTED` | `Conflict` | overwrites the disk | drops the edits | accepted |
| `DELETED_EXTERNALLY` | `Conflict` | recreates the file (parent must exist) | `NotFound` | accepted |

The UI must offer the choice (reload / keep buffer / overwrite) and never pick
one. Undoing the edits so the buffer is clean again moves `CONFLICTED` back to
`MODIFIED_EXTERNALLY`.

## Versions

- `currentVersion`: 1 after open; +1 per accepted `update` and per `reload`.
  Every `update` quotes the version it was derived from; a stale one is
  refused, which is what protects a reload from an edit that was in flight.
  Callers serialize their updates per document.
- `savedRevision`: 1 after open; +1 per save and per reload.

## Detection and limits

| Rule | Value |
|---|---|
| binary | any NUL byte, or invalid UTF-8 anywhere in the file (UTF-16 therefore reads as binary) |
| BOM | `EF BB BF` stripped from the text, written back on save |
| line endings | opaque; stored and written exactly as edited |
| large file | size ≥ 10 MiB (`LARGE_FILE_THRESHOLD`): `largeFileMode: true`, read-only until `largeFileEditing`; decided at open/reload |
| refused | size > 64 MiB (`MAX_DOCUMENT_BYTES`): `InvalidInput` with `details.reason = "too large"` |
| read-only | no write permission bit (`Permissions::readonly`), or large without `largeFileEditing` |

## Atomic write

`write_file_atomic` in `crates/workspace/src/file.rs`: create
`.<name>.gritty-<pid>-<n>.tmp` in the destination directory (`create_new`) →
write → `fsync` → copy the destination's permissions → `rename` over the
destination → `fsync` the directory (Unix). Any failure removes the temporary
file and leaves the destination as it was. A crash between write and rename
leaves the temporary file (its content is the unsaved buffer); it is
recognizable through `is_temp_file_name` and its cleanup is a follow-up. The
destination is the canonical path, so writing through a symlink inside the
root updates the target and keeps the link. Ownership is not preserved (no
privilege for that); permissions are.

## Language ids

Exact file names first, then the lower-cased extension; unknown → `plaintext`.
The vocabulary is VS Code's / Monaco's; T040 maps ids Monaco does not know
(`toml`, `ignore`, `dotenv`, `makefile`, `cmake`) to `plaintext`.

| Kind | Mapping |
|---|---|
| file names | `Dockerfile` → dockerfile, `Makefile` → makefile, `Cargo.lock` → toml, `CMakeLists.txt` → cmake, `.gitignore` `.gitattributes` `.dockerignore` → ignore, `.npmrc` `.editorconfig` → ini, `.env` → dotenv |
| extensions | ts mts cts tsx → typescript; js mjs cjs jsx → javascript; rs → rust; json jsonc → json; md markdown → markdown; toml; yaml yml → yaml; html htm → html; css; scss; less; py → python; go; java; kt → kotlin; swift; c h → c; cpp cc cxx hpp hh → cpp; cs → csharp; rb → ruby; php; sh bash zsh → shell; ps1 → powershell; sql; xml svg plist → xml; ini; txt log → plaintext |

Source of truth: `FILE_NAMES` and `EXTENSIONS` in `crates/workspace/src/detect.rs`.

## Unsaved changes and workspaces

`DocumentManager` installs itself as the `WorkspaceCloseGuard`: `workspace.close`
and a replacing `workspace.open` fail with `Conflict` (`details.dirtyDocuments`)
while any document of that workspace is dirty, unless the request carries
`force: true`. Whatever the reason a workspace closes (`USER`, `REPLACED`,
`ROOT_MISSING`), its documents are closed with `DocumentClosed { reason:
WORKSPACE_CLOSED, dirty }`; the desktop shell keeps the text of dirty ones on
screen as "unsaved buffers from closed documents" until dismissed. Durable
recovery of dirty buffers is T060.

## Fixture

`fixtures/weird-files` (committed) plus `generated/` (git-ignored, written by
`gritty_workspace::fixtures::stage_weird_files` / the `generate_weird_files`
example): Unicode and space-laden names, a 100k-character line, binary and
invalid UTF-8 files, BOM / CRLF / mixed EOL / UTF-16 samples, hidden entries,
symlinks (file, directory, broken), a 10-level path, an empty file, a 12 MiB
text file and a mode-000 file. `fixtures/README.md` has the inventory.
