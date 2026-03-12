import path from "path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import wasm from "vite-plugin-wasm";
import topLevelAwait from "vite-plugin-top-level-await";

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    wasm(),
    topLevelAwait(),
  ],
  root: "src/mainview",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src/mainview"),
    },
  },
  worker: {
    format: "es",
  },
  build: {
    outDir: "../../dist",
    emptyOutDir: true,
    target: "es2022",
    sourcemap: false,
    reportCompressedSize: false,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("@replit/codemirror-vim")) return "editor-vim";
          if (id.includes("codemirror-lang-typst")) return "editor-typst";
          if (id.includes("@codemirror/lang-") || id.includes("@lezer")) return "editor-language";
          if (id.includes("@codemirror") || id.includes("/codemirror/")) return "editor-core";
          if (id.includes("@myriaddreamin") || id.includes(".wasm")) return "typst-core";
          if (id.includes("react")) return "react-core";
          return "vendor";
        },
      },
    },
  },
  server: {
    port: 5173,
    strictPort: true,
  },
});
