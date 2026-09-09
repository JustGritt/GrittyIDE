/**
 * The vocabulary shared across quick open (T130-U, T130-S).
 *
 * Deliberately tiny. Only the types more than one module both produces and
 * consumes live here; everything owned by a single module stays in that
 * module. This file exists because two things are built in parallel against
 * it, which is the same reason a wire contract lands before its consumers.
 *
 * Nothing here touches the core protocol. `workspace.findFiles` is T130-C's and
 * does not exist yet, so the files mode's result shape is deliberately absent:
 * writing it now would be guessing at a contract nobody has agreed.
 */

/**
 * What the field is currently doing, decided by a prefix on the raw query.
 *
 * `files` is the default and has no prefix, which is why a path beginning with
 * `>`, `:` or `@` needs the parser's escape rule rather than a fourth mode.
 */
export type QuickOpenMode = 'files' | 'commands' | 'line' | 'symbols';

/** What kind of thing an outline row points at. */
export type OutlineKind =
  | 'heading'
  | 'function'
  | 'class'
  | 'interface'
  | 'type'
  | 'struct'
  | 'enum'
  | 'trait'
  | 'impl'
  | 'module'
  | 'constant';

/**
 * One entry in the current file's outline (T130-S).
 *
 * Produced by `outlineOf`, consumed by the `symbols` arm of the row model. It
 * is an *outline*, extracted by pattern from the buffer — not a symbol from a
 * language server, which Gritty has no source of. See `outline.ts`.
 */
export interface OutlineEntry {
  /** The text shown in the row: a heading, or a declared name. */
  readonly name: string;
  readonly kind: OutlineKind;
  /** 1-based, so it can be handed straight to the editor's reveal. */
  readonly line: number;
  /** Nesting depth. Markdown uses the heading level; code uses 0. */
  readonly depth: number;
}
