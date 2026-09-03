import { ID_PREFIX } from './sanitize';

/**
 * GitHub-style heading ids: lower-case, punctuation removed, spaces to
 * hyphens, duplicates suffixed `-1`, `-2`, …; prefixed so a document can
 * never mint an id the application uses.
 */
export function slugify(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{M}\p{N}\p{Pc}\- ]/gu, '')
    .replace(/ /g, '-');
}

export function createSlugger(): (text: string) => string {
  const seen = new Map<string, number>();
  return (text) => {
    const base = slugify(text);
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    return `${ID_PREFIX}${count === 0 ? base : `${base}-${count}`}`;
  };
}

/** The id a `#fragment` written GitHub-style points at. */
export function anchorId(fragment: string): string {
  return `${ID_PREFIX}${slugify(fragment)}`;
}
