import { describe, expect, it } from 'vitest';

/**
 * MONACO-DEFER. Monaco is ~4.2 MB and the app boots with no document open, so
 * it is reached only through `import()`. That property is invisible in review —
 * one convenience import puts it back in the startup chunk and nothing else
 * fails — so it is asserted here rather than left to a build someone remembers
 * to measure.
 */

/** Every app source file, as text. Vite resolves this at transform time. */
const sources = import.meta.glob('../../**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

/** Modules that pull Monaco. Reaching one of these statically defeats the split. */
const MONACO = [/^@gritty\/editor\/monaco$/, /^monaco-editor(\/|$)/, /monaco-setup$/];
/**
 * The one module allowed to import Monaco: it is itself only ever `import()`ed.
 * Vite normalises glob keys against the importing file, so this is matched as a
 * suffix rather than as a fixed path.
 */
const ALLOWED = /(^|\/)monaco-setup\.ts$/;

/**
 * Static, value-carrying imports only. `import type` is erased by TypeScript
 * and `import(...)` is the whole point, so neither is a finding.
 */
function staticValueImports(source: string): string[] {
  const specifiers: string[] = [];
  for (const match of source.matchAll(
    /(?:^|\n)\s*import\s+(type\s+)?([^;]*?)\bfrom\s*['"]([^'"]+)['"]/g,
  )) {
    const typeKeyword = match[1];
    const clause = match[2] ?? '';
    const specifier = match[3] ?? '';
    if (typeKeyword) continue;
    // `import { type A, type B } from` is also fully erased.
    const named = clause.match(/\{([\s\S]*)\}/);
    const onlyTypes =
      named !== null &&
      clause.replace(named[0], '').trim() === '' &&
      (named[1] ?? '')
        .split(',')
        .filter((part) => part.trim() !== '')
        .every((part) => /^\s*type\s/.test(part));
    if (onlyTypes) continue;
    specifiers.push(specifier);
  }
  // Bare side-effect imports (`import 'monaco-editor'`) carry values too.
  for (const match of source.matchAll(/(?:^|\n)\s*import\s*['"]([^'"]+)['"]/g)) {
    specifiers.push(match[1] ?? '');
  }
  return specifiers;
}

describe('Monaco stays off the startup path', () => {
  const offenders = Object.entries(sources)
    .filter(([path]) => !/\.test\.tsx?$/.test(path) && !ALLOWED.test(path))
    .flatMap(([path, source]) =>
      staticValueImports(source)
        .filter((specifier) => MONACO.some((pattern) => pattern.test(specifier)))
        .map((specifier) => `${path} imports ${specifier}`),
    );

  it('is imported only dynamically, or as a type', () => {
    expect(offenders).toEqual([]);
  });

  it('scanned a plausible number of files, so the glob has not gone stale', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(50);
  });

  it('still has exactly one module that pulls it', () => {
    const entry = Object.entries(sources).find(([path]) => ALLOWED.test(path));
    expect(entry, 'monaco-setup.ts not found').toBeDefined();
    expect(staticValueImports(entry?.[1] ?? '')).toContain('@gritty/editor/monaco');
  });
});
