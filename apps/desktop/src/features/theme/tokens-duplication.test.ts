import { describe, expect, it } from 'vitest';

// The stylesheet and the Monaco adapter as text. `?raw` rather than `node:fs`:
// `apps/desktop` is a browser project and its tsconfig carries `vite/client`,
// not `@types/node`, so reading a file the Vite way is the one that typechecks.
// `import.meta.glob` rather than a plain `?raw` import, matching
// `monaco-deferred.test.ts`: it is how this project already reads a source file
// as text, and it resolves a path outside `apps/desktop` without ceremony.
const tokensCss = Object.values(
  import.meta.glob('../../styles/tokens.css', {
    query: '?raw',
    import: 'default',
    eager: true,
  }) as Record<string, string>,
)[0] as string;
const monacoThemeSource = Object.values(
  import.meta.glob('../../../../../packages/editor/src/monaco/theme.ts', {
    query: '?raw',
    import: 'default',
    eager: true,
  }) as Record<string, string>,
)[0] as string;

/**
 * The two copies of the light ramp, and what Monaco is allowed to be handed
 * (T120-T §2, ADR-016 amendment).
 *
 * The restructure that lets an explicit choice beat the operating system needs
 * light declared twice — once inside the media query, narrowed so Dark wins,
 * and once outside it for the user who chose Light while the OS asks for dark.
 * ADR-016's real invariant is that the two say the same thing, and this is what
 * holds it. The alternative was ~110 lines of `var(--light-*)` indirection to
 * make the duplication impossible; this is cheaper and it fails just as loudly.
 */

/** The stylesheet with comments removed, so one inside a block cannot confuse the scan. */
function source(): string {
  return tokensCss.replace(/\/\*[\s\S]*?\*\//g, '');
}

/** The custom properties one block declares, in order. */
function declarationsAfter(css: string, selector: string): Map<string, string> {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`no block for ${selector} in tokens.css`);
  const open = css.indexOf('{', at + selector.length - 1);
  // Balanced, so a nested block (the media query's) is consumed rather than
  // ending the scan at the wrong brace.
  let depth = 0;
  let end = open;
  for (; end < css.length; end += 1) {
    if (css[end] === '{') depth += 1;
    else if (css[end] === '}') {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  const body = css.slice(open + 1, end);
  const found = new Map<string, string>();
  for (const [, name, value] of body.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/gi)) {
    if (name !== undefined && value !== undefined) found.set(name, value.trim());
  }
  return found;
}

const dark = () => declarationsAfter(source(), ':root {');
const lightFromTheSystem = () => declarationsAfter(source(), ':root:not([data-theme="dark"]) {');
const lightByChoice = () => declarationsAfter(source(), ':root[data-theme="light"] {');

describe('the two light blocks', () => {
  it('declare exactly the same tokens', () => {
    expect([...lightByChoice().keys()].sort()).toEqual([...lightFromTheSystem().keys()].sort());
  });

  it('declare exactly the same values', () => {
    // The whole point. A value edited in one block and not the other means a
    // user who chose Light sees a different window from a user whose OS asked
    // for it, and nothing else in the build would ever say so.
    expect(Object.fromEntries(lightByChoice())).toEqual(Object.fromEntries(lightFromTheSystem()));
  });

  it('are not empty, so a broken scan cannot pass by finding nothing', () => {
    expect(lightByChoice().size).toBeGreaterThan(30);
  });
});

describe('the dark block', () => {
  it('is the bare :root, so it is what paints when nothing overrides it', () => {
    // Dark is the designed theme (artboard 2a) and the fallback everywhere:
    // the signal resolves to it when there is no `matchMedia` at all.
    expect(dark().get('--surface-base')).toBe('#0f1216');
  });

  it('declares every token the light ramp overrides', () => {
    // A light-only token would be undefined in dark, which is a token that
    // silently resolves to nothing rather than to a colour.
    const missing = [...lightByChoice().keys()].filter((name) => !dark().has(name));
    expect(missing).toEqual([]);
  });
});

describe('what Monaco is handed', () => {
  /**
   * The tokens `colorsFor` reads, taken from the file itself rather than
   * copied: a hand-kept list here would be one more thing to drift.
   */
  function monacoTokens(): string[] {
    return [
      ...new Set(
        [...monacoThemeSource.matchAll(/token\('(--[a-z0-9-]+)'/gi)].map(([, name]) => name ?? ''),
      ),
    ].filter((name) => name !== '');
  }

  it('reads tokens that exist', () => {
    const names = monacoTokens();
    expect(names.length).toBeGreaterThan(0);
    for (const name of names) {
      expect(dark().has(name), `${name} is read by Monaco but not declared`).toBe(true);
    }
  });

  it('is only ever given colours Monaco can parse, in both themes', () => {
    // `hexOnly` exists because Monaco *throws* on a colour it cannot parse, so
    // a token that became an `oklch` or a `color-mix` would be silently dropped
    // and the editor would quietly fall back to the base theme. This fails
    // instead, which is the difference between a bug you can see and one you
    // find months later by squinting at the gutter.
    const hex = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6,8})$/i;
    for (const name of monacoTokens()) {
      expect(dark().get(name), `${name} (dark)`).toMatch(hex);
      expect(lightByChoice().get(name), `${name} (light)`).toMatch(hex);
    }
  });
});
