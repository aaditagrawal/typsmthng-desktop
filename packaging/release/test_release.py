#!/usr/bin/env python3
"""Exercise release promotion with corrupted artifacts and untrusted candidates."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from release_artifacts import METADATA, asset_names, expected_metadata, main, verify
from tag import verify_tag

spec = importlib.util.spec_from_file_location("find_candidate", Path(__file__).with_name("find-candidate.py"))
candidate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(candidate)


def arguments():
    return argparse.Namespace(
        version="0.2.0", source_sha="a" * 40, signing="unsigned",
        repository="owner/project", run_id="123", rust_toolchain="1.93.1",
        wait_seconds=0, poll_seconds=1,
    )


class ArtifactVerification(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.args = arguments()
        for name in asset_names(self.args.version):
            (self.directory / name).write_bytes(f"{name} package payload".encode())
        flags = ["assemble", str(self.directory), "--version", self.args.version,
                 "--source-sha", self.args.source_sha, "--signing", self.args.signing,
                 "--repository", self.args.repository, "--run-id", self.args.run_id,
                 "--rust-toolchain", self.args.rust_toolchain]
        with patch("sys.argv", ["release_artifacts.py", *flags]):
            main()
        self.expected = expected_metadata(self.args)

    def test_complete_release_and_signed_release(self):
        verify(self.directory, self.expected)
        self.expected["macos_signing"] = "signed"
        (self.directory / METADATA).write_text(json.dumps(self.expected))
        verify(self.directory, self.expected)

    def test_metadata_requires_exact_source_and_build_identity(self):
        for field, value in [("source_sha", "b" * 40), ("version", "0.2.1"),
                             ("macos_signing", "signed"), ("repository", "elsewhere/project"),
                             ("run_id", "456"), ("rust_toolchain", "1.94.0")]:
            with self.subTest(field=field):
                changed = dict(self.expected, **{field: value})
                (self.directory / METADATA).write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    verify(self.directory, self.expected)
        (self.directory / METADATA).write_text(json.dumps(self.expected))

    def test_modified_package_fails_checksum(self):
        (self.directory / "typsmthng-windows-x64.exe").write_bytes(b"altered")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
            verify(self.directory, self.expected)

    def test_missing_and_unexpected_files_are_rejected(self):
        unexpected = self.directory / "stale-installer.exe"
        unexpected.write_bytes(b"stale")
        with self.assertRaisesRegex(ValueError, "artifact set"):
            verify(self.directory, self.expected)
        unexpected.unlink()
        (self.directory / "typsmthng-windows-x64.exe").unlink()
        with self.assertRaisesRegex(ValueError, "artifact set"):
            verify(self.directory, self.expected)

    def test_symlink_and_empty_packages_are_rejected(self):
        asset = self.directory / "typsmthng-windows-x64.exe"
        asset.write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "Missing or empty"):
            verify(self.directory, self.expected)
        asset.unlink()
        asset.symlink_to(self.directory / asset_names(self.args.version)[0])
        with self.assertRaisesRegex(ValueError, "regular release asset"):
            verify(self.directory, self.expected)

    def test_duplicate_and_traversal_checksums_are_rejected(self):
        sums = self.directory / "SHA256SUMS"
        original = sums.read_text()
        for content in [original + original.splitlines()[0] + "\n",
                        original.replace("typsmthng-windows-x64.exe", "../typsmthng-windows-x64.exe")]:
            with self.subTest(content=content):
                sums.write_text(content)
                with self.assertRaisesRegex(ValueError, "checksum entry"):
                    verify(self.directory, self.expected)


class CandidateSelection(unittest.TestCase):
    def setUp(self):
        self.args = arguments()
        self.workflow = {"id": 7, "state": "active", "path": candidate.WORKFLOW}
        self.run = {
            "id": 123, "workflow_id": 7, "head_sha": self.args.source_sha,
            "head_branch": "main", "event": "push", "repository": {"full_name": self.args.repository},
            "head_repository": {"full_name": self.args.repository}, "path": candidate.WORKFLOW,
            "status": "completed", "conclusion": "success",
        }

    def find(self, runs, compatible=True):
        with patch.object(candidate, "api", side_effect=[self.workflow, {"workflow_runs": runs}]), \
                patch.object(candidate, "compatible_artifact", return_value=compatible):
            return candidate.find(self.args)

    def test_matching_success_is_reused(self):
        self.assertEqual(self.find([self.run]), "123")

    def test_untrusted_and_failed_runs_are_not_reused(self):
        cases = [("head_sha", "b" * 40), ("head_branch", "feature"),
                 ("event", "pull_request"), ("path", ".github/workflows/ci.yml"),
                 ("workflow_id", 8), ("conclusion", "failure"),
                 ("head_repository", {"full_name": "fork/project"})]
        for key, value in cases:
            with self.subTest(key=key):
                run = dict(self.run, **{key: value})
                self.assertEqual(self.find([run]), "")

    def test_missing_disabled_and_incompatible_candidates_fall_back(self):
        self.assertEqual(self.find([]), "")
        self.assertEqual(self.find([self.run], compatible=False), "")
        with patch.object(candidate, "api", return_value=None):
            self.assertEqual(candidate.find(self.args), "")
        workflow = dict(self.workflow, state="disabled_manually")
        with patch.object(candidate, "api", return_value=workflow):
            self.assertEqual(candidate.find(self.args), "")

    def test_active_candidate_is_waited_for_and_reused(self):
        self.args.wait_seconds = 10
        active = dict(self.run, status="in_progress", conclusion=None)
        with patch.object(candidate, "api", side_effect=[self.workflow,
                {"workflow_runs": [active]}, {"workflow_runs": [self.run]}]), \
                patch.object(candidate, "compatible_artifact", return_value=True), \
                patch.object(candidate.time, "sleep") as sleep:
            self.assertEqual(candidate.find(self.args), "123")
            sleep.assert_called_once()

    def test_active_candidate_timeout_falls_back(self):
        active = dict(self.run, status="queued", conclusion=None)
        self.assertEqual(self.find([active]), "")

    def test_expired_artifact_is_not_reused(self):
        assets = {"artifacts": [{"name": "verified-release", "expired": True},
                                 {"name": "release-candidate-metadata", "expired": False}]}
        with patch.object(candidate, "api", return_value=assets):
            self.assertFalse(candidate.compatible_artifact(self.args.repository, "123", expected_metadata(self.args)))

    def test_downloaded_metadata_must_match_signing_and_version(self):
        available = {"artifacts": [{"name": "verified-release", "expired": False},
                                    {"name": "release-candidate-metadata", "expired": False}]}
        def download(command, **kwargs):
            directory = Path(command[command.index("--dir") + 1])
            value = expected_metadata(self.args)
            value["macos_signing"] = "signed"
            (directory / METADATA).write_text(json.dumps(value))
            return subprocess.CompletedProcess(command, 0, "", "")
        with patch.object(candidate, "api", return_value=available), \
                patch.object(candidate.subprocess, "run", side_effect=download):
            self.assertFalse(candidate.compatible_artifact(self.args.repository, "123", expected_metadata(self.args)))


class OlderSourceRef(unittest.TestCase):
    def test_workflow_helper_can_parse_an_older_tree_without_its_own_helpers(self):
        helper = Path(__file__).with_name("source.py").resolve()
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            (source / "native/gtk").mkdir(parents=True)
            (source / "native/gtk/Cargo.toml").write_text('[package]\nname = "test"\nversion = "0.2.0-rc.1+build.2"\n')
            (source / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.93.1"\n')
            subprocess.run(["git", "init", "--quiet", str(source)], check=True)
            subprocess.run(["git", "-C", str(source), "add", "."], check=True)
            subprocess.run(["git", "-C", str(source), "-c", "user.name=Release Tests",
                            "-c", "user.email=release-tests@example.invalid",
                            "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "Older release source"], check=True)
            sha = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
            output = source / "outputs"
            environment = dict(os.environ, GITHUB_OUTPUT=str(output), RAW_VERSION="v0.2.0-rc.1+build.2",
                               REQUESTED_SIGNING="signed", EXPECTED_SOURCE_SHA=sha)
            result = subprocess.run(["uv", "run", "--no-project", str(helper)], cwd=source,
                                    env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            values = dict(line.split("=", 1) for line in output.read_text().splitlines())
            self.assertEqual(values["source_sha"], sha)
            self.assertEqual(values["version"], "0.2.0-rc.1+build.2")
            self.assertEqual(values["prerelease"], "true")
            self.assertEqual(values["macos_signing"], "signed")
            self.assertFalse((source / "packaging/release/source.py").exists())
            for field, bad in [("EXPECTED_SOURCE_SHA", "b" * 40), ("RAW_VERSION", "0.2.1"),
                               ("REQUESTED_SIGNING", "unvalidated")]:
                with self.subTest(field=field):
                    result = subprocess.run(["uv", "run", "--no-project", str(helper)], cwd=source,
                                            env=dict(environment, **{field: bad}), capture_output=True)
                    self.assertNotEqual(result.returncode, 0)


class TagSourceIdentity(unittest.TestCase):
    def test_lightweight_annotated_and_missing_tags(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            subprocess.run(["git", "init", "--quiet", str(source)], check=True)
            for number in [1, 2]:
                (source / "file").write_text(str(number))
                subprocess.run(["git", "-C", str(source), "add", "."], check=True)
                subprocess.run(["git", "-C", str(source), "-c", "user.name=Release Tests",
                                "-c", "user.email=release-tests@example.invalid",
                                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", str(number)], check=True)
                if number == 1:
                    first = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
            second = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
            subprocess.run(["git", "-C", str(source), "tag", "v0.2.0", first], check=True)
            subprocess.run(["git", "-C", str(source), "-c", "user.name=Release Tests",
                            "-c", "user.email=release-tests@example.invalid", "-c", "tag.gpgsign=false",
                            "tag", "-a", "v0.2.1", "-m", "Annotated", first], check=True)
            for version in ["0.2.0", "0.2.1"]:
                with self.subTest(version=version):
                    verify_tag(str(source), version, first)
                    with self.assertRaisesRegex(ValueError, "verified packages"):
                        verify_tag(str(source), version, second)
            verify_tag(str(source), "0.2.2", second)


if __name__ == "__main__":
    unittest.main()
