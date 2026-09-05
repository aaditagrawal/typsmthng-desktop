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
            let rename_destination = (kind == ExternalEventKind::Renamed && event.paths.len() == 2)
                .then(|| relative_path_from_root(&self.root, &event.paths[1]))
                .flatten();
            for (index, path) in event.paths.into_iter().enumerate() {
                if kind == ExternalEventKind::Renamed && rename_destination.is_some() && index > 0 {
                    continue;
                }
                let Some(relative) = relative_path_from_root(&self.root, &path) else {
                    continue;
                };
                if relative.rsplit('/').next().is_some_and(|name| {
                    name.starts_with(".typsmthng-preview-") && name.ends_with(".typ")
                }) {
                    continue;
                }
                let metadata = fs::metadata(&path).ok();
                let fingerprint = FileFingerprint::capture(&path).ok();
                if self.is_suppressed(&relative, fingerprint.as_ref()) {
                    continue;
                }
                output.push(ExternalEvent {
                    kind,
                    path: relative.clone(),
                    renamed_to: rename_destination.clone(),
                    is_directory: metadata.as_ref().is_some_and(fs::Metadata::is_dir),
                    fingerprint,
                });
            }
        }
        Ok(coalesce(output))
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
