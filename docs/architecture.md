# Native app modules

The GTK controller coordinates project selection, background work, and user feedback. Filesystem and document rules belong in the backend so they can be checked without a display.

| Module | Responsibility | Interface |
| --- | --- | --- |
| `backend::project` | Project files, atomic writes, and path safety | `Project` operations |
| `backend::imports` | Copying imports, retaining original LaTeX, conversion, and collision-safe names | The existing import functions and shared path allocator |
| `backend::notes` | Inline/comment fallback and Markdown sidecar loading and saving | `load` and `save` |
| `backend::rendered_preview` | SVG cache ownership and presentation-note cropping | `prepare_preview` and its owned result |
| `backend::compile_options` | Environment defaults and explicit-setting precedence | `CompileOptions::resolved` |
| `backend::editor` | UTF-8 offsets, completion snippets, and explicit formatting | Editor requests/results and text helpers |
| `backend::preview` | Persistent Typst world, semantic requests, compiled headings/statistics, and source maps | `PreviewCompiler` and immutable `SourceMap` |
| `backend::typst` | Pinned CLI invocation, export profiles, page archives, and atomic saves | `TypstTool` and `ExportArtifact` |
| `backend::app_fonts` | Process-private GTK/Pango font registration | Desktop feature only |
| `ui::workspace` | Editor and preview widgets, edit revisions, and callbacks | `WorkspaceView` |
| `ui::editor_tools` | Completion, hover, formatting, undo, and stale-result rejection | Editor actions using the shared worker |
| `ui::controls` | Optional cursor centering and a cached-image preview lens | View controls with weak widget references |
| `ui::app` | Selection, scheduling, dialogs, and results | Controller actions |

Imports and notes share the `Project` seam for filesystem access. Their implementations keep path validation and atomic writes in that module. The UI chooses destinations and presents errors; it does not parse note sidecars or implement recursive import rules.

Keep GTK/Pango modules and display-dependent integration tests behind the `desktop` feature. Pure backend behavior belongs in the headless suite. Native tests that initialize GTK run in separate processes, because serial Rust tests still use different threads.

Before adding a module, check its existing callers. A useful extraction improves locality by keeping a rule and its tests together. Prefer an existing concrete interface over a new adapter for a single implementation. Delete unused helpers and avoid retaining pass-through wrappers solely for a possible future caller.

Run backend checks with `scripts/test-typst-compatibility.sh`. With GTK development libraries available, run the ordinary workspace tests and Clippy, then the display checks listed in `.github/workflows/ci.yml`.
