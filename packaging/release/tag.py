#!/usr/bin/env python3
"""Prevent a release from publishing packages for a different commit than its tag."""

import argparse
import re
import subprocess


def verify_tag(repository, version, source_sha):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        raise ValueError("Invalid release version")
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("Release source must be a full commit SHA")
    ref = f"refs/tags/v{version}"
    output = subprocess.check_output(["git", "ls-remote", "--tags", repository, ref, ref + "^{}"], text=True)
    refs = dict(line.split()[::-1] for line in output.splitlines())
    if not refs:
        # gh release create --target creates the missing tag at this exact SHA.
        return
    if ref not in refs or set(refs) - {ref, ref + "^{}"}:
        raise ValueError("Unexpected release tag lookup result")
    target = refs.get(ref + "^{}", refs[ref])
    if target != source_sha:
        raise ValueError(f"Existing v{version} tag targets {target}, but verified packages target {source_sha}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-sha", required=True)
    args = parser.parse_args()
    verify_tag("origin", args.version, args.source_sha)


if __name__ == "__main__":
    main()
