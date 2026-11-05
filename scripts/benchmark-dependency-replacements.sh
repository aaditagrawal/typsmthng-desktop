#!/usr/bin/env bash
# Oracle-only dependencies live in a temporary crate, outside the application.
set -euo pipefail
if [[ "$(uname -s)" != Linux ]]; then
  echo "These replacements are measured and enabled only on Linux." >&2
  exit 1
fi
project_root=$(git rev-parse --show-toplevel)
baseline=0f6130447b42a2eca8e2d3bdbb3a95c81c5413a2
bench_dir=$(mktemp -d "${TMPDIR:-/tmp}/typsmthng-dependency-bench.XXXXXX")
trap 'rm -rf "$bench_dir"' EXIT
mkdir -p "$bench_dir/src"
git -C "$project_root" show "$baseline:Cargo.lock" > "$bench_dir/Cargo.lock"
cp "$project_root/native/gtk/src/backend/platform_paths.rs" "$bench_dir/src/platform_paths.rs"
cp "$project_root/scripts/benchmark-dependency-replacements.rs" "$bench_dir/src/main.rs"
cat > "$bench_dir/Cargo.toml" <<'TOML'
[package]
name = "typsmthng-gtk"
version = "0.1.12"
edition = "2021"
[dependencies]
directories = "=6.0.0"
libc = "=0.2.191"
TOML
printf 'Baseline: %s\nCandidate: %s + working diff\n' "$baseline" "$(git -C "$project_root" rev-parse HEAD)"
rustc --version
uname -sm
printf 'Host load: '
cat /proc/loadavg
# Reuse compiled dependencies while each invocation gets fresh inputs.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$project_root/target}/dependency-benchmark"
cargo test --offline --manifest-path "$bench_dir/Cargo.toml" -j 2 -- --test-threads=1
cargo build --offline --release --manifest-path "$bench_dir/Cargo.toml" -j 2
if command -v taskset >/dev/null; then
  allowed_cpus=$(awk '/Cpus_allowed_list/ { print $2 }' /proc/self/status)
  bench_cpu="${allowed_cpus%%[-,]*}"
  printf 'Benchmark affinity: CPU %s\n' "$bench_cpu"
  taskset -c "$bench_cpu" "$CARGO_TARGET_DIR/release/typsmthng-gtk"
else
  "$CARGO_TARGET_DIR/release/typsmthng-gtk"
fi
