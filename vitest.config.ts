import { defineConfig } from 'vitest/config';

// Every workspace package is a Vitest project; each may add its own vitest.config.ts.
export default defineConfig({
  test: {
    projects: ['apps/*', 'packages/*'],
  },
});
