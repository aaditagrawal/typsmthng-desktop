#!/usr/bin/env python3
"""Exercise changelog rendering and the actual PR Git comparison."""
import subprocess
import tempfile
import unittest
from pathlib import Path

from changelog import check, render


class ChangelogTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "native/gtk").mkdir(parents=True)
        (self.root / "native/gtk/Cargo.toml").write_text('[package]\nversion = "0.2.0"\n')
        (self.root / "releases").mkdir()
        self.entry("old", "- Existing change.\n")
        self.git("init", "--quiet")
        self.git("config", "user.name", "Changelog tests")
        self.git("config", "user.email", "tests@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.commit()
        self.base = self.git("rev-parse", "HEAD")

    def git(self, *arguments):
        return subprocess.check_output(["git", "-C", str(self.root), *arguments], text=True).strip()

    def commit(self):
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Fixture change")

    def entry(self, slug, text, version="0.2.0"):
        path = self.root / "changelog" / version / f"{slug}.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def test_legacy_and_new_notes_render_once_in_stable_order(self):
        (self.root / "releases/v0.2.0.md").write_text("# v0.2.0\n\nLegacy change.\n")
        self.entry("z-last", "- Last café change.\n")
        self.entry("a-first", "- First new change.\n")
        self.assertEqual(render(self.root, "0.2.0"), "# v0.2.0\n\nLegacy change.\n\n- First new change.\n\n- Existing change.\n\n- Last café change.\n")

    def test_old_entry_and_old_version_entry_do_not_satisfy_pr(self):
        with self.assertRaisesRegex(ValueError, "Every PR"):
            check(self.root, self.base)
        self.entry("old", "- Editing a previous entry.\n")
        self.entry("different-version", "- An older release change.\n", "0.1.9")
        self.commit()
        with self.assertRaisesRegex(ValueError, "Every PR"):
            check(self.root, self.base)
        self.entry("new-change", "- A complete new change.\n")
        self.commit()
        self.assertEqual(check(self.root, self.base), "0.2.0")

    def test_nested_entry_is_not_silently_excluded_from_the_app(self):
        self.entry("nested/change", "- A change outside the bundled directory.\n")
        self.commit()
        with self.assertRaisesRegex(ValueError, "Every PR"):
            check(self.root, self.base)

    def test_released_version_requires_bump(self):
        self.git("tag", "v0.2.0")
        self.entry("new-change", "- A new change.\n")
        self.commit()
        with self.assertRaisesRegex(ValueError, "already released"):
            check(self.root, self.base)
        (self.root / "native/gtk/Cargo.toml").write_text('[package]\nversion = "0.2.1"\n')
        self.entry("new-change", "- Next version change.\n", "0.2.1")
        self.commit()
        self.assertEqual(check(self.root, self.base), "0.2.1")
        self.assertNotIn("Existing change", render(self.root, "0.2.1"))

    def test_published_tag_at_head_does_not_break_rerunning_its_ci(self):
        self.entry("new-change", "- A new change.\n")
        self.commit()
        self.git("tag", "v0.2.0")
        self.assertEqual(check(self.root, self.base), "0.2.0")

    def test_empty_placeholder_and_missing_notes_are_rejected(self):
        for text in ["", "- TODO\n", "- TBD\n", "Unstructured prose\n", "- Fix\n"]:
            with self.subTest(text=text):
                self.entry("old", text)
                with self.assertRaises(ValueError):
                    render(self.root, "0.2.0")
        with self.assertRaisesRegex(ValueError, "No release notes"):
            render(self.root, "0.3.0")

    def test_add_command_preserves_existing_entries(self):
        script = Path(__file__).with_name("changelog.py")
        command = ["uv", "run", "--no-project", str(script), "--root", str(self.root), "add", "fresh", "--change", "Create a useful change."]
        subprocess.run(command, check=True, capture_output=True)
        path = self.root / "changelog/0.2.0/fresh.md"
        original = path.read_text()
        self.assertNotEqual(subprocess.run(command, capture_output=True, check=False).returncode, 0)
        self.assertEqual(path.read_text(), original)


if __name__ == "__main__":
    unittest.main()
