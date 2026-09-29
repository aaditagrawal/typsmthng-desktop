#!/usr/bin/env python3
"""Create and verify the exact, versioned set of distributable packages."""

import argparse
import hashlib
import json
from pathlib import Path
import re


METADATA = "release-metadata.json"


def asset_names(version):
    return sorted([
        f"typsmthng_{version}_amd64.deb",
        f"typsmthng_{version}_x86_64.rpm",
        f"typsmthng-{version}-linux-x64.AppImage",
        f"typsmthng_{version}_linux_x64.flatpak",
        f"typsmthng-{version}-macos-arm64.dmg",
        f"typsmthng-{version}-macos-x64.dmg",
        "typsmthng-windows-x64.exe",
    ])


def expected_metadata(args):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", args.version):
        raise ValueError("Invalid release version")
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_sha):
        raise ValueError("Release source must be a full commit SHA")
    if not re.fullmatch(r"[0-9]+", args.run_id):
        raise ValueError("Invalid build run ID")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", args.rust_toolchain):
        raise ValueError("Release toolchain must be pinned")
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", args.repository):
        raise ValueError("Invalid repository")
    return {
        "schema_version": 1,
        "version": args.version,
        "source_sha": args.source_sha,
        "macos_signing": args.signing,
        "repository": args.repository,
        "run_id": args.run_id,
        "rust_toolchain": args.rust_toolchain,
    }


def check_metadata(path, expected):
    if path.is_symlink() or not path.is_file():
        raise ValueError("Missing regular release metadata file")
    if json.loads(path.read_text()) != expected:
        raise ValueError("Release metadata does not match the requested source, version, signing mode, repository, run, or toolchain")


def digest(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
        raise ValueError(f"Missing or empty regular release asset: {path.name}")
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def verify(directory, expected):
    names = asset_names(expected["version"])
    actual = {path.name for path in directory.iterdir()}
    if actual != set(names + ["SHA256SUMS", METADATA]):
        raise ValueError(f"Unexpected release artifact set: {sorted(actual)}")
    check_metadata(directory / METADATA, expected)
    sums = directory / "SHA256SUMS"
    if sums.is_symlink() or not sums.is_file():
        raise ValueError("Missing regular checksum file")
    lines = sums.read_text().splitlines()
    checksums = {}
    for line in lines:
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if not match or match[2] not in names or match[2] in checksums:
            raise ValueError("Invalid, duplicate, or unexpected checksum entry")
        checksums[match[2]] = match[1]
    if set(checksums) != set(names):
        raise ValueError("Incomplete release checksums")
    for name in names:
        if digest(directory / name) != checksums[name]:
            raise ValueError(f"Checksum mismatch: {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["assemble", "verify", "metadata"])
    parser.add_argument("path", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--signing", required=True, choices=["unsigned", "signed"])
    parser.add_argument("--repository", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--rust-toolchain", required=True)
    args = parser.parse_args()
    expected = expected_metadata(args)
    if args.operation == "metadata":
        check_metadata(args.path, expected)
        return
    if args.operation == "assemble":
        names = asset_names(args.version)
        if {path.name for path in args.path.iterdir()} != set(names):
            raise ValueError("Build must contain exactly the seven expected installers")
        checksums = "".join(f"{digest(args.path / name)}  {name}\n" for name in names)
        (args.path / "SHA256SUMS").write_text(checksums)
        (args.path / METADATA).write_text(json.dumps(expected, indent=2) + "\n")
    verify(args.path, expected)
    print(f"Verified seven {args.version} installers from {args.source_sha}")


if __name__ == "__main__":
    main()
