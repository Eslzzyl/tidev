import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// https://vite.dev/config/
export default defineConfig({
  plugins: [tailwindcss(), react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:26502",
        ws: true,
      },
    },
  },
  build: {
    outDir: "dist",
    assetsDir: "assets",
    rolldownOptions: {
      output: {
        manualChunks(id: string) {
          if (
            id.includes("node_modules/react-dom") ||
            id.includes("node_modules/react/")
          ) {
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
