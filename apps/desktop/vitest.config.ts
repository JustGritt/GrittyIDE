import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    name: 'desktop',
    // Pure unit tests only; DOM/component tests (jsdom) arrive with the editor work (T040).
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
