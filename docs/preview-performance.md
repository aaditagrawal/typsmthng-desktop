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

## Display resolution and memory

Pages render at their allocated size multiplied by the display scale. Two
background workers rasterize visible pages and pages within 64 logical pixels
of the viewport. Requests replace queued work for the same page; obsolete
results are discarded. Leaving that region releases the page texture. The
queue holds at most 64 requests, and an image is capped at 32 megapixels.

Reading numeric SVG header dimensions avoids decoding every page on GTK's
thread. Unchanged pages retain their cached image through partial recompiles.
Changed pages retain the previous image until new pixels arrive, and source
clicks wait for those pixels so they cannot navigate using a mismatched layout.

Measured on September 30, 2026, with Rust 1.93.1, GTK 4.14.5, optimized release
builds, the Cairo renderer, a 1920-pixel logical window, and `GDK_SCALE=2`.
The fixture contains twenty A4 pages of text and math. Both builds display the
first viewport for three seconds. Values are median peak process RSS from
three runs; they include the compiler and the whole application.

| Measurement | Main (`10bddc8`, v0.1.3) | Preview fix (v0.1.4) |
| --- | ---: | ---: |
| Peak process RSS | 522.3 MiB | 318.7 MiB |

The reduction is 39%. This measures the initial viewport, rather than scrolling
through the whole document. The GTK regression separately scrolls twenty pages
and checks texture eviction, nearby-page caching, zoom, display scale changes,
page reuse, stale requests, and image fallback behavior.

```sh
for scale in 1 2; do
  dbus-run-session -- xvfb-run -a env GTK_A11Y=none GSK_RENDERER=cairo \
    GDK_SCALE="$scale" cargo test --locked --bin typsmthng \
    native_svg_pages_follow_zoom_and_display_scale \
    -- --ignored --test-threads=1
done
```

## Direct Typst rendering

Compiled editor pages now render directly from the immutable Typst page, using
`typst-render` 0.15.1. Its glyph rasterizer retains subpixel placement and caches
glyph coverage between edits. The preview still produces SVG for page identity,
hyperlinks, exports, and presentation. Imported SVGs and cropped presentation
pages continue to use the platform SVG loader.

The renderer uses the surface's actual fractional display scale, rather than
the widget's rounded integer scale factor. At 125%, for example, a 480-pixel
page requests 600 device pixels instead of 960. Current textures draw at their
exact device dimensions, clipped to the fitted page bounds. Previous textures
still stretch smoothly during an asynchronous resize. Regular GTK texture nodes
preserve HiDPI detail in both GPU and Cairo rendering; the explicit scaling-filter
node was rejected after physical screenshots exposed a Cairo resolution loss.

The two workers, coalescing queue, stale-result checks, nearby-page prefetch,
offscreen texture eviction, and image size limits remain in place. Page clones
share the existing compiled frames, fonts, and assets. Pixel buffers transfer
into GTK without a second pixel copy, with premultiplied alpha preserved.

Measured October 1, 2026, in the Ubuntu 24.04 development container with Rust
1.93.1 and an optimized release test binary. The fixture is an A4 page with
30 repeated text-and-math paragraphs. Eleven edits change a visible heading
number. These timings measure rasterization only, excluding compilation,
debounce, texture upload, and GTK painting. The redraw column measures eleven
additional rasterizations of identical content; actual unchanged pages reuse
their texture without rasterizing at all.

| Scale | Renderer | Cold raster | Redraw median | Edit median | Edit max | Pixel buffer |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1× | Platform SVG | 27.15 ms | 22.60 ms | 22.88 ms | 34.10 ms | 3.40 MiB |
| 1× | Direct Typst | 2.78 ms | 0.42 ms | 0.47 ms | 0.51 ms | 3.40 MiB |
| 2× | Platform SVG | 51.19 ms | 34.28 ms | 34.87 ms | 42.18 ms | 13.61 MiB |
| 2× | Direct Typst | 10.92 ms | 1.71 ms | 1.83 ms | 2.10 ms | 13.61 MiB |

