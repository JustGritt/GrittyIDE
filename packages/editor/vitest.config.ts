import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    name: 'editor',
    // Pure unit tests against fake models by default; the one file that needs a
    // real Monaco model opts into jsdom with a `@vitest-environment` docblock.
    environment: 'node',
    include: ['src/**/*.test.ts'],
    server: {
      deps: {
        // monaco-editor ships ESM without "type": "module" and imports .css;
        // Node cannot load it externalized, so Vite transforms it.
        inline: [/monaco-editor/],
      },
    },
  },
});
