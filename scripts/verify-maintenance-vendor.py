#!/usr/bin/env -S uv run --no-project python
"""Verify each vendored release against its registry archive plus the recorded patch."""

import hashlib
import io
import json
import os
import subprocess
import tarfile
import tempfile
import urllib.request
from pathlib import Path


def main():
    vendor = Path(__file__).resolve().parents[1] / "third-party"
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    for record in json.loads((vendor / "maintenance.json").read_text()):
        name, version = record["name"], record["version"]
        archives = list(
            (cargo_home / "registry" / "cache").glob(f"*/{name}-{version}.crate")
        )
        if archives:
            data = archives[0].read_bytes()
        else:
            with urllib.request.urlopen(record["url"], timeout=60) as response:
                data = response.read()
        if hashlib.sha256(data).hexdigest() != record["sha256"]:
            raise ValueError(f"{name}: registry archive checksum mismatch")
        with tempfile.TemporaryDirectory() as directory:
            expected = Path(directory)
            with tarfile.open(fileobj=io.BytesIO(data)) as archive:
                for member in archive.getmembers():
                    parts = Path(member.name).parts
                    if len(parts) > 1:
                        member.name = str(Path(*parts[1:]))
                        archive.extract(member, expected, filter="data")
            subprocess.run(
                [
                    "patch",
                    "--batch",
                    "--fuzz=0",
                    "-p1",
                    "-i",
                    str(vendor / record["patch"]),
                ],
                cwd=expected,
                check=True,
            )
            actual = vendor / name
            expected_files = {
                p.relative_to(expected) for p in expected.rglob("*") if p.is_file()
            }
            actual_files = {
                p.relative_to(actual) for p in actual.rglob("*") if p.is_file()
            }
            if expected_files != actual_files:
                raise ValueError(f"{name}: file inventory mismatch")
            for relative in sorted(expected_files):
                if (expected / relative).read_bytes() != (
                    actual / relative
                ).read_bytes():
                    raise ValueError(f"{name}: unexpected change in {relative}")
        print(
            f"{name} {version}: verified release checksum and complete maintenance patch"
        )


if __name__ == "__main__":
    main()