For a separate twenty-page text-and-math fixture, median peak process RSS over
three runs was 323.0 MiB before and 319.6 MiB after. Both were debug application
builds, using a 1920-pixel logical window, `GDK_SCALE=2`, Cairo, and a three-second
initial viewport. This does not measure memory after scrolling the whole document.

Physical 2× screenshots show [the previous preview](screenshots/gtk4/preview-native-before.png)
and [the direct preview](screenshots/gtk4/preview-native-after.png) at the same
zoom. They are crops of actual window pixels, with no rescaling.

Reproduce the raster benchmark:

```sh
TYPSMTHNG_RASTER_ARTIFACT_DIR=build/renderer-raster \
  cargo test --locked --release --bin typsmthng \
  benchmark_preview_rasterization -- --ignored --nocapture
```

Validation passed with 145 ordinary tests, Clippy, and formatting. The GTK page
regression passed at 1× and 2× and covers native rendering after compiler/cache
disposal, texture reuse, resize fallbacks, cancellation, and eviction. Other
display tests passed source clicks, cursor navigation, headings, cropped pages,
delayed diagnostics, and the cached magnifier. The interaction and keyboard
smokes passed rapid edits, save/undo/redo, file switching, binary preservation,
resize, and presentation controls. Fractional sizes have numerical regression
coverage; a physical fractional-scale Wayland session was not available here.

## Preview quality

Settings offers Standard, High, and Ultra. Standard retains the native display
resolution. High and Ultra rasterize with up to two or three samples per axis,
then average coverage into the device-sized texture before GTK paints it. This
refines fine lines, curves, and diagonal text without increasing the cached
texture size. Premultiplied channels preserve transparent edges. Intermediate
rasters keep the 32-megapixel and 16,384-pixel dimension limits; large pages or
high display scales reduce the sample count to stay within those limits.

Changing quality cancels stale jobs and retains the previous pixels until the
replacement arrives. The two workers, queue coalescing, and offscreen eviction
apply to all quality levels. Standard remains the default for fast live edits.

Measured October 2, 2026, in the Ubuntu 24.04 development container, Rust 1.93.1,
with an optimized release test. The fixture contains 7-point text, mathematics,
rotated text, a 0.2-point line, and a circle. Times are the median of five warm
rasterizations, including downsampling, excluding GTK upload. Mean squared
pixel error compares each device-sized output with an eight-sample reference;
lower error means coverage closer to that reference. It measures this fixture,
not perceived contrast or legibility across all documents and displays.

| Output pixels | Quality | Warm raster | Mean squared error |
| --- | --- | ---: | ---: |
| 480 × 320 | Standard | 0.24 ms | 3.0720 |
| 480 × 320 | High | 1.87 ms | 1.1484 |
| 480 × 320 | Ultra | 3.05 ms | 0.6971 |
| 960 × 640 | Standard | 0.51 ms | 2.6016 |
| 960 × 640 | High | 7.08 ms | 0.8035 |
| 960 × 640 | Ultra | 12.63 ms | 0.3994 |

```sh
TYPSMTHNG_QUALITY_ARTIFACT_DIR=build/preview-quality \
  cargo test --locked --release --bin typsmthng \
  compare_preview_quality -- --ignored --nocapture
```

Fit text width now scales the full page by its measured text width, preserving
page dimensions and margins. It centers the page horizontally and allows
sideways panning. Source navigation and the magnifier use the same full-page
coordinates. The display regressions cover centering, panning, source clicks,
and quality switching at 1× and 2×. An optional geometry comparison recreates
the earlier cropped sizing with the current renderer, then captures the full
page and the original left margin, isolating the zoom change:

```sh
dbus-run-session -- xvfb-run -a -s '-screen 0 3840x2400x24' \
  env GTK_A11Y=none GSK_RENDERER=cairo \
  TYPSMTHNG_FIT_TEXT_ARTIFACT_DIR=build/fit-text \
  cargo test --locked --bin typsmthng \
  native_preview_clicks_and_diagnostic_idle_timing \
  -- --ignored --test-threads=1
```
