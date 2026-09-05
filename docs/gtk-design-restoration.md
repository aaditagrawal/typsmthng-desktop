# GTK design restoration

The reference is the original desktop UI immediately before merge commit `06b64083` (PR #37), especially `src/mainview/index.css` and the home, workspace, and presentation components. This work preserves the GTK runtime and the existing uncommitted feature work.

## Restored interface

The app uses the original orange accent, compact monospace chrome, neutral light/dark backgrounds, the bundled application icon, centered home workspace picker, two-column project cards, and dashed creation action. The editor has a compact toolbar, file sidebar, balanced editor/preview split, preview controls above the document, and a status strip. GtkSourceView explicitly selects a matching syntax theme. Transient settings/search windows inherit the selected theme.

GTK provides native controls, focus, selection, text editing, and layout. The home scrolls on short windows; the file sidebar collapses below 1000px and remains available through its toolbar toggle. Presenter controls use two compact rows. System monospace fallback replaces Geist Mono when that font is unavailable. Native translucency remains an approximation of the original web-backed material.

## Performance changes

- Project search runs on a background worker with one active search and one replaceable pending request. Input is debounced; stale replies cannot replace current results.
- File menus are built when opened. Identical file-tree refreshes avoid reconstructing rows.
- Identical compiled SVG decks reuse decoded images and widgets when source navigation mapping is unchanged. Old temporary files can be removed without invalidating the displayed paintables.
- Preview cache writing and notes splitting run on the compiler worker.
- Already-saved buffers avoid redundant atomic writes and watcher notifications.
- Downloaded Google Font families have persistent manifests. Warm preparation avoids network requests; failed families retry after a delay. Regexes and HTTP connections are reused.
- Typst executable version detection is cached and invalidated when the executable changes.
- Ignored dependency/build directories are pruned before traversal; watcher notifications are coalesced before content hashing.
- Editor styling reuses one CSS provider. Search/replace uses a single Unicode offset pass.

The compiler still starts a full Typst compilation for each accepted preview request, but obsolete processes are cancelled. Inline note queries run only during presentation. Font downloads run separately from preview and trigger a refresh when ready. Large documents, initial SVG decoding, and filesystem latency can still affect time to preview. These changes do not implement an incremental Typst compiler or establish a universal keystroke-latency guarantee.

## Correctness checks

Regression tests cover cached fonts, retry behavior, watcher suppression versus external edits, notes splitting and cache lifetime, Unicode search offsets, and hidden-file search behavior. Queue handling discards a previous file's pending source on navigation, reads the live buffer when draining queued work, releases completed timeout IDs, and recovers if a compiler worker disconnects.

The native interaction smoke test makes twelve Unicode edits while requesting compilation, opens a binary asset during queued compilation, returns to the document, and asserts that the asset bytes and final text survive and that all three pages compile. Its project and application state are temporary.

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release
target/release/typsmthng --interaction-smoke-test
cargo test -p typsmthng-gtk --lib benchmark_ -- --ignored --nocapture
```

## Native visual review

Smoke modes use temporary settings and history, leaving the user's saved state alone. Optional snapshots are generated directly from GTK's rendered widget tree, without a browser.

```sh
TYPSMTHNG_SMOKE_THEME=dark \
TYPSMTHNG_SNAPSHOT_DIR=build/design-review/home-dark \
  target/release/typsmthng --smoke-test

TYPSMTHNG_SMOKE_THEME=light \
TYPSMTHNG_SNAPSHOT_DIR=build/design-review/presentation-light \
  target/release/typsmthng --presentation-smoke-test native/gtk/tests/fixtures/demo
```

`TYPSMTHNG_SMOKE_WIDTH=760` exercises the minimum width. `TYPSMTHNG_SMOKE_PROJECTS=3` populates the home with temporary sample projects. `TYPSMTHNG_SMOKE_VIEW=settings` or `search` opens those windows during ordinary smoke mode. Snapshot numbering follows GTK's window list and includes all visible windows. The local review artifacts are under `build/design-review/` and are intentionally not tracked.

Adwaita icons are a runtime dependency. The macOS installation instructions and CI now install `adwaita-icon-theme`, matching the existing bundler's required icon directory. Other platform packaging and release signing still require their normal CI/release verification.

## Local measurements

Measured on this Apple Silicon macOS development machine with Typst 0.15.1. These are reproducible sample measurements, not cross-platform guarantees or a whole-app before/after comparison.

| Check | Result |
| --- | --- |
| Ten-page SVG compile, ten runs | 58.35 ms median, 65.02 ms maximum |
| 100 warm font preparations | 19.76 ms total |
| 1,000 SVG root-header parses | 5.34 ms versus 650.68 ms recreating the previous regex |
| Optimized home startup to smoke-ready callback | 340–361 ms in the sampled runs |
| Twelve-edit/file-switch smoke | 825 ms total; 172 ms maximum interval for a 25 ms GTK timer |
| Release executable | 5.5 MB, excluding GTK and Typst runtime libraries |

The timer interval includes main-loop scheduling and rendering during initial document load and is not a direct input-latency measurement. It shows that occasional UI stalls remain; the app should not be described as maintaining a fixed frame-time budget on all documents.

Validation completed locally: 60 regular tests, three manual benchmarks, Clippy with warnings denied, formatting, release compilation, native light/dark home and workspace snapshots, populated home, settings, search, and presenter/audience windows. No Windows/Linux release artifact was built or signed in this session.

## v0.2.0 follow-up

The release follow-up removes speaker-note queries and font downloads from the editing critical path, cancels obsolete compiler processes, and reuses individual unchanged preview pages. Native surface resize events replace unreliable window-size notifications. Selection deletion, LaTeX verbatim/path conversion, boundary-crossing renames, and partial project export are corrected.

Validation includes 71 regular tests, native wide/narrow/wide resizing, a presenter assertion for metadata notes loaded on demand, and a compiler-wrapper check proving ordinary editing invokes no query process while presentation invokes one. Full cross-platform package builds are release gates. macOS artifacts are explicitly unsigned by Developer ID and not notarized, as selected for v0.2.0.
