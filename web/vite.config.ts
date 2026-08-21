import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    tailwindcss(),
    react(),
    VitePWA({
      registerType: "autoUpdate",
      workbox: {
        // restty embeds its terminal WASM runtime in the application chunk.
        maximumFileSizeToCacheInBytes: 8 * 1024 * 1024,
        globPatterns: ["**/*.{js,css,svg,png,woff2,ico}"],
        runtimeCaching: [
          {
            // API calls — Network First with 5s timeout, fall back to cache
            urlPattern: /^\/api\/(?!events|terminal\/events)/,
            handler: "NetworkFirst",
            options: {
              cacheName: "api-cache",
              expiration: {
                maxEntries: 100,
                maxAgeSeconds: 300, // 5 minutes
              },
              networkTimeoutSeconds: 5,
            },
          },
          {
            // SPA shell — Network First, fallback to cache
            urlPattern: /\/$/,
            handler: "NetworkFirst",
            options: {
              cacheName: "html-cache",
            },
          },
        ],
      },
      manifest: {
        name: "tidev",
        short_name: "tidev",
        description: "AI-powered coding assistant",
        theme_color: "#ffffff",
        background_color: "#ffffff",
        display: "standalone",
        start_url: "/",
        icons: [
          {
            src: "/favicon.svg",
            sizes: "any",
            type: "image/svg+xml",
          },
        ],
      },
    }),
  ],
  server: {
    port: 5173,
    // The Rust HTTP adapter proxies the Vite assets, but must not proxy the
    // persistent HMR WebSocket. Point the browser at Vite directly so a
    // reconnect cannot retain a proxy-side socket pair.
    ws: {
      clientPort: 5173,
    },
    proxy: {
      "/api": {
        target: "http://127.0.0.1:26502",
        ws: true,
      },
      "/health": {
        target: "http://127.0.0.1:26502",
      },
    },
  },
  build: {
    outDir: "dist",
    assetsDir: "assets",
    rolldownOptions: {
      output: {
        manualChunks(id: string) {
          if (id.includes("node_modules/react-dom") || id.includes("node_modules/react/")) {
            return "vendor-react";
          }
          // Only eager CodeMirror core packages — leave dynamic language imports split naturally
          if (
            id.includes("node_modules/@codemirror/state") ||
            id.includes("node_modules/@codemirror/view") ||
            id.includes("node_modules/@codemirror/commands") ||
            id.includes("node_modules/@codemirror/autocomplete") ||
            id.includes("node_modules/@codemirror/language") ||
            id.includes("node_modules/@codemirror/search")
          ) {
            return "vendor-codemirror";
          }
        },
      },
    },
  },
});
