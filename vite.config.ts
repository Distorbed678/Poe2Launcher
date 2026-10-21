/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri expects a fixed dev port (tauri.conf.json: devUrl).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: 'dist', sourcemap: false, target: 'es2022' },
  test: { environment: 'jsdom', include: ['src/**/*.test.ts'] },
});
