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
  },
});
