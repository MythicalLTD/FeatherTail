import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Served by the daemon under /ui/. `npm run dev` proxies the API to a running daemon:
//   FT_API=http://10.0.0.1:8686 npm run dev
export default defineConfig({
  base: '/ui/',
  plugins: [react()],
  build: { outDir: 'dist', emptyOutDir: true, chunkSizeWarningLimit: 1000 },
  server: {
    proxy: { '/api': { target: process.env.FT_API || 'http://127.0.0.1:8686', changeOrigin: true } },
  },
})
