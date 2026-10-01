#!/usr/bin/env python3
"""Prepare COPR metadata and preserve vendored dependency license notices."""
import argparse
from pathlib import Path
import shutil
import tomllib


def prepare(source: Path, vendor: Path, output: Path):
    package = tomllib.loads((source / "native/gtk/Cargo.toml").read_text())["package"]
    toolchain = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    notices = output / "bundled-licenses"
    notices.mkdir()
    licenses = {"MIT", "Apache-2.0"}
    inventory = []
    for manifest in sorted(vendor.glob("*/Cargo.toml")):
        dependency = tomllib.loads(manifest.read_text())["package"]
        license_id = dependency.get("license", "").replace("/", " OR ")
        if not license_id or any(char in license_id for char in "\r\n@"):
            raise ValueError(f"Missing or invalid license expression: {manifest}")
        licenses.add(license_id)
        inventory.append(f"{dependency['name']}\t{dependency['version']}\t{license_id}\n")
        destination = notices / manifest.parent.name
        destination.mkdir()
        for path in manifest.parent.rglob("*"):
            if path.is_file() and path.name.lower().startswith(("license", "licence", "copying", "notice")):
                target = destination / path.relative_to(manifest.parent)
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, target)
    (notices / "manifest.tsv").write_text("".join(inventory))
    template = (source / "packaging/copr/typsmthng.spec.in").read_text()
    spec = template.replace("@VERSION@", package["version"]).replace("@RUST_VERSION@", toolchain)
    spec = spec.replace("@LICENSES@", " AND ".join(f"({item})" for item in sorted(licenses)))
    (output / "typsmthng.spec").write_text(spec)
    print(package["version"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("vendor", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    prepare(args.source, args.vendor, args.output)
