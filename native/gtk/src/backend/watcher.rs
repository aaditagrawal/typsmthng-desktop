use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend::error::{BackendError, Result};
use crate::backend::paths::relative_path_from_root;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFingerprint {
    pub size_bytes: u64,
    pub modified_ns: u128,
    pub sha256: Option<String>,
}

impl FileFingerprint {
    pub fn capture(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).map_err(|error| BackendError::io(path, error))?;
        let sha256 = if metadata.is_file() && metadata.len() <= 8 * 1024 * 1024 {
            let bytes = fs::read(path).map_err(|error| BackendError::io(path, error))?;
            Some(format!("{:x}", Sha256::digest(bytes)))
        } else {
            None
        };
        Ok(Self {
            size_bytes: metadata.len(),
            modified_ns: metadata
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            sha256,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalEventKind {
    Added,
    Changed,
    Removed,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEvent {
    pub kind: ExternalEventKind,
    pub path: String,
    pub renamed_to: Option<String>,
    pub is_directory: bool,
    pub fingerprint: Option<FileFingerprint>,
}

pub struct ExternalWatcher {
    root: PathBuf,
    _watcher: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    suppressed: HashMap<String, (FileFingerprint, SystemTime)>,
}

impl ExternalWatcher {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|error| BackendError::io(root.as_ref(), error))?;
        let (sender, events) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;
        Ok(Self {
            root,
            _watcher: watcher,
            events,
            suppressed: HashMap::new(),
        })
    }

    pub fn suppress_own_write(&mut self, relative: &str, for_duration: Duration) -> Result<()> {
        let path = self
            .root
            .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let fingerprint = FileFingerprint::capture(path)?;
        self.suppressed.insert(
            relative.into(),
            (fingerprint, SystemTime::now() + for_duration),
        );
        Ok(())
    }

    pub fn drain(&mut self) -> Result<Vec<ExternalEvent>> {
        let mut output = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            let event = event?;
            let kind = match event.kind {
                EventKind::Create(CreateKind::Any | CreateKind::File | CreateKind::Folder) => {
                    ExternalEventKind::Added
                }
                EventKind::Modify(ModifyKind::Name(
                    RenameMode::Any | RenameMode::Both | RenameMode::From | RenameMode::To,
                )) => ExternalEventKind::Renamed,
                EventKind::Modify(_) => ExternalEventKind::Changed,
                EventKind::Remove(RemoveKind::Any | RemoveKind::File | RemoveKind::Folder) => {
                    ExternalEventKind::Removed
                }
                _ => continue,
            };
            if kind == ExternalEventKind::Renamed && event.paths.len() == 2 {
                let from = visible_relative_path(&self.root, &event.paths[0]);
                let to = visible_relative_path(&self.root, &event.paths[1]);
                let (kind, path, renamed_to) = match (from, to) {
                    (Some(from), Some(to)) => (ExternalEventKind::Renamed, from, Some(to)),
                    (Some(from), None) => (ExternalEventKind::Removed, from, None),
                    (None, Some(to)) => (ExternalEventKind::Added, to, None),
                    (None, None) => continue,
                };
                output.push(ExternalEvent {
                    kind,
                    path,
                    renamed_to,
                    is_directory: false,
                    fingerprint: None,
                });
                continue;
            }
            for path in event.paths {
                let Some(relative) = visible_relative_path(&self.root, &path) else {
                    continue;
                };
                output.push(ExternalEvent {
                    kind,
                    path: relative.clone(),
                    renamed_to: None,
                    is_directory: false,
                    fingerprint: None,
                });
            }
        }
        // Coalesce before touching disk: a save can produce many notifications,
        // but each changed file needs only one content hash of its final state.
        let output = coalesce(output)
            .into_iter()
            .filter_map(|mut event| {
                let path = self
                    .root
                    .join(event.renamed_to.as_deref().unwrap_or(&event.path));
                event.is_directory = path.is_dir();
                event.fingerprint = FileFingerprint::capture(&path).ok();
                let own_write = matches!(
                    event.kind,
                    ExternalEventKind::Added | ExternalEventKind::Changed
                ) && self.is_suppressed(&event.path, event.fingerprint.as_ref());
                (!own_write).then_some(event)
            })
            .collect();
        Ok(output)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn is_suppressed(&mut self, path: &str, fingerprint: Option<&FileFingerprint>) -> bool {
        let now = SystemTime::now();
        self.suppressed.retain(|_, (_, expiry)| *expiry > now);
        self.suppressed
            .get(path)
            .is_some_and(|(expected, _)| fingerprint == Some(expected))
    }
}

fn visible_relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = relative_path_from_root(root, path)?;
    let preview_wrapper = relative
        .rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with(".typsmthng-preview-") && name.ends_with(".typ"));
    (!preview_wrapper).then_some(relative)
}

