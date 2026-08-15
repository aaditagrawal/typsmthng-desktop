import path from "path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import wasm from "vite-plugin-wasm";
import topLevelAwait from "vite-plugin-top-level-await";

/**
 * Native webviews (WKWebView, WebView2, WebKitGTK) load the renderer from
 * `views://mainview/index.html`. Absolute `/assets/...` URLs and `crossorigin`
 * on module scripts fail on that custom protocol, which is what previously
 * stalled the Typst WASM renderer after a production build.
 */
function electrobunWebviewHtml(): Plugin {
  return {
    name: "electrobun-webview-html",
    transformIndexHtml(html) {
      return html
        .replace(/\s+crossorigin(="[^"]*")?/g, "")
        .replace(/<link\s+rel="modulepreload"[^>]*>/g, "");
    },
  };
}

const wasmPlugins = () => [wasm(), topLevelAwait()];

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    ...wasmPlugins(),
    electrobunWebviewHtml(),
  ],
  // Relative URLs so `views://mainview/index.html` can load `./assets/*`
  // on macOS, Windows, and Linux native webviews.
  base: "./",
  root: "src/mainview",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src/mainview"),
    },
  },
  optimizeDeps: {
    exclude: [
      "@myriaddreamin/typst.ts",
      "@myriaddreamin/typst-ts-web-compiler",
      "@myriaddreamin/typst-ts-renderer",
    ],
  },
  worker: {
    format: "es",
    plugins: wasmPlugins,
  },
  build: {
    outDir: "../../dist",
    emptyOutDir: true,
    target: "es2022",
    sourcemap: false,
    reportCompressedSize: false,
    // Keep WASM as separate files. Inlining as base64 breaks instantiate on
    // several system webviews.
    assetsInlineLimit: 0,
    modulePreload: {
      polyfill: false,
    },
  },
  server: {
    port: 5173,
    strictPort: true,
  },
});
