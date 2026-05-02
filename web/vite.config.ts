import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [tailwindcss(), react()],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:26502',
    },
  },
  build: {
    outDir: 'dist',
    assetsDir: 'assets',
  },
})
