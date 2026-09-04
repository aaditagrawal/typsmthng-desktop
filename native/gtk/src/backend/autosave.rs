use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::backend::error::{BackendError, Result};
use crate::backend::paths::normalize_relative_path;
use crate::backend::project::Project;
use crate::backend::watcher::FileFingerprint;

pub const DEFAULT_SAVE_DEBOUNCE: Duration = Duration::from_millis(450);

#[derive(Debug, Clone)]
pub struct PendingWrite {
    pub path: String,
    pub content: String,
    pub staged_at: Instant,
    pub disk_baseline: Option<FileFingerprint>,
}

#[derive(Debug)]
pub struct AutosaveQueue {
    debounce: Duration,
    writes: BTreeMap<String, PendingWrite>,
}

impl Default for AutosaveQueue {
    fn default() -> Self {
        Self::new(DEFAULT_SAVE_DEBOUNCE)
    }
}

impl AutosaveQueue {
    pub fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            writes: BTreeMap::new(),
        }
    }

    pub fn stage(
        &mut self,
        project: &Project,
        path: impl AsRef<Path>,
        content: String,
    ) -> Result<()> {
        let path = normalize_relative_path(path)?;
        let baseline = self
            .writes
            .get(&path)
            .and_then(|write| write.disk_baseline.clone())
            .or_else(|| {
                FileFingerprint::capture(
                    project
                        .root()
                        .join(path.replace('/', std::path::MAIN_SEPARATOR_STR)),
                )
                .ok()
            });
        self.writes.insert(
            path.clone(),
            PendingWrite {
                path,
                content,
                staged_at: Instant::now(),
                disk_baseline: baseline,
            },
        );
        Ok(())
    }

    pub fn pending_content(&self, path: &str) -> Option<&str> {
        let path = normalize_relative_path(path).ok()?;
        self.writes.get(&path).map(|write| write.content.as_str())
    }

    pub fn is_dirty(&self) -> bool {
        !self.writes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.writes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    pub fn flush_due(&mut self, project: &Project) -> Result<Vec<String>> {
        let now = Instant::now();
        let paths = self
            .writes
            .iter()
            .filter(|(_, write)| now.duration_since(write.staged_at) >= self.debounce)
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for path in &paths {
            self.flush(project, path, false)?;
        }
        Ok(paths)
    }

    pub fn flush_all(&mut self, project: &Project) -> Result<()> {
        let paths = self.writes.keys().cloned().collect::<Vec<_>>();
        for path in paths {
            self.flush(project, &path, false)?;
        }
        Ok(())
    }

    pub fn flush(&mut self, project: &Project, path: &str, force: bool) -> Result<()> {
        let path = normalize_relative_path(path)?;
        let Some(write) = self.writes.get(&path) else {
            return Ok(());
        };
        if !force {
            let absolute = project
                .root()
                .join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let current = FileFingerprint::capture(&absolute).ok();
            if current != write.disk_baseline {
                return Err(BackendError::Process(format!(
                    "{path} changed on disk after editing began"
                )));
            }
        }
        project.write_text_atomic(&path, &write.content)?;
        self.writes.remove(&path);
        Ok(())
    }

    pub fn discard(&mut self, path: &str) -> Result<Option<PendingWrite>> {
        Ok(self.writes.remove(&normalize_relative_path(path)?))
    }

    pub fn rename_path(&mut self, old: &str, new: &str) -> Result<()> {
        let old = normalize_relative_path(old)?;
        let new = normalize_relative_path(new)?;
        let child_prefix = format!("{old}/");
        let migrations = self
            .writes
            .keys()
            .filter_map(|path| {
                if path == &old {
                    Some((path.clone(), new.clone()))
                } else {
                    path.strip_prefix(&child_prefix)
                        .map(|suffix| (path.clone(), format!("{new}/{suffix}")))
                }
            })
            .collect::<Vec<_>>();
        for (source, target) in migrations {
            if let Some(mut write) = self.writes.remove(&source) {
                write.path = target.clone();
                self.writes.insert(target, write);
            }
        }
        Ok(())
    }

    pub fn remove_path(&mut self, path: &str) -> Result<()> {
        let path = normalize_relative_path(path)?;
        let child_prefix = format!("{path}/");
        self.writes
            .retain(|pending, _| pending != &path && !pending.starts_with(&child_prefix));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn flushes_latest_text_and_detects_external_conflicts() {
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "Draft").unwrap();
        let mut saves = AutosaveQueue::default();
        saves.stage(&project, "main.typ", "first".into()).unwrap();
        saves.stage(&project, "main.typ", "latest".into()).unwrap();
        saves.flush(&project, "main.typ", false).unwrap();
        assert_eq!(
            fs::read_to_string(project.root().join("main.typ")).unwrap(),
            "latest"
        );

        saves.stage(&project, "main.typ", "editor".into()).unwrap();
        fs::write(project.root().join("main.typ"), "external").unwrap();
        assert!(saves.flush(&project, "main.typ", false).is_err());
        saves.flush(&project, "main.typ", true).unwrap();
    }

    #[test]
    fn migrates_pending_children_on_folder_rename() {
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "Draft").unwrap();
        project.create_folder("old").unwrap();
        project.create_text_file("old/a.typ", "a").unwrap();
        let mut saves = AutosaveQueue::default();
        saves.stage(&project, "old/a.typ", "new".into()).unwrap();
        saves.rename_path("old", "renamed").unwrap();
        assert_eq!(saves.pending_content("renamed/a.typ"), Some("new"));
    }
}
