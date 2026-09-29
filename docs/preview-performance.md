# Preview performance and source navigation

The live preview now keeps a Typst 0.15.1 compiler on a dedicated thread. It
retains fonts and parsed sources between edits. `FileStore::reset()` revalidates
sources and assets on each request, including imports, missing files, and edits
that preserve file size. Typst's memoization can reuse unaffected work. Cache
entries age out after ten requests, including failed compilations.

The implementation follows the persistent-world approach in the local
`~/Code/vibes/typrst/src/compatible.rs`. It uses upstream crates directly so the
app does not depend on another checkout or an external worker binary. It does
not use typrst's experimental independent engine or its font-index optimization.
PDF export, template initialization, and presentation-note queries still use the
existing CLI backend.

Each successful preview retains its document and a snapshot of the source trees.
Single clicks use Typst's `jump_from_click` to resolve file, line, and character
column. When its glyph hit test misses, equation tags and rendered bounds map
the click to the start of the formula. This covers tall symbols, descenders,
generated differential glyphs, and gaps inside a formula. It also supports local
imports and transformed frames, while respecting clipping. Coordinates
account for zoom, centered image fitting, and presentation-note cropping.
Unchanged SVG pages retain their GTK widgets while receiving the latest source
mapping. Editing invalidates navigation until a matching preview arrives.

Diagnostics and error status wait for 900 ms without typing. Successful previews
still use the configured compile delay, which defaults to 100 ms. New edits
cancel pending diagnostic publication. Editor revisions prevent an earlier
compilation from publishing after a newer edit, even before its debounce fires.
The last successful preview remains visible through incomplete expressions.

Compilation requests are coalesced by the application. A superseded in-process
layout finishes before the next request; it is not interrupted midway. Its
result is discarded. Project or compiler-option changes recreate the world,
and completed Google Font downloads explicitly invalidate the font store.

## Measurements

Measured on September 29, 2026, with Rust 1.93.1, an optimized release build,
and Typst 0.15.1. The fixture has ten pages of text and math. Each request changes
a visible number on one page. There is one cold run followed by eleven edits.
Every page's SVG is compared with the CLI output after normalizing whitespace
between tags. These measurements include compilation and SVG output, not the
editor debounce, GTK delivery, image decoding, or painting.

| Environment | CLI cold | Persistent cold | CLI edit median | Persistent edit median | Persistent edit max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Host, system fonts enabled, 1,061 font faces reported by fontconfig | 222.28 ms | 134.34 ms | 222.35 ms | 3.66 ms | 5.19 ms |
| Development container, embedded fonts only | 20.19 ms | 8.61 ms | 19.40 ms | 3.74 ms | 5.11 ms |

The host comparison is about 61 times faster for repeated edits. It does not
reproduce the reported 600 ms on the user's document; that document was not
part of this benchmark. Cold starts, package downloads, document complexity,
and font configuration still affect latency.

## Reproduce

Build in an environment with GTK 4, GtkSourceView 5, and libadwaita development
libraries installed. The benchmark also needs Typst 0.15.1 on PATH, or
`TYPSMTHNG_TYPST` pointing to that executable.

```sh
TYPSMTHNG_BENCH_SYSTEM_FONTS=1 cargo test --locked --release --lib \
  backend::preview::tests::benchmark_incremental_preview_against_cli \
  -- --ignored --nocapture

cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings

dbus-run-session -- xvfb-run -a env GTK_A11Y=none GSK_RENDERER=cairo \
  cargo test --locked --bin typsmthng \
  native_preview_clicks_and_diagnostic_idle_timing \
  -- --ignored --test-threads=1
```

Validation passed with 51 backend tests, 23 UI/model tests, the display-dependent
GTK test, and Clippy. Four manual benchmarks and the GTK test are ignored by
the ordinary suite. The new tests cover imported source jumps, math, rotation,
Unicode columns, unchanged SVG with moved source lines, stable preamble entries,
changed and deleted dependencies, recovery from compiler errors, and project
switching. The GTK test exercises the gesture handler, cursor placement, widget
reuse, and delayed/cancelled diagnostics.

The existing interaction smoke test also passed with twelve rapid edits,
file switches, binary-file preservation, and resizing. The X11 keyboard smoke
passed save/undo/redo, pairing, Vim writes, file-picker cancellation, theme and
settings controls, search, and presentation navigation/drawing.
