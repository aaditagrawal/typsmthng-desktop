#![cfg(target_os = "linux")]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use typsmthng_gtk::backend::update_install::{hash_file, InstallJob};

#[test]
fn helper_waits_for_exit_then_replaces_and_relaunches() {
    let dir = tempfile::tempdir().unwrap();
    let artifact = dir.path().join("new.AppImage");
    let target = dir.path().join("installed.AppImage");
    let marker = dir.path().join("started");
    std::fs::write(
        &artifact,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::write(&target, "old").unwrap();
    let job = InstallJob {
        sha256: hash_file(&artifact).unwrap(),
        artifact,
        target: target.clone(),
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_typsmthng-updater"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "{}", serde_json::to_string(&job).unwrap()).unwrap();
    let mut reply = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut reply)
        .unwrap();
    assert_eq!(reply.trim(), "ready");
    writeln!(input, "install").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    assert!(child.try_wait().unwrap().is_none());
    drop(input);
    assert!(child.wait().unwrap().success());
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(marker.exists(), "new app was not relaunched");
    assert_eq!(hash_file(&target).unwrap(), job.sha256);
}

#[test]
fn corrupt_artifact_never_replaces_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let artifact = dir.path().join("new.AppImage");
    let target = dir.path().join("installed.AppImage");
    std::fs::write(&artifact, "corrupt").unwrap();
    std::fs::write(&target, "old").unwrap();
    let job = InstallJob {
        sha256: "0".repeat(64),
        artifact,
        target: target.clone(),
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_typsmthng-updater"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        serde_json::to_string(&job).unwrap()
    )
    .unwrap();
    assert!(!child.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "old");
}
