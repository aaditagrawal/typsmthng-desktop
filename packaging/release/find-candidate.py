#!/usr/bin/env python3
"""Find a successful, compatible main-branch candidate, waiting for active builds."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from release_artifacts import check_metadata, expected_metadata


WORKFLOW = ".github/workflows/release-candidate.yml"


def api(path, **parameters):
    command = ["gh", "api", path, "--method", "GET"]
    for name, value in parameters.items():
        command.extend(["-f", f"{name}={value}"])
    result = subprocess.run(command, text=True, capture_output=True)
    if result.returncode:
        if "HTTP 404" in result.stderr:
            return None
        raise RuntimeError(result.stderr.strip())
    return json.loads(result.stdout)


def trusted_run(run, repository, sha, workflow_id):
    return (
        run["workflow_id"] == workflow_id
        and run["head_sha"] == sha
        and run["head_branch"] == "main"
        and run["event"] in {"push", "workflow_dispatch"}
        and (run.get("repository") or {}).get("full_name") == repository
        and (run.get("head_repository") or {}).get("full_name") == repository
        and run["path"] == WORKFLOW
    )


def compatible_artifact(repository, run_id, expected):
    artifacts = api(f"repos/{repository}/actions/runs/{run_id}/artifacts")
    if artifacts is None:
        return False
    available = {asset["name"]: asset for asset in artifacts["artifacts"] if not asset["expired"]}
    if not {"verified-release", "release-candidate-metadata"}.issubset(available):
        return False
    with tempfile.TemporaryDirectory() as directory:
        result = subprocess.run([
            "gh", "run", "download", str(run_id), "--repo", repository,
            "--name", "release-candidate-metadata", "--dir", directory,
        ], text=True, capture_output=True)
        if result.returncode:
            print(f"Candidate {run_id} metadata is unavailable; rebuilding: {result.stderr.strip()}")
            return False
        try:
            check_metadata(Path(directory) / "release-metadata.json", expected)
        except (ValueError, OSError) as error:
            print(f"Candidate {run_id} is incompatible: {error}")
            return False
    return True


def find(args):
    workflow = api(f"repos/{args.repository}/actions/workflows/release-candidate.yml")
    if workflow is None or workflow["state"] != "active" or workflow["path"] != WORKFLOW:
        print("Candidate workflow is unavailable; building this release")
        return ""
    deadline = time.monotonic() + args.wait_seconds
    rejected = set()
    while True:
        result = api(
            f"repos/{args.repository}/actions/workflows/{workflow['id']}/runs",
            head_sha=args.source_sha, branch="main", per_page="100",
        )
        runs = result["workflow_runs"] if result else []
        active = False
        for run in runs:
            run_id = str(run["id"])
            if run_id in rejected or not trusted_run(run, args.repository, args.source_sha, workflow["id"]):
                continue
            if run["status"] != "completed":
                active = True
                continue
            rejected.add(run_id)
            if run["conclusion"] != "success":
                continue
            args.run_id = run_id
            if compatible_artifact(args.repository, run_id, expected_metadata(args)):
                print(f"Reusing verified release candidate from run {run_id}")
                return run_id
        remaining = deadline - time.monotonic()
        if not active or remaining <= 0:
            print("No successful compatible candidate is available; building this release")
            return ""
        print("Matching candidate is still building; waiting before deciding to rebuild", flush=True)
        time.sleep(min(args.poll_seconds, remaining))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--signing", required=True, choices=["unsigned", "signed"])
    parser.add_argument("--rust-toolchain", required=True)
    parser.add_argument("--wait-seconds", type=int, default=2700)
    parser.add_argument("--poll-seconds", type=int, default=30)
    args = parser.parse_args()
    args.run_id = "0"
    expected_metadata(args)
    if args.wait_seconds < 0 or args.poll_seconds <= 0:
        parser.error("Wait must be nonnegative and polling must be positive")
    run_id = find(args)
    with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
        stream.write(f"run_id={run_id}\n")


if __name__ == "__main__":
    main()
