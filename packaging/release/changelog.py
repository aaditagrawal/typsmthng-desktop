#!/usr/bin/env python3
"""Maintain versioned changelog entries shared by the app and GitHub releases."""
import argparse
import re
import subprocess
import tomllib
from pathlib import Path

VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")
SLUG = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")


def app_version(root: Path) -> str:
    version = tomllib.loads((root / "native/gtk/Cargo.toml").read_text())["package"]["version"]
    if not isinstance(version, str) or not VERSION.fullmatch(version):
        raise ValueError("Invalid Cargo changelog version")
    return version


def entry_text(path: Path) -> str:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 65536:
        raise ValueError(f"Expected a regular changelog entry under 64 KiB: {path}")
    text = path.read_text(encoding="utf-8").strip()
    lines = [line for line in text.splitlines() if line.strip()]
    if not lines or any(not line.startswith("- ") or len(line[2:].strip()) < 4 for line in lines):
        raise ValueError(f"Changelog entries need complete '- ' bullet lines: {path}")
    if any(line[2:].strip().upper() in {"TODO", "TBD", "PLACEHOLDER", "NO CHANGES"} for line in lines):
        raise ValueError(f"Replace placeholder changelog text: {path}")
    return text


def render(root: Path, version: str) -> str:
    if not VERSION.fullmatch(version):
        raise ValueError("Invalid changelog version")
    parts = []
    legacy = root / "releases" / f"v{version}.md"
    if legacy.is_file():
        text = legacy.read_text(encoding="utf-8").strip()
        if text.startswith("# v"):
            text = text.partition("\n")[2].strip()
        if text:
            parts.append(text)
    for path in sorted((root / "changelog" / version).glob("*.md")):
        if not SLUG.fullmatch(path.stem):
            raise ValueError(f"Use a lowercase hyphenated changelog name: {path}")
        parts.append(entry_text(path))
    if not parts:
        raise ValueError(f"No release notes for v{version}; add changelog/{version}/description.md")
    return f"# v{version}\n\n" + "\n\n".join(parts) + "\n"


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def check(root: Path, base: str | None = None) -> str:
    version = app_version(root)
    for directory in sorted((root / "changelog").iterdir()):
        if directory.is_dir():
            if not VERSION.fullmatch(directory.name):
                raise ValueError(f"Invalid changelog version directory: {directory.name}")
            render(root, directory.name)
    render(root, version)
    if base and set(base) != {"0"}:
        base = git(root, "rev-parse", "--verify", f"{base}^{{commit}}")
        added = git(root, "diff", "--diff-filter=A", "--name-only", f"{base}...HEAD").splitlines()
        prefix = f"changelog/{version}/"
        entries = [name for name in added if name.startswith(prefix) and name.endswith(".md")
                   and Path(name).parent == Path("changelog") / version]
        if not entries:
            raise ValueError(f"Every PR needs a new changelog/{version}/description.md entry. Use the changelog.py add command.")
        tag = subprocess.run(["git", "-C", str(root), "rev-parse", "--verify", f"refs/tags/v{version}^{{commit}}"], capture_output=True, text=True, check=False)
        if tag.returncode == 0:
            released = subprocess.run(["git", "-C", str(root), "merge-base", "--is-ancestor", tag.stdout.strip(), base], check=False)
            if released.returncode == 0:
                raise ValueError(f"v{version} is already released. Bump the Cargo version and add an entry for the next version.")
        for entry in entries:
            entry_text(root / entry)
    return version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    commands = parser.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("check")
    validate.add_argument("--base")
    output = commands.add_parser("render")
    output.add_argument("--version")
    output.add_argument("--output", type=Path)
    add = commands.add_parser("add")
    add.add_argument("slug")
    add.add_argument("--change", action="append", required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    try:
        if args.command == "check":
            print(f"Changelog entries verified for v{check(root, args.base)}")
        elif args.command == "render":
            text = render(root, args.version or app_version(root))
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(text, encoding="utf-8")
            else:
                print(text, end="")
        else:
            if not SLUG.fullmatch(args.slug):
                raise ValueError("Use a lowercase hyphenated entry name")
            path = root / "changelog" / app_version(root) / f"{args.slug}.md"
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("x", encoding="utf-8") as stream:
                stream.write("".join(f"- {line.strip()}\n" for line in args.change))
            try:
                entry_text(path)
            except ValueError:
                path.unlink()
                raise
            print(path.relative_to(root))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
