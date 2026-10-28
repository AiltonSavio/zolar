/// <reference types="vitest/config" />
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

// The generated clients live outside this package, so their bare imports
// must resolve to the single copy of @solana/kit installed here.
export default defineConfig(({ mode }) => ({
  // The generated clients read process.env.NODE_ENV.
  define: { 'process.env.NODE_ENV': JSON.stringify(mode) },
  plugins: [react()],
  resolve: {
    alias: {
      '@solana/kit': here('./node_modules/@solana/kit'),
      '@solana/program-client-core': here('./node_modules/@solana/program-client-core'),
      '@relay-client': here('../clients/js/src/generated'),
      '@escrow-client': here('../clients/escrow-js/src/generated'),
    },
  },
  server: { host: true, allowedHosts: true, fs: { allow: [here('..')] } },
  test: { environment: 'jsdom', include: ['src/**/*.test.{ts,tsx}'] },
}));
