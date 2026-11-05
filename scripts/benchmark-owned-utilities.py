"""Compare actual base/candidate utility sources without another checkout.

Run with `uv run --no-project python scripts/benchmark-owned-utilities.py`.
Baseline dependencies live only in a temporary crate. Its lock starts from the
base lock, and Cargo runs offline. JSON reports nanoseconds per operation.
"""

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BASE = "0f6130447b42a2eca8e2d3bdbb3a95c81c5413a2"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, cwd=ROOT, text=True, **kwargs)


MAIN = r"""
#![allow(dead_code)]
mod baseline_error;
mod candidate_error;
mod baseline_update_install;
mod candidate_update_install;
mod encoding;

use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::error::Error;
use std::time::Instant;

fn measure(mut operation: impl FnMut(), count: usize) -> f64 {
    let started = Instant::now();
    for _ in 0..count { operation(); }
    started.elapsed().as_nanos() as f64 / count as f64
}

fn compare(name: &str, count: usize, samples: usize,
    mut baseline: impl FnMut(), mut candidate: impl FnMut()) {
    for _ in 0..2 {
        measure(&mut baseline, count);
        measure(&mut candidate, count);
    }
    let mut old = Vec::new();
    let mut new = Vec::new();
    for sample in 0..samples {
        if sample % 2 == 0 {
            old.push(measure(&mut baseline, count));
            new.push(measure(&mut candidate, count));
        } else {
            new.push(measure(&mut candidate, count));
            old.push(measure(&mut baseline, count));
        }
    }
    let mut old_sorted = old.clone(); old_sorted.sort_by(f64::total_cmp);
    let mut new_sorted = new.clone(); new_sorted.sort_by(f64::total_cmp);
    println!("{{\"workload\":{name:?},\"iterations\":{count},\"baseline_ns\":{old:?},\"candidate_ns\":{new:?},\"baseline_median_ns\":{},\"candidate_median_ns\":{},\"baseline_range_ns\":[{},{}],\"candidate_range_ns\":[{},{}]}}",
        old_sorted[samples / 2], new_sorted[samples / 2],
        old_sorted[0], old_sorted[samples - 1], new_sorted[0], new_sorted[samples - 1]);
}

macro_rules! errors {
    ($module:ident) => {{
        use $module::BackendError as E;
        [E::UnsafePath("../draft.typ".into()),
         E::OutsideProject("/documents/稿.typ".into()),
         E::InvalidTypstConfiguration { setting: "timestamp", reason: "invalid date" },
         E::TypstTimeout,
         E::io("/documents/draft.typ", std::io::Error::other("disk full")),
         E::from(zip::result::ZipError::FileNotFound),
         E::from(notify::Error::generic("watch failed"))]
    }};
}

fn main() {
    let samples: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    for size in [0, 32, 4096] {
        let data: Vec<u8> = (0..size).map(|i| i as u8).collect();
        assert_eq!(hex::encode(&data), encoding::encode_hex(&data));
        compare(&format!("hex/{size}-bytes"), if size < 100 { 200_000 } else { 4_000 }, samples,
            || { black_box(hex::encode(black_box(&data))); },
            || { black_box(encoding::encode_hex(black_box(&data))); });
    }
    let family = "Noto Sans Devanagari";
    compare("font-cache/sha256-and-hex", 100_000, samples,
        || { black_box(hex::encode(Sha256::digest(black_box(family.as_bytes())))); },
        || { black_box(encoding::encode_hex(Sha256::digest(black_box(family.as_bytes())))); });
    let old = errors!(baseline_error);
    let new = errors!(candidate_error);
    for (old, new) in old.iter().zip(&new) {
        assert_eq!(old.to_string(), new.to_string());
    }
    compare("backend-error/display-mixed", 50_000, samples,
        || { for error in &old { black_box(black_box(error).to_string()); } },
        || { for error in &new { black_box(black_box(error).to_string()); } });
    let message = "invalid setting in /documents/稿/".repeat(256);
    let old_large = baseline_error::BackendError::Process(message.clone());
    let new_large = candidate_error::BackendError::Process(message);
    assert_eq!(old_large.to_string(), new_large.to_string());
    compare("backend-error/display-long-unicode", 50_000, samples,
        || { black_box(black_box(&old_large).to_string()); },
        || { black_box(black_box(&new_large).to_string()); });
    compare("backend-error/source-mixed", 200_000, samples,
        || { for error in &old { black_box(black_box(error).source()); } },
        || { for error in &new { black_box(black_box(error).source()); } });
    compare("backend-error/from-notify-zip", 200_000, samples,
        || {
            black_box(baseline_error::BackendError::from(black_box(notify::Error::generic("failed"))));
            black_box(baseline_error::BackendError::from(black_box(zip::result::ZipError::FileNotFound)));
        },
        || {
            black_box(candidate_error::BackendError::from(black_box(notify::Error::generic("failed"))));
            black_box(candidate_error::BackendError::from(black_box(zip::result::ZipError::FileNotFound)));
        });
    let path = std::env::temp_dir().join(format!("typsmthng-desktop-owned-hash-benchmark-{}", std::process::id()));
    for size in [0, 32, 64 * 1024, 4 * 1024 * 1024] {
        std::fs::write(&path, vec![0xa5; size]).unwrap();
        assert_eq!(baseline_update_install::hash_file(&path).unwrap(), candidate_update_install::hash_file(&path).unwrap());
        compare(&format!("updater/hash-file-{size}-bytes"), if size < 100_000 { 500 } else { 20 }, samples,
            || { black_box(baseline_update_install::hash_file(black_box(&path)).unwrap()); },
            || { black_box(candidate_update_install::hash_file(black_box(&path)).unwrap()); });
    }
    std::fs::remove_file(path).unwrap();
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default=DEFAULT_BASE)
    parser.add_argument("--samples", type=int, default=21)
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    if args.samples < 3 or args.samples % 2 == 0:
        parser.error("--samples must be an odd number >= 3")

    def base_source(relative):
        return run("git", "show", f"{args.base}:{relative}", capture_output=True).stdout

    with tempfile.TemporaryDirectory(
        prefix="typsmthng-desktop-owned-utilities-"
    ) as temporary:
        project = Path(temporary)
        source = project / "src"
        source.mkdir()
        manifest = """[package]
