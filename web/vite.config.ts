import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

const api = 'http://localhost:8150';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5190,
    // The app is same-origin in production, so the dev server proxies rather
    // than the code learning about CORS and absolute API URLs.
    proxy: {
      '/api': { target: api, changeOrigin: true },
      '/f': { target: api, changeOrigin: true },
      '/t': { target: api, changeOrigin: true },
      '/s': { target: api, changeOrigin: true },
    },
  },
});
