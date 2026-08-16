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

const WASM_HELPER_ID = "/__vite-plugin-wasm-helper";

/**
 * Replace vite-plugin-wasm's fetch/instantiateStreaming helper. Custom
 * schemes often give the wrong WASM MIME type or reject streaming fetch,
 * which would leave the editor chunk's top-level await hanging.
 */
function electrobunWasmHelper(): Plugin {
  const helper = `export default async function(opts = {}, url) {
  if (url.startsWith("data:")) {
    const urlContent = url.replace(/^data:.*?base64,/, "");
    let bytes;
    if (typeof Buffer === "function" && typeof Buffer.from === "function") {
      bytes = Buffer.from(urlContent, "base64");
    } else if (typeof atob === "function") {
      const binaryString = atob(urlContent);
      bytes = new Uint8Array(binaryString.length);
      for (let i = 0; i < binaryString.length; i++) {
        bytes[i] = binaryString.charCodeAt(i);
      }
    } else {
      throw new Error("Cannot decode base64-encoded data URL");
    }
    const dataResult = await WebAssembly.instantiate(bytes, opts);
    return dataResult.instance.exports;
  }

  const loadBuffer = async (target) => {
    try {
      const response = await fetch(target);
      const ok = response.ok || response.status === 0;
      if (ok) {
        const buffer = await response.arrayBuffer();
        if (buffer.byteLength > 0) return buffer;
      }
    } catch {}
    return await new Promise((resolve, reject) => {
      const request = new XMLHttpRequest();
      request.open("GET", target, true);
      request.responseType = "arraybuffer";
      request.onload = () => {
        const okStatus = request.status === 200 || request.status === 0;
        const response = request.response;
        if (okStatus && response instanceof ArrayBuffer && response.byteLength > 0) {
          resolve(response);
          return;
        }
        reject(new Error("Failed to load wasm module: " + request.status));
      };
      request.onerror = () => {
        reject(new Error("Failed to load wasm module: " + request.status));
      };
      request.send();
    });
  };

  const buffer = await loadBuffer(url);
  const result = await WebAssembly.instantiate(buffer, opts);
  return result.instance.exports;
}`;

  return {
    name: "electrobun-wasm-helper",
    enforce: "pre",
    resolveId(id) {
      if (id === WASM_HELPER_ID) return id;
    },
    load(id) {
      if (id === WASM_HELPER_ID) return helper;
    },
  };
}

const wasmPlugins = () => [wasm(), topLevelAwait()];

export default defineConfig({
  plugins: [
    electrobunWasmHelper(),
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
      // Skip <link rel="modulepreload" crossorigin> which native webviews
      // reject on views://. Dynamic import() still resolves relative URLs.
      resolveDependencies: () => [],
    },
  },
  server: {
    port: 5173,
    strictPort: true,
  },
});
