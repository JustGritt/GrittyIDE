import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    name: 'protocol',
    // Pure unit tests against a fake transport; nothing here touches Tauri.
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
