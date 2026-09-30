# Typesetter comparison and Typst compatibility assessment

Assessed on 2026-09-30. typsmthng source: `bc07145b6c57a37ce6747462f8b22ab84bfe3c6e`, app version 0.1.4. The comparison is with Haydn Trowell's GNOME Typesetter, application ID `net.trowell.typesetter`.

Follow-up implementation resolves the three configuration mismatches identified below. Preview, PDF/SVG export, and speaker-note queries now use a shared environment resolver with explicit-option precedence, signed timestamps, package-cache support, and both font-ignore settings. The app resolves environment fonts before adding its Google Fonts cache. Invalid settings produce consistent errors. All 20 original probes now agree, including 18 identical valid SVG outputs and two matching invalid-document failures. The original findings below and baseline JSON remain historical evidence; the JSON also includes results after the fix.

Run `scripts/test-typst-compatibility.sh` for the permanent headless suite. It selects or downloads the checksum-verified pinned CLI and runs without GTK dependencies. CLI tests fail when Typst is unavailable instead of silently returning. Validation passed 76 tests, including nine compatibility tests covering the environment regressions and representative document cases, with four benchmarks ignored. Follow-up native validation on October 1 used an existing local GTK container image: 100 tests passed, with four benchmarks and three display tests ignored in the ordinary run. Each of those three display tests then passed in its own Xvfb process. Headless/native Clippy and formatting passed. CI now has a separate headless compatibility job alongside the existing native checks. See the [test suite](../native/gtk/tests/typst_compatibility.rs) and [configuration resolver](../native/gtk/src/backend/compile_options.rs).

typsmthng has the broader project and presentation workflow. Typesetter has more assistance for everyday writing and editing. The strongest additions would be semantic completion, hover help, formatting, document navigation, and export-standard selection. Local compatibility testing is worthwhile: the investigation reproduced three disagreements between our preview and export configuration.

Typesetter's [Flathub manifest](https://github.com/flathub/net.trowell.typesetter/blob/master/net.trowell.typesetter.json) currently selects release 0.15.2 at `1271b451fa6d00d50708cf6d9aeaf570a2af79cd`. I inspected that tag and development source at `efc327877dc37a95b03c183fd4d6c3f8af1ab08c`, dated September 27. Development declares version 0.16.0, but I found no corresponding release tag. Development-only features below should not be treated as shipped Flathub features. Both inspected versions use Typst 0.15.1.

The following inventory comes from our source and Typesetter's [release source](https://codeberg.org/haydn/typesetter/src/tag/v0.15.2), with the [Flathub listing](https://flathub.org/en/apps/net.trowell.typesetter) as a public cross-check. It is a source audit, not an interactive evaluation of both applications.

| Capability | typsmthng | Typesetter |
| --- | --- | --- |
| Native local editor | Rust, GTK 4, libadwaita; Linux, macOS, Windows packaging | Rust, GTK 4, libadwaita; official Linux Flatpak |
| Project workflow | Folder tree; create, rename, duplicate, move, Trash; project workspaces and favorites | Release is focused on individual documents and recent files; development adds ZIP-based `.typz` projects |
| Import and portability | Ordinary files and folders; ZIP import/export; Overleaf ZIP and partial LaTeX conversion | Ordinary `.typ` documents; development adds `.typz` archive open/save and explicit entrypoint metadata |
| Live preview | Persistent official compiler; multi-page SVG; zoom, fit width, page navigation; image previews | Persistent official compiler; rendered preview; automatic sizing, manual zoom, magnifier |
| Source navigation | Preview clicks jump to text/math and local imports | Preview-to-source and source-to-preview navigation |
| Editing basics | Syntax coloring, undo, wrapping, line numbers, find/replace, pair completion, Vim-style input | Syntax coloring, find/replace, delimiter handling, centered scrolling |
| Semantic assistance | No semantic autocomplete or Typst function hover help found | Ctrl+Space completion and hover help through `typst-ide` |
| Formatting | No Typst formatter found | `typstyle-core` formatter |
| Writing checks | No spelling/grammar integration found | `libspelling` spelling checks; local Harper grammar checks, English only |
| Document navigation | File tree and project search | Heading outline; development combines navigation with comments and statistics |
| Statistics | No word/character statistics found | Rendered page, word, and character counts |
| Packages | Official package resolution/download/cache; Universe template search/init | Automatic package downloads; local package install/remove interface and cache controls |
| Templates | Built-in starters and live Typst Universe search | Built-in and user-installed templates |
| Fonts | System/embedded fonts; cached Google Fonts discovery; backend accepts explicit font directories | System/embedded fonts; user custom-font directory |
| Document export | PDF; SVG used internally for preview; project ZIP | PDF with version/PDF-A/PDF-UA selection; HTML and Markdown |
| Preview accessibility tools | No color-vision simulation or lightness inversion found | Color-vision simulation and optional lightness inversion; these do not certify PDF accessibility |
| Presentations | Presenter and audience windows, notes, timer, clock, overview, annotations, laser, blackout | Can write slide documents; no equivalent presenter/annotation workflow found |
| App localization | No translation infrastructure found | gettext/Weblate translations |

