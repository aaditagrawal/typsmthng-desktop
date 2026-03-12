# typsmthng-desktop

> **WARNING: ACTIVE DEVELOPMENT BETA, DO NOT USE FOR PRODUCTION**

The native desktop version of [typsmthng](https://github.com/aaditagrawal/typsmthng), a Typst editor with live preview. Built with Electrobun, so it runs on macOS, Linux, and Windows using native webviews instead of bundling Chromium.

## Why a desktop app?

The web version stores projects in IndexedDB. That works, but it means your `.typ` files live inside the browser, invisible to the rest of your system. The desktop version reads and writes directly to folders on disk; your files are just files. You can edit them in typsmthng-desktop, switch to another editor, use git, or back them up however you like.

The desktop app also adds a vault system for organizing multiple projects, full-text search across all your documents, and background task processing.

## Features

- Live Typst compilation to SVG preview (WebAssembly)
- Reads and writes `.typ` files directly on the filesystem
- Vault-based project organization with full-text search
- Multi-file projects with file tree, drag-and-drop, rename, delete
- PDF export
- LaTeX-to-Typst conversion
- Command palette search
- Vim mode, theme switching, editor preferences
- Background compilation and task queue
- Auto-update support

## Tech Stack

- Electrobun (native desktop shell, no Chromium)
- React 19 + TypeScript
- Vite 7
- CodeMirror 6
- Zustand
- Typst WASM toolchain via `@myriaddreamin/typst.ts`
- Tailwind CSS 4
- Vitest

## Getting Started

### Prerequisites

- Bun `1.3+`

### Install

```bash
bun install
```

### Run locally

```bash
# Development with file watching
bun run dev

# Development with hot module replacement
bun run dev:hmr
```

`dev` rebuilds on file changes. `dev:hmr` starts a Vite dev server on `localhost:5173` alongside Electrobun, so React component changes apply instantly without a full reload.

### Build

```bash
# Standard build
bun run build

# Canary build
bun run build:canary
```

## Scripts

- `bun run dev` - start Electrobun in dev mode with file watching
- `bun run dev:hmr` - start with Vite HMR for instant React updates
- `bun run build` - create production build
- `bun run build:canary` - create canary release build
- `bun run typecheck` - run TypeScript type checking
- `bun run test` - run Vitest once

## Project Structure

```
src/
├── bun/                  # Main process (runs in Bun, not the browser)
│   ├── index.ts          # Window creation, menu, RPC setup
│   └── services/         # Vault indexing, full-text search, background tasks
├── mainview/             # Renderer (React app loaded in the webview)
│   ├── components/       # UI: editor, preview, sidebar, search, settings
│   ├── stores/           # Zustand state stores
│   ├── lib/              # Compiler integration, keybindings, file I/O
│   └── workers/          # Typst compilation web worker
└── shared/               # RPC type definitions shared between main and renderer
```

## How it differs from typsmthng (web)

| | Web | Desktop |
|---|---|---|
| Storage | IndexedDB | Native filesystem |
| Project format | Virtual file tree in browser | Regular folders and files |
| Install | PWA or just visit the URL | Standalone app binary |
| Shell | Browser tab | Electrobun (native webview) |
| Extras | Offline/PWA support | Vault system, full-text search, background tasks, auto-update |

## Contributing

1. Create a feature branch.
2. Make your changes.
3. Run:

```bash
bun run typecheck
bun run test
bun run build
```

4. Open a pull request. **(If you're vibecoding, kindly include your prompt as well.)**

## License

No license file is currently included in this repository.
