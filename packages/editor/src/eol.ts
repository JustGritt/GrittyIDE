/**
 * Line-ending classification. The core stores and writes text verbatim
 * (ADR-011); Monaco can only represent one line ending per model and rewrites
 * every `\r\n`, `\r` and `\n` to the dominant one when a model is created. A
 * consistently LF or CRLF text therefore round-trips byte-exact through a
 * model, anything else does not, which is why the editor refuses to edit
 * `cr` and `mixed` documents until the user asks for an explicit normalization.
 */
export type Eol = 'lf' | 'crlf' | 'cr' | 'mixed' | 'none';

/** The two line endings Monaco can produce. */
export type EolTarget = 'lf' | 'crlf';

export const EOL_SEQUENCE: Record<EolTarget, string> = { lf: '\n', crlf: '\r\n' };

const CR = 13;
const LF = 10;

export function detectEol(text: string): Eol {
  let lf = 0;
  let crlf = 0;
  let cr = 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code === CR) {
      if (text.charCodeAt(i + 1) === LF) {
        crlf += 1;
        i += 1;
      } else {
        cr += 1;
      }
    } else if (code === LF) {
      lf += 1;
    }
  }
  const kinds = (lf > 0 ? 1 : 0) + (crlf > 0 ? 1 : 0) + (cr > 0 ? 1 : 0);
  if (kinds === 0) return 'none';
  if (kinds > 1) return 'mixed';
  if (lf > 0) return 'lf';
  return crlf > 0 ? 'crlf' : 'cr';
}

/** Whether a Monaco model can hold this text without changing its bytes. */
export function isEditableEol(eol: Eol): boolean {
  return eol === 'lf' || eol === 'crlf' || eol === 'none';
}

/** Rewrite every line ending to `target`. Idempotent for already consistent text. */
export function normalizeEol(text: string, target: EolTarget): string {
  return text.replace(/\r\n|\r|\n/g, EOL_SEQUENCE[target]);
}

/**
 * The line ending Monaco would pick for this text (mirrors the piece-tree
 * builder: CRLF when more than half of the line endings carry a CR, else LF).
 * Used to pre-select the normalization target.
 */
export function dominantEol(text: string): EolTarget {
  let eols = 0;
  let withCr = 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code === CR) {
      eols += 1;
      withCr += 1;
      if (text.charCodeAt(i + 1) === LF) i += 1;
    } else if (code === LF) {
      eols += 1;
    }
  }
  return withCr > eols / 2 ? 'crlf' : 'lf';
}

export function eolLabel(eol: Eol): string {
  switch (eol) {
    case 'lf':
      return 'LF';
    case 'crlf':
      return 'CRLF';
    case 'cr':
      return 'CR';
    case 'mixed':
      return 'mixed';
    case 'none':
      return 'no line endings';
  }
}