Our relevant implementations are in [workspace.rs](../native/gtk/src/ui/workspace.rs), [app.rs](../native/gtk/src/ui/app.rs), [presentation.rs](../native/gtk/src/ui/presentation.rs), and the [backend](../native/gtk/src/backend/mod.rs). Typesetter's relevant release modules are `src/editor`, `src/outline.rs`, `src/typst_system/ide.rs`, `src/typst_system/compilation.rs`, and `data/gtk/preferences.ui`.

I would adopt the following ideas in this order. Effort estimates are relative judgments, not delivery commitments.

1. **Semantic completion and hover help.** High daily value, medium effort. We already depend on `typst-ide`; expose its autocomplete and tooltip functions through GtkSourceView. Start with explicit completion, then add automatic suggestions after validating latency and cancellation. Include functions, parameters, symbols, references, and paths where the official API supports them. Our current source-map `Snapshot` only retains source trees and reports no date; expand its file/date support before using it as a general IDE world. Completion also needs the current unsaved editor buffer.
2. **Heading navigation, statistics, and source-to-preview jumps.** High value for long documents, low-to-medium effort. We already retain the compiled document. Add heading queries and cursor navigation without another compilation. Keep source prose counts distinct from rendered counts, which can include repeated headers or generated text. Comment navigation can follow Typesetter's development idea.
3. **Explicit formatting.** Medium effort. Integrate a compatible `typstyle-core` version with one undo transaction and preserve caret/scroll position. Offer formatting as an explicit command first.
4. **PDF standards and additional exports.** Medium effort. Our pinned CLI already exposes PDF standards and HTML, PNG, and SVG export. Add PDF/A and PDF/UA selection before more complex export conversions. Markdown export is an additional conversion feature, not a native Typst CLI format.
5. **Centered scrolling and a preview magnifier.** Small, optional improvements that fit our native UI. We already have zoom and fit-width controls, so their absence is not a reason to redesign preview.
6. **Local spelling/grammar and package management.** Useful later. Grammar can stay local with Harper. Spelling introduces a native library/dictionary packaging task across our three platforms. Package/cache controls would make existing compiler capabilities easier to discover.
7. **Color-vision simulation and custom templates.** Useful follow-ups for slide authors and repeat workflows. Keep document accessibility checks separate from preview color filters.

Preserve folder-based projects and our presentation tools. Typesetter's `.typz` development format is its own project-container proposal, not a Typst language requirement. Optional import/export interoperability could be useful; replacing our project storage is unnecessary. Likewise, keep ordinary documents usable offline while allowing explicit package/font downloads.

Typesetter's source is GPL-3.0-or-later; this project is MIT. The recommendations concern behavior and independently implemented integrations with shared dependencies. They are not a recommendation to copy Typesetter source into this repository.

For Typst compliance, the useful reference is the versioned language/API documentation and the official compiler's regression tests. I did not identify a separate editor certification or a numerical conformance standard. Typst documents [markup, math, and code modes](https://typst.app/docs/reference/syntax/); its [test suite](https://github.com/typst/typst/blob/v0.15.1/tests/README.md) checks evaluation, diagnostics, rendered pages, SVG, PDF tags, and HTML. This assessment does not assign a percentage of specification compliance.

Our preview calls `typst::compile::<PagedDocument>` and `typst-svg` 0.15.1 directly. PDF export uses the official 0.15.1 CLI, and executable detection rejects other versions. The dependencies, lockfile, downloader, and Flatpak compiler source agree on 0.15.1. The language and layout implementation therefore come from upstream, while our application controls files, fonts, package access, dates, inputs, and export options. That application integration is the useful testing target. See [preview.rs](../native/gtk/src/backend/preview.rs), [typst.rs](../native/gtk/src/backend/typst.rs), and [Cargo.toml](../native/gtk/Cargo.toml).

I tested the actual backend source through a temporary Cargo manifest that removed the unused GTK dependencies and binaries. The repository manifest and runtime code were not changed. This environment lacks GTK, GtkSourceView, and libadwaita development libraries, so the full workspace and native UI tests could not run. The official CLI was downloaded through our checksum-verifying packaging script and explicitly supplied using `TYPSMTHNG_TYPST`.

The existing backend suite passed **60 tests, with four ignored benchmarks**. I then ran 20 additional comparative probes against preview, CLI SVG, and CLI PDF. Seventeen agreed: 15 valid documents produced identical SVG text, and two invalid documents produced the expected matching failures. PDF compilation agreed on success/failure for those cases; PDF appearance was not independently rendered or compared.

