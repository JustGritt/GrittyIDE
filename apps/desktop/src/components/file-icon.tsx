import { Glyph } from './icons';

/**
 * File-type icons (FILE-ICONS), for every place the window lists a file: the
 * explorer tree and the Changes list today, the editor tabs next.
 *
 * The look follows the Seti theme VS Code and Cursor ship — a small coloured
 * glyph per type, letters where a language is known by its letters — drawn
 * fresh here rather than vendored. The kind is decided from the name alone:
 * nothing is read, so a row never waits on the filesystem to get its icon.
 *
 * Colours come from CSS (`.file-icon--<kind>` in `app.css`), on the ANSI ramp
 * `tokens.css` already carries for both themes — the only set of named hues
 * the palette has.
 */
export type FileIconKind =
  | 'markdown'
  | 'json'
  | 'typescript'
  | 'react'
  | 'javascript'
  | 'rust'
  | 'python'
  | 'config'
  | 'yaml'
  | 'lock'
  | 'css'
  | 'html'
  | 'readme'
  | 'license'
  | 'git'
  | 'image'
  | 'shell'
  | 'sql'
  | 'text'
  | 'file'
  | 'symlink'
  | 'special';

const BY_EXTENSION: Readonly<Record<string, FileIconKind>> = {
  md: 'markdown',
  mdx: 'markdown',
  markdown: 'markdown',
  json: 'json',
  jsonc: 'json',
  json5: 'json',
  jsonl: 'json',
  tsbuildinfo: 'json',
  ts: 'typescript',
  mts: 'typescript',
  cts: 'typescript',
  tsx: 'react',
  jsx: 'react',
  js: 'javascript',
  mjs: 'javascript',
  cjs: 'javascript',
  rs: 'rust',
  py: 'python',
  toml: 'config',
  ini: 'config',
  cfg: 'config',
  conf: 'config',
  env: 'config',
  yaml: 'yaml',
  yml: 'yaml',
  lock: 'lock',
  css: 'css',
  scss: 'css',
  sass: 'css',
  less: 'css',
  html: 'html',
  htm: 'html',
  xml: 'html',
  png: 'image',
  jpg: 'image',
  jpeg: 'image',
  gif: 'image',
  webp: 'image',
  avif: 'image',
  bmp: 'image',
  ico: 'image',
  icns: 'image',
  svg: 'image',
  sh: 'shell',
  bash: 'shell',
  zsh: 'shell',
  fish: 'shell',
  sql: 'sql',
  txt: 'text',
  log: 'text',
};

/** Dotfiles are named, not extended: `.editorconfig` has no extension. */
const BY_DOTFILE: Readonly<Record<string, FileIconKind>> = {
  '.gitignore': 'git',
  '.gitattributes': 'git',
  '.gitmodules': 'git',
  '.gitkeep': 'git',
  '.editorconfig': 'config',
  '.npmrc': 'config',
  '.prettierrc': 'config',
  '.env': 'config',
  '.nvmrc': 'text',
  '.node-version': 'text',
};

export function fileIconKind(name: string): FileIconKind {
  const lower = name.toLowerCase();
  const dotfile = BY_DOTFILE[lower];
  if (dotfile !== undefined) return dotfile;
  if (lower.startsWith('.env.')) return 'config';
  // README and LICENSE outrank their extension: `README.md` is the project's
  // front page before it is a Markdown file, which is how Seti draws it too.
  if (/^readme(\.|$)/.test(lower)) return 'readme';
  // `LICENSE-MIT` and `LICENSE-APACHE` are how dual-licensed Rust crates ship.
  if (/^(license|licence|copying)([.-]|$)/.test(lower)) return 'license';
  const dot = lower.lastIndexOf('.');
  // A leading dot alone (`.hidden-file`) names the file; it is not an extension.
  if (dot <= 0 || dot === lower.length - 1) return 'file';
  return BY_EXTENSION[lower.slice(dot + 1)] ?? 'file';
}

export function FileIcon({ kind }: { kind: FileIconKind }) {
  return (
    <Glyph className={`file-icon file-icon--${kind}`}>
      <FileIconShape kind={kind} />
    </Glyph>
  );
}

