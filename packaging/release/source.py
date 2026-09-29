#!/usr/bin/env python3
"""Resolve the checked-out source into immutable release workflow outputs."""

import os
from pathlib import Path
import re
import subprocess
import tomllib


version = os.environ.get("RAW_VERSION", "").removeprefix("v")
manifest = tomllib.loads(Path("native/gtk/Cargo.toml").read_text())["package"]["version"]
version = version or manifest
if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
    raise SystemExit("Invalid release version")
if version != manifest:
    raise SystemExit(f"Cargo version {manifest} != {version}")
signing = os.environ.get("REQUESTED_SIGNING") or "unsigned"
if signing not in {"unsigned", "signed"}:
    raise SystemExit("Invalid macOS signing mode")
toolchain = tomllib.loads(Path("rust-toolchain.toml").read_text())["toolchain"]["channel"]
if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", toolchain):
    raise SystemExit("Release Rust toolchain must be pinned")
sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
requested_sha = os.environ.get("EXPECTED_SOURCE_SHA", sha)
if not re.fullmatch(r"[0-9a-f]{40}", requested_sha) or requested_sha != sha:
    raise SystemExit("Checked out source does not match the requested immutable SHA")
outputs = {
    "version": version,
    "prerelease": str("-" in version.split("+", 1)[0]).lower(),
    "macos_signing": signing,
    "source_sha": sha,
    "rust_toolchain": toolchain,
}
with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
    stream.writelines(f"{key}={value}\n" for key, value in outputs.items())
