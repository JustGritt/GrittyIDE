import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    name: 'markdown',
    // DOMPurify needs a DOM and is a silent no-op without one; every test in
    // this package runs under jsdom so a sanitizer test can never pass vacuously.
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
  },
});