The probes cover markup, scripting/show rules, complex math, multiple pages, columns/tables, nested imports and root-relative paths, JSON/CSV/YAML/TOML, SVG images/gradients, bibliography, references/outline/counters/metadata, Unicode/RTL, fixed dates, missing symbols/imports, page preambles, explicit local packages, explicit font paths, and a duplicate-font-family case. The custom-font precedence probe passed, and upstream font discovery uses the same system/embedded/explicit-directory ordering as our preview.

Three probes reproduced application compatibility gaps:

| Setting | Preview result | CLI SVG/PDF result | Consequence |
| --- | --- | --- | --- |
| `TYPST_PACKAGE_PATH` points to a valid local package | Package not found | Compiles | A document can export while preview reports an error |
| `TYPST_FONT_PATHS` supplies Geist Mono, with system fonts disabled | Unknown-font warning and fallback | Requested font loads | Preview and export use different glyphs/layout |
| `SOURCE_DATE_EPOCH=946684800`, with no explicit timestamp option | Uses current date; year-2000 assertion fails | Uses year 2000; assertion passes | Date-dependent document content can disagree |

These gaps arise because our CLI subprocess inherits environment variables while the in-process preview only receives `CompileOptions`. They are not evidence that upstream's Typst parser violates its language rules. Resolve effective configuration once and give the same resolved values to both paths. Also audit the analogous package-cache and ignore-font environment settings before claiming complete environment parity; those were not independently probed.

Other gaps are product capabilities rather than language violations. `sys.inputs` is valid Typst but we expose no input configuration, and `Library::default()` provides no external inputs. Syntax highlighting is a small regex definition: it omits math-aware coloring, block/nested comments, and several keywords. Those constructs still compile through the official parser. Our entrypoint resolver prioritizes `main.typ`, which is convenient but insufficient for projects needing an explicit different entrypoint. Custom page-size settings intentionally prepend a wrapper, so compare both paths with identical preambles. Ordinary files default to page size `auto`.

PDF standard compliance is a separate issue. Typst supports [PDF/A output profiles](https://typst.app/docs/reference/pdf/) and writes tagged PDFs by default. PDF/UA requires additional document information and checks. Our UI does not select a standard, so ordinary PDF export is not evidence of PDF/A or PDF/UA conformance. Once we add standard selection, use the compiler's checks and a validator such as veraPDF on representative exports, following Typst's [accessibility guidance](https://typst.app/docs/guides/accessibility/). Color-vision simulation alone does not check document tags, alternative text, or reading order.

**Local compatibility tests are worth maintaining.** This small prototype found three real mismatches with modest runtime cost. Across the 20 final probes, warm preview calls totalled about 34 ms and CLI SVG calls about 299 ms, excluding cold preview initialization, PDF compilation, and build time. These are small-document observations, not a benchmark for long documents.

Start with about 20 to 40 application fixtures on every relevant PR. Require the pinned CLI instead of silently returning when it is unavailable; some current CLI tests have that early-return behavior. Keep the existing source-navigation and stale-dependency tests, then add environment configuration, font fallback/precedence, local and cached packages, dates, nested/binary assets, explicit entrypoints, and representative document/slide fixtures. Freeze time and fonts for deterministic comparisons. Use CLI SVG as the baseline for preview, compare diagnostics/page counts/dimensions with appropriate rounding tolerance, and render selected SVG/PDF pairs to test export appearance. Raw PDF bytes are a poor initial comparison because metadata and encoding can change independently of appearance.

Include a frozen cached Universe package and a WASM plugin fixture in the eventual suite; this investigation did not exercise package network downloads, WASM execution, variable-font axes, or all cross-platform font behavior. Run broader real-world documents and native packaged smoke checks on compiler upgrades and releases. The current CI already downloads the pinned CLI and runs GTK/presentation checks, so extend that workflow rather than creating a disconnected certification process.

Running all upstream compiler tests against an unchanged upstream compiler has limited additional value for this app. The upstream runner uses its own test world, helper functions, fonts, section syntax, and references; executing it alone does not exercise our `World`, subprocess configuration, or UI. Use selected upstream cases as inspiration for app fixtures. Reserve the full suite for upstream/compiler modifications or investigations of a suspected compiler regression.

The best next change is a shared preview/export configuration resolver and regression cases for the three reproduced mismatches. After that, prioritize semantic editing help and document navigation. The observed result is strong compatibility for the exercised Typst 0.15.1 document features, with known environment-integration gaps and untested areas, rather than a claim of complete specification compliance.

Raw final probe results are preserved in [typst-compatibility-2026-09-30.json](assessments/typst-compatibility-2026-09-30.json). The temporary harness, probe source, backend test logs, pinned CLI, and rendered outputs remain at `/tmp/typsmthng-assessment-dbjk3J` for this session.
