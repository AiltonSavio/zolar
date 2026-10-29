/// <reference types="vitest/config" />
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

// The SDK is used from source, as a shop would use the published package.
export default defineConfig({
  plugins: [react()],
  build: { rollupOptions: { output: { manualChunks: { solana: ['@zolar/sdk'] } } } },
  resolve: { alias: { '@zolar/sdk': here('../sdk/src/index.ts') } },
  server: { fs: { allow: [here('..')] } },
  test: { environment: 'jsdom', setupFiles: ['src/testSetup.ts'], include: ['src/**/*.test.{ts,tsx}'] },
});