/** Letters stand in for a language whose logo is its letters. */
function Letters({ children }: { children: string }) {
  return (
    <text className="file-icon__letters" x="8" y="11.25" textAnchor="middle">
      {children}
    </text>
  );
}

function FileIconShape({ kind }: { kind: FileIconKind }) {
  switch (kind) {
    case 'markdown':
      return (
        <>
          <path d="M8 2.5v8" />
          <path d="M4.5 7.5 8 11l3.5-3.5" />
          <path d="M3.5 13.5h9" />
        </>
      );
    case 'json':
      return (
        <>
          <path d="M6 2.5h-.5c-.8 0-1 .5-1 1.5v2.25C4.5 7.25 4 8 3 8c1 0 1.5.75 1.5 1.75V12c0 1 .2 1.5 1 1.5H6" />
          <path d="M10 2.5h.5c.8 0 1 .5 1 1.5v2.25c0 1 .5 1.75 1.5 1.75-1 0-1.5.75-1.5 1.75V12c0 1-.2 1.5-1 1.5H10" />
        </>
      );
    case 'typescript':
      return <Letters>TS</Letters>;
    case 'javascript':
      return <Letters>JS</Letters>;
    case 'rust':
      return <Letters>RS</Letters>;
    case 'python':
      return <Letters>PY</Letters>;
    case 'react':
      return (
        <>
          <ellipse cx="8" cy="8" rx="6.5" ry="2.5" />
          <ellipse cx="8" cy="8" rx="6.5" ry="2.5" transform="rotate(60 8 8)" />
          <ellipse cx="8" cy="8" rx="6.5" ry="2.5" transform="rotate(120 8 8)" />
          <circle cx="8" cy="8" r="1" fill="currentColor" stroke="none" />
        </>
      );
    case 'config':
      return (
        <>
          <circle cx="8" cy="8" r="4.25" />
          <circle cx="8" cy="8" r="1.5" />
          <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" />
        </>
      );
    case 'yaml':
      return (
        <>
          <path d="M8 2.75v6.5" strokeWidth="2" />
          <circle cx="8" cy="12.75" r="1.1" fill="currentColor" stroke="none" />
        </>
      );
    case 'lock':
      return (
        <>
          <rect x="3.5" y="7" width="9" height="6.5" rx="1" />
          <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
        </>
      );
    case 'css':
      return <path d="M6.5 2.5 5 13.5M11 2.5 9.5 13.5M3 6h10.5M2.5 10H13" />;
    case 'html':
      return <path d="M5 4.5 1.5 8 5 11.5M11 4.5 14.5 8 11 11.5M9 3l-2 10" />;
    case 'readme':
      return (
        <>
          <circle cx="8" cy="8" r="6" />
          <path d="M8 7.25v4" />
          <circle cx="8" cy="5" r="0.8" fill="currentColor" stroke="none" />
        </>
      );
    case 'license':
      return (
        <>
          <circle cx="8" cy="6" r="3.75" />
          <path d="M5.75 9 5 14.5l3-1.75 3 1.75L10.25 9" />
        </>
      );
    case 'git':
      return (
        <>
          <path d="M8 1.5 14.5 8 8 14.5 1.5 8z" />
          <circle cx="8" cy="8" r="1.25" fill="currentColor" stroke="none" />
        </>
      );
    case 'image':
      return (
        <>
          <rect x="2" y="3" width="12" height="10" rx="1.5" />
          <path d="M2.5 11.5 6 8l3 3 2-2 2.5 2.5" />
          <circle cx="10.5" cy="6" r="1" />
        </>
      );
    case 'shell':
      return <path d="M3 5l3 3-3 3M8 11.5h5" />;
    case 'sql':
      return (
        <>
          <ellipse cx="8" cy="4" rx="5" ry="2" />
          <path d="M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4" />
          <path d="M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" />
        </>
      );
    case 'text':
      return <path d="M3 4.5h10M3 8h10M3 11.5h7" />;
    case 'symlink':
      return <path d="M5 11 11 5M6.5 5H11v4.5" />;
    case 'special':
      return <circle cx="8" cy="8" r="5" strokeDasharray="2 2" />;
    case 'file':
      return (
        <>
          <path d="M4 1.5h5l3 3v10H4z" />
          <path d="M9 1.5v3h3" />
        </>
      );
  }
}
