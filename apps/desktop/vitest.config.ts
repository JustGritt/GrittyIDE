import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    name: 'desktop',
    // Node stays the default: most of the shell's logic is pure, and a
    // project-wide jsdom would be both slower and less honest — a test that
    // never meant to touch the DOM would silently be handed one. Component
    // tests opt in per file with a `/** @vitest-environment jsdom */`
    // docblock, the pattern `preview-links.test.ts` and `packages/editor`'s
    // `model-eol.dom.test.ts` already use. `packages/markdown` flips its whole
    // project the other way because there every test needs a DOM (DOMPurify).
    environment: 'node',
    // `.tsx` so a React component can be rendered against a real DOM and
    // asserted on, not only rendered to a static string.
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
    // CSS stays stubbed — that is what keeps a component test from paying to
    // process a stylesheet it never looks at — except for the one file a test
    // reads as *text*. `tokens.css` declares the light ramp twice so an
    // explicit theme choice can beat the operating system (T120-T, ADR-016
    // amendment), and `features/theme/tokens-duplication.test.ts` parses it to
    // prove the two copies still agree. Without this its `?raw` import is an
    // empty string and the guard passes by finding nothing.
    //
    // Narrower than `@types/node` in `tsconfig.json`, which was the
    // alternative: this is test-only, where that would put Node's globals in
    // reach of production WebView code.
    //
    // Unanchored on purpose: the pattern is matched against the module id,
    // which carries the `?raw` query, so a `$` here silently matches nothing
    // and the guard goes back to passing on an empty string.
    css: { include: [/tokens\.css/] },
  },
});
