import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// Built assets served under random console path: /{console}/assets/...
export default defineConfig({
  plugins: [vue()],
  resolve: {
    // Locale files stay as ordinary JSON strings. The runtime-only composer
    // avoids new Function(), which is intentionally blocked by the panel CSP.
    alias: { 'vue-i18n': 'vue-i18n/dist/vue-i18n.runtime.esm-bundler.js' },
  },
  base: './',
  build: {
    outDir: '../admin-spa',
    emptyOutDir: true,
    assetsDir: 'assets',
  },
  server: {
    port: 5173,
    proxy: {
      // local dev: proxy API to gr-service console path — set VITE_CONSOLE in env
    },
  },
})