fn coalesce(events: Vec<ExternalEvent>) -> Vec<ExternalEvent> {
    let mut by_path = HashMap::new();
    for event in events {
        by_path.insert(event.path.clone(), event);
    }
    let mut events = by_path.into_values().collect::<Vec<_>>();
    events.sort_by(|left, right| left.path.cmp(&right.path));
    events
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn paired_renames_crossing_project_boundary_are_not_lost() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("project");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let outside = directory.path().canonicalize().unwrap().join("outside.typ");
        let inside = root.join("inside.typ");
        fs::write(&inside, "imported").unwrap();
        let mut watcher = ExternalWatcher::new(&root).unwrap();
        let (sender, receiver) = mpsc::channel();
        watcher.events = receiver;
        let rename = |from: PathBuf, to: PathBuf| {
            notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(from)
                .add_path(to)
        };
        sender
            .send(Ok(rename(outside.clone(), inside.clone())))
            .unwrap();
        let added = watcher.drain().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].kind, ExternalEventKind::Added);
        assert_eq!(added[0].path, "inside.typ");
        assert!(added[0].fingerprint.is_some());
        fs::rename(&inside, &outside).unwrap();
        sender.send(Ok(rename(inside.clone(), outside))).unwrap();
        let removed = watcher.drain().unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].kind, ExternalEventKind::Removed);
        assert_eq!(removed[0].path, "inside.typ");
        assert!(removed[0].renamed_to.is_none());
        assert!(removed[0].fingerprint.is_none());
    }

    #[test]
    fn internal_directory_rename_reads_destination_metadata() {
        let directory = tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::create_dir(root.join("before")).unwrap();
        let mut watcher = ExternalWatcher::new(&root).unwrap();
        let (sender, receiver) = mpsc::channel();
        watcher.events = receiver;
        fs::rename(root.join("before"), root.join("after")).unwrap();
        sender
            .send(Ok(notify::Event::new(EventKind::Modify(ModifyKind::Name(
                RenameMode::Both,
            )))
            .add_path(root.join("before"))
            .add_path(root.join("after"))))
            .unwrap();
        let events = watcher.drain().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ExternalEventKind::Renamed);
        assert_eq!(events[0].renamed_to.as_deref(), Some("after"));
        assert!(events[0].is_directory);
    }

    #[test]
    fn repeated_save_notifications_preserve_external_change_detection() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("main.typ");
        fs::write(&path, "original").unwrap();
        let path = path.canonicalize().unwrap();
        let mut watcher = ExternalWatcher::new(directory.path()).unwrap();
        // Use a deterministic notification stream rather than platform watcher timing.
        let (sender, receiver) = mpsc::channel();
        watcher.events = receiver;
        watcher
            .suppress_own_write("main.typ", Duration::from_secs(2))
            .unwrap();
        for _ in 0..100 {
            sender
                .send(Ok(
                    notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path.clone())
                ))
                .unwrap();
        }
        assert!(watcher.drain().unwrap().is_empty());
        fs::write(&path, "external").unwrap();
        for _ in 0..100 {
            sender
                .send(Ok(
                    notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path.clone())
                ))
                .unwrap();
        }
        let events = watcher.drain().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, "main.typ");
        assert_eq!(
            events[0].fingerprint,
            Some(FileFingerprint::capture(&path).unwrap())
        );
    }

    #[test]
    fn fingerprint_notices_same_length_rewrites() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("main.typ");
        fs::write(&path, "abc").unwrap();
        let first = FileFingerprint::capture(&path).unwrap();
        fs::write(&path, "xyz").unwrap();
        let second = FileFingerprint::capture(&path).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn coalesces_repeated_events_by_path() {
        let events = vec![
            ExternalEvent {
                kind: ExternalEventKind::Added,
                path: "a.typ".into(),
                renamed_to: None,
                is_directory: false,
                fingerprint: None,
            },
            ExternalEvent {
                kind: ExternalEventKind::Changed,
                path: "a.typ".into(),
                renamed_to: None,
                is_directory: false,
                fingerprint: None,
            },
        ];
        let merged = coalesce(events);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].kind, ExternalEventKind::Changed);
    }
}
