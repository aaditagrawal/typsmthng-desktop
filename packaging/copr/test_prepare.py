#!/usr/bin/env python3
"""Check source RPM metadata and dependency license retention."""
from contextlib import redirect_stdout
import io
from pathlib import Path
import tempfile
import unittest

from prepare import prepare


class PreparationTests(unittest.TestCase):
    def test_version_toolchain_and_notices_follow_the_archived_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "native/gtk").mkdir(parents=True)
            (root / "packaging/copr").mkdir(parents=True)
            (root / "native/gtk/Cargo.toml").write_text('[package]\nversion = "0.1.7"\n')
            (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.93.1"\n')
            (root / "packaging/copr/typsmthng.spec.in").write_text('@VERSION@\n@RUST_VERSION@\n@LICENSES@\n')
            vendor = root / "vendor/demo-1.0"
            (vendor / "nested").mkdir(parents=True)
            (vendor / "Cargo.toml").write_text('[package]\nname = "demo"\nversion = "1.0"\nlicense = "MIT/Apache-2.0"\n')
            (vendor / "nested/LICENSE").write_text("License notice\n")
            captured = io.StringIO()
            with redirect_stdout(captured):
                prepare(root, root / "vendor", root)
            self.assertEqual(captured.getvalue(), "0.1.7\n")
            spec = (root / "typsmthng.spec").read_text()
            self.assertIn("0.1.7\n1.93.1", spec)
            self.assertIn("MIT OR Apache-2.0", spec)
            self.assertEqual((root / "bundled-licenses/demo-1.0/nested/LICENSE").read_text(), "License notice\n")
            self.assertIn("demo\t1.0\tMIT OR Apache-2.0", (root / "bundled-licenses/manifest.tsv").read_text())


if __name__ == "__main__":
    unittest.main()
