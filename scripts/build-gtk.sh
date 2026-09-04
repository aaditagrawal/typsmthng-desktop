#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

scripts/check-gtk-prereqs.sh
cargo build --locked --release --manifest-path native/gtk/Cargo.toml "$@"

echo "Built $repo_root/target/release/typsmthng"
