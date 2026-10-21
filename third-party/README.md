# Dependency maintenance backports

The Syntect, Hayagriva, and Biblatex directories are complete, checksum-verified crates.io release snapshots, including their
licenses, assets, examples, and tests. `maintenance.json` records their archive
URLs and SHA-256 checksums. The adjacent `*-maintenance.patch` files are the
complete differences from those releases. No package version is changed. Workspace exclusions keep these external
dependencies outside the application workspace, as they were before the backport.

Run `uv run --no-project python scripts/verify-maintenance-vendor.py` to verify
every file against the release archive and its patch. It reuses Cargo's archive
cache when available and otherwise downloads over verified HTTPS. It fails on
unrecorded changes or extra/missing files. Nested release lockfiles are retained
as provenance; the workspace's root `Cargo.lock` controls application builds.

- Syntect 5.3.0 adopts the maintained `yaml-rust2` parser using the same change as
  [upstream commit af7d1a8](https://github.com/trishume/syntect/commit/af7d1a88cce6680ed99a2623197cc7fa779f30e4).
  The backport uses `yaml-rust2` 0.13.0, checked against the same compatibility
  tests. Its dump module backports the `DumpError` abstraction from
  [upstream PR 694 at 509deb8](https://github.com/trishume/syntect/pull/694), then
  uses [Bincode-Next 3.1.1](https://github.com/Apich-Organization/bincode) with its
  bincode 1 legacy configuration. The proposed `serde-wincode` codec rejected
  strings above 4 MiB. Bincode-Next preserves that existing cache support without
  disabling an allocation guard. A 5 MiB regression case covers it.
  It additionally propagates writer flush/compression-finalization failures and
  spells out one parser lifetime for the pinned Rust toolchain. Serialization
  layouts and bundled dumps are unchanged. Newer Syntect revisions erase the
  YAML error type that Typst 0.15.1 needs for diagnostic positions, so the whole
  newer revision cannot be used directly. The application tests load a cache
  produced by unmodified Syntect 5.3.0/bincode 1.3.3, compare its replacement
  serialization byte for byte, load bundled syntax/theme dumps, and exercise
  runtime grammar loading, diagnostic positions, and IO failures.
- Biblatex 0.12.0 and Hayagriva 0.10.1 change only their `paste` dependency to
  [pastey 0.2.3](https://github.com/as1100k/pastey/tree/4f134ba98ddd307ff7920d3701eed9522976a1b8), under the existing
  import name. Pastey is an actively developed successor, not a renamed copy.
  [Biblatex PR 96](https://github.com/typst/biblatex/pull/96) proposes this same
  migration, but its old base predates the release used here. Keeping these
  release snapshots avoids reverting bibliography fixes or altering their APIs.

The maintained replacements are real dependencies in `Cargo.lock`: `pastey`,
`yaml-rust2`, and `bincode-next`. The retired `paste`, `yaml-rust`, and
`bincode` implementations are absent from the application dependency graph.
There are no advisory ignores.

Remove these snapshots when compatible upstream releases include the changes.
`rustybuzz` and `ttf-parser` remain separate compiler dependencies; these
backports do not claim to resolve their maintenance notices.

The application-owned [serde-yaml-compat adapter](serde-yaml-compat/README.md)
re-exports the maintained upstream `yaml_serde 0.10.7` release and its
`libyaml-rs 0.3.0` backend for both Typst and Hayagriva. It contains no copied
parser implementation. The deprecated wrapper and `unsafe-libyaml` packages
are absent from the effective graph. Its README explains the honest local
compatibility version, maintenance evidence, unchanged unsafe parser algorithms,
licenses, and frozen legacy behavior tests. This directory is local adapter
code, separate from the three release snapshots verified by maintenance.json.
