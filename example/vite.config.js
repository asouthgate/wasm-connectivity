import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const wasmThreadHeaders = process.env.WASM_THREADS === '1' ? {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
} : undefined;

export default defineConfig({
  plugins: [react()],
  appType: 'spa',
  server: {
    port: 8080,
    strictPort: false,
    headers: wasmThreadHeaders,
    fs: {
      allow: ['.', '../lib']
    }
  },
  preview: { headers: wasmThreadHeaders },
  build: {
    target: 'esnext',
  },
});
