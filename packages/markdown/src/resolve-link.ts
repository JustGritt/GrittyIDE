/**
 * Classify a Markdown link relative to the document it appears in. Runs
 * before sanitization (a `path:12` suffix looks like a URL scheme to every
 * URL parser and would otherwise be stripped) and produces workspace paths
 * that satisfy the ADR-010 wire grammar: relative, `/`-separated, no `.` or
 * `..` segments, no leading or trailing slash.
 */
export type LinkTarget =
  | {
      readonly kind: 'document';
      readonly path: string;
      readonly line?: number;
      readonly column?: number;
      readonly fragment?: string;
    }
  | { readonly kind: 'anchor'; readonly id: string }
  | { readonly kind: 'external'; readonly url: string }
  | { readonly kind: 'unsupported'; readonly reason: string };

const SCHEME = /^([a-z][a-z0-9+.-]*):/i;
const LINE_SUFFIX = /^(.+?):(\d{1,9})(?::(\d{1,9}))?$/;

function unsupported(reason: string): LinkTarget {
  return { kind: 'unsupported', reason };
}

function decode(value: string): string | null {
  try {
    return decodeURIComponent(value);
  } catch {
    return null;
  }
}

/** Directory of a workspace path (`''` for the root). */
export function dirname(path: string): string {
  const index = path.lastIndexOf('/');
  return index === -1 ? '' : path.slice(0, index);
}

/** Join and normalize against the wire grammar; `null` when the result escapes the root. */
export function joinWorkspacePath(base: string, relative: string): string | null {
  const segments = base === '' ? [] : base.split('/');
  for (const segment of relative.split('/')) {
    if (segment === '' || segment === '.') continue;
    if (segment === '..') {
      if (segments.length === 0) return null;
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return segments.join('/');
}

export function resolveLink(rawHref: string, fromPath: string): LinkTarget {
  const href = rawHref.trim();
  if (href === '') return unsupported('empty link');

  if (href.startsWith('#')) {
    const fragment = decode(href.slice(1));
    if (fragment === null || fragment === '') return unsupported('empty anchor');
    return { kind: 'anchor', id: fragment };
  }

  if (/^https?:\/\//i.test(href)) return { kind: 'external', url: href };

  // `path:line[:col]` before any scheme check: `notes.md:12` parses as a scheme.
  let pathPart = href;
  let line: number | undefined;
  let column: number | undefined;
  let fragment: string | undefined;
  const hash = pathPart.indexOf('#');
  if (hash !== -1) {
    fragment = decode(pathPart.slice(hash + 1)) ?? undefined;
    pathPart = pathPart.slice(0, hash);
  }
  const query = pathPart.indexOf('?');
  if (query !== -1) pathPart = pathPart.slice(0, query);
  const suffix = LINE_SUFFIX.exec(pathPart);
  if (suffix?.[1] !== undefined && !suffix[1].includes(':')) {
    pathPart = suffix[1];
    line = Number(suffix[2]);
    if (suffix[3] !== undefined) column = Number(suffix[3]);
  }

  const scheme = SCHEME.exec(pathPart);
  if (scheme?.[1] !== undefined) {
    return unsupported(`links with the '${scheme[1].toLowerCase()}:' scheme are not supported`);
  }

  const decoded = decode(pathPart);
  if (decoded === null) return unsupported('malformed percent-encoding');
  const decodedScheme = SCHEME.exec(decoded);
  if (decodedScheme?.[1] !== undefined) {
    return unsupported(
      `links with the '${decodedScheme[1].toLowerCase()}:' scheme are not supported`,
    );
  }
  if (decoded.includes('\\')) return unsupported('backslashes are not allowed in paths');
  if (decoded.includes('\0')) return unsupported('NUL in path');
  if (decoded === '' || decoded.endsWith('/')) return unsupported('not a file path');

  const rootRelative = decoded.startsWith('/');
  const joined = joinWorkspacePath(
    rootRelative ? '' : dirname(fromPath),
    rootRelative ? decoded.replace(/^\/+/, '') : decoded,
  );
  if (joined === null) return unsupported('outside the workspace');
  if (joined === '') return unsupported('not a file path');

  const target: {
    -readonly [K in keyof Extract<LinkTarget, { kind: 'document' }>]: Extract<
      LinkTarget,
      { kind: 'document' }
    >[K];
  } = { kind: 'document', path: joined };
  if (line !== undefined && line > 0) target.line = line;
  if (column !== undefined && column > 0) target.column = column;
  if (fragment !== undefined && fragment !== '') target.fragment = fragment;
  return target;
}

/** `docs/x.ts:3:5` for tooltips. */
export function describeTarget(target: LinkTarget): string {
  switch (target.kind) {
    case 'document': {
      let text = target.path;
      if (target.line !== undefined) text += `:${target.line}`;
      if (target.column !== undefined) text += `:${target.column}`;
      if (target.fragment !== undefined) text += `#${target.fragment}`;
      return text;
    }
    case 'anchor':
      return `#${target.id}`;
    case 'external':
      return target.url;
    case 'unsupported':
      return target.reason;
  }
}
