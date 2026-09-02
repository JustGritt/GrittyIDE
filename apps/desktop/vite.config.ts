import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Set by `tauri dev` when developing against a remote host; unset for plain desktop dev.
const host = process.env.TAURI_DEV_HOST;
const debug = Boolean(process.env.TAURI_ENV_DEBUG);

export default defineConfig({
  plugins: [react()],
  // Keep Rust compiler output visible in the `tauri dev` terminal.
  clearScreen: false,
  server: {
    // Must match `build.devUrl` in src-tauri/tauri.conf.json.
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    // WKWebView on macOS 14 (Safari 17) is the minimum supported target.
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome105' : 'safari17',
    minify: !debug,
    sourcemap: debug,
  },
});