name = "typsmthng-utility-benchmark"
version = "0.0.0"
edition = "2021"
[dependencies]
hex = "=0.4.3"
thiserror = "=2.0.21"
notify = "=8.2.1"
zip = { version = "=9.1.3", default-features = false }
serde_json = "=1.0.154"
serde = { version = "=1.0.229", features = ["derive"] }
sha2 = "=0.11.1"
tempfile = "=3.27.0"
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
"""
        (project / "Cargo.toml").write_text(manifest)
        (project / "Cargo.lock").write_text(base_source("Cargo.lock"))
        for name in ["error", "update_install"]:
            relative = f"native/gtk/src/backend/{name}.rs"
            (source / f"baseline_{name}.rs").write_text(base_source(relative))
            shutil.copyfile(ROOT / relative, source / f"candidate_{name}.rs")
        shutil.copyfile(ROOT / "native/gtk/src/encoding.rs", source / "encoding.rs")
        (source / "main.rs").write_text(MAIN)
        env = os.environ.copy()
        env["CARGO_BUILD_JOBS"] = "2"
        # Keep the isolated crate out of the application's build artifacts.
        env["CARGO_TARGET_DIR"] = str(ROOT / "target/owned-utility-benchmark")
        cargo = ["cargo", "--manifest-path", str(project / "Cargo.toml")]
        # Cargo's manifest-path option belongs to the subcommand.
        run(
            "cargo",
            "test",
            *cargo[1:],
            "--offline",
            "--release",
            env=env,
            stdout=sys.stderr,
        )
        if args.check_only:
            return
        run("cargo", "build", *cargo[1:], "--offline", "--release", env=env)
        print(
            json.dumps(
                {
                    "base": args.base,
                    "rustc": run(
                        "rustc", "--version", capture_output=True
                    ).stdout.strip(),
                    "machine": platform.platform(),
                    "cpu_count": os.cpu_count(),
                    "load_average": os.getloadavg()
                    if hasattr(os, "getloadavg")
                    else None,
                    "profile": "release, thin LTO, one codegen unit, two build jobs",
                    "samples": args.samples,
                    "ordering": "two warmups, alternating baseline/candidate per sample",
                    "units": "nanoseconds per operation; display-mixed formats seven errors",
                }
            ),
            flush=True,
        )
        executable = (
            Path(env["CARGO_TARGET_DIR"]) / "release/typsmthng-utility-benchmark"
        )
        if os.name == "nt":
            executable = executable.with_suffix(".exe")
        run(str(executable), str(args.samples), env=env)


if __name__ == "__main__":
    main()
