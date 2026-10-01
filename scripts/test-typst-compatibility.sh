#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
required_version="0.15.1"
version_matches() {
  case "$1" in
    "typst $required_version"|"typst $required_version "*) return 0 ;;
    *) return 1 ;;
  esac
}

if [[ -n "${TYPSMTHNG_TYPST:-}" ]]; then
  compiler="$TYPSMTHNG_TYPST"
else
  compiler="$(command -v typst || true)"
  if [[ -z "$compiler" ]] || ! version_matches "$("$compiler" --version 2>/dev/null || true)"; then
    case "$(uname -s):$(uname -m)" in
      Linux:x86_64) triple=x86_64-unknown-linux-musl; executable=typst ;;
      Darwin:arm64|Darwin:aarch64) triple=aarch64-apple-darwin; executable=typst ;;
      Darwin:x86_64) triple=x86_64-apple-darwin; executable=typst ;;
      MINGW*:x86_64|MSYS*:x86_64) triple=x86_64-pc-windows-msvc; executable=typst.exe ;;
      *) echo "Install Typst $required_version and set TYPSMTHNG_TYPST on this platform." >&2; exit 1 ;;
    esac
    tools_target="${CARGO_TARGET_DIR:-$repo_root/target}"
    compiler="$tools_target/tools/typst/$triple/$executable"
    if [[ ! -x "$compiler" ]] || ! version_matches "$("$compiler" --version 2>/dev/null || true)"; then
      packaging/download-typst.sh "$triple" "$(dirname "$compiler")"
    fi
  fi
fi

compiler_version="$("$compiler" --version)"
if ! version_matches "$compiler_version"; then
  echo "Tests require Typst $required_version; found $compiler_version at $compiler." >&2
  exit 1
fi
# Cargo runs package tests from native/gtk, not the repository root.
compiler="$(command -v "$compiler")"
compiler="$(cd "$(dirname "$compiler")" && pwd)/$(basename "$compiler")"
export TYPSMTHNG_TYPST="$compiler"
echo "Using $compiler_version at $compiler"
cargo test --locked --workspace --no-default-features --all-targets "$@"
