use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use tempfile::NamedTempFile;

use crate::backend::error::{BackendError, Result};
use crate::backend::model::{
    EntryKind, FileContent, PathSearchResult, ProjectEntry, ProjectFile, TextSearchResult,
};
use crate::backend::paths::{
    basename, canonical_project_root, extension, is_hidden, is_text_path, normalize_relative_path,
    parent_path, relative_path_from_root, safe_existing_path, safe_write_path,
    sanitize_project_name,
};

const MAX_INDEX_ENTRIES: usize = 40_000;
const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TEXT_SEARCH_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
}

impl Project {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            root: canonical_project_root(root.as_ref())?,
        })
    }

    pub fn create(parent: impl AsRef<Path>, name: &str) -> Result<Self> {
        let name = sanitize_project_name(name)?;
        let root = parent.as_ref().join(name);
        match fs::read_dir(&root) {
            Ok(entries) => {
                if entries.into_iter().next().is_some() {
                    return Err(BackendError::AlreadyExists(root));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&root).map_err(|error| BackendError::io(&root, error))?;
            }
            Err(error) => return Err(BackendError::io(&root, error)),
        }
        let project = Self::open(&root)?;
        let title = project.name();
        project.create_text_file(
            "main.typ",
            &format!("// {title}\n\n= {title}\n\nStart writing here.\n"),
        )?;
        Ok(project)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Project".into())
    }

    pub fn entries(&self, include_hidden: bool) -> Result<Vec<ProjectEntry>> {
        let mut builder = WalkBuilder::new(&self.root);
        builder
            .hidden(!include_hidden)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            // Prune excluded trees before walking their contents, including outside Git repos.
            .filter_entry(|entry| {
                entry.depth() == 0
                    || !matches!(
                        entry.file_name().to_str(),
                        Some(".git" | ".svn" | ".hg" | "node_modules" | "dist" | "build")
                    )
            });

        let mut entries = Vec::new();
        for result in builder.build() {
            let item = result.map_err(|error| BackendError::Process(error.to_string()))?;
            if item.path() == self.root {
                continue;
            }
            let Some(relative) = relative_path_from_root(&self.root, item.path()) else {
                continue;
            };
            if relative.split('/').any(|part| {
                matches!(
                    part,
                    ".git" | ".svn" | ".hg" | "node_modules" | "dist" | "build"
                )
            }) {
                continue;
            }
            let metadata = item
                .metadata()
                .map_err(|error| BackendError::Process(error.to_string()))?;
            let kind = if metadata.is_dir() {
                EntryKind::Directory
            } else if metadata.is_file() {
                EntryKind::File
            } else {
                continue;
            };
            entries.push(entry_from_metadata(relative, kind, &metadata));
            if entries.len() >= MAX_INDEX_ENTRIES {
                break;
            }
        }
        entries.sort_by(compare_entries_preorder);
        Ok(entries)
    }

    pub fn read_file(&self, relative: impl AsRef<Path>) -> Result<ProjectFile> {
        let (relative, absolute) = safe_existing_path(&self.root, relative)?;
        let metadata =
            fs::metadata(&absolute).map_err(|error| BackendError::io(&absolute, error))?;
        if !metadata.is_file() {
            return Err(BackendError::NotFound(absolute));
        }
        if metadata.len() > MAX_READ_BYTES {
            return Err(BackendError::Process(format!(
                "{} is larger than the 64 MiB editor limit",
                relative
            )));
        }
        let content = if is_text_path(&relative) {
            FileContent::Text(
                fs::read_to_string(&absolute)
                    .map_err(|error| BackendError::io(&absolute, error))?,
            )
        } else {
            FileContent::Binary(
                fs::read(&absolute).map_err(|error| BackendError::io(&absolute, error))?,
            )
        };
        Ok(ProjectFile {
            entry: entry_from_metadata(relative, EntryKind::File, &metadata),
            content,
        })
    }

    pub fn create_text_file(
        &self,
        relative: impl AsRef<Path>,
        content: &str,
    ) -> Result<ProjectEntry> {
        let (relative, absolute) = safe_write_path(&self.root, relative)?;
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&absolute)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    BackendError::AlreadyExists(absolute.clone())
                } else {
                    BackendError::io(&absolute, error)
                }
            })?;
        file.write_all(content.as_bytes())
            .map_err(|error| BackendError::io(&absolute, error))?;
        file.sync_all()
            .map_err(|error| BackendError::io(&absolute, error))?;
        self.entry(&relative)
    }

    pub fn create_binary_file(
        &self,
        relative: impl AsRef<Path>,
        content: &[u8],
    ) -> Result<ProjectEntry> {
        let (relative, absolute) = safe_write_path(&self.root, relative)?;
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&absolute)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    BackendError::AlreadyExists(absolute.clone())
                } else {
                    BackendError::io(&absolute, error)
                }
            })?;
        file.write_all(content)
            .map_err(|error| BackendError::io(&absolute, error))?;
        file.sync_all()
            .map_err(|error| BackendError::io(&absolute, error))?;
        self.entry(&relative)
    }

    pub fn write_text_atomic(
        &self,
        relative: impl AsRef<Path>,
        content: &str,
    ) -> Result<ProjectEntry> {
        let (relative, absolute) = safe_write_path(&self.root, relative)?;
        let parent = absolute
            .parent()
            .ok_or_else(|| BackendError::UnsafePath(relative.clone()))?;
        fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
        let mut temporary =
            NamedTempFile::new_in(parent).map_err(|error| BackendError::io(parent, error))?;
        temporary
            .write_all(content.as_bytes())
            .map_err(|error| BackendError::io(&absolute, error))?;
        temporary
            .as_file_mut()
            .sync_all()
            .map_err(|error| BackendError::io(&absolute, error))?;
        persist_replace(temporary, &absolute)?;
        self.entry(&relative)
    }

    pub fn create_folder(&self, relative: impl AsRef<Path>) -> Result<()> {
        let (_, absolute) = safe_write_path(&self.root, relative)?;
        fs::create_dir_all(&absolute).map_err(|error| BackendError::io(&absolute, error))
    }

    pub fn duplicate(
        &self,
        source: impl AsRef<Path>,
        target: impl AsRef<Path>,
    ) -> Result<ProjectEntry> {
        let (_, source) = safe_existing_path(&self.root, source)?;
        let (target_relative, target) = safe_write_path(&self.root, target)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
        }
        let mut input = File::open(&source).map_err(|error| BackendError::io(&source, error))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    BackendError::AlreadyExists(target.clone())
                } else {
                    BackendError::io(&target, error)
                }
            })?;
        std::io::copy(&mut input, &mut output).map_err(|error| BackendError::io(&target, error))?;
        output
            .sync_all()
            .map_err(|error| BackendError::io(&target, error))?;
        self.entry(&target_relative)
    }

    pub fn rename(&self, old: impl AsRef<Path>, new: impl AsRef<Path>) -> Result<()> {
        let (_, old) = safe_existing_path(&self.root, old)?;
        let (_, new) = safe_write_path(&self.root, new)?;
        if new.exists() {
            let same_file = old.canonicalize().ok() == new.canonicalize().ok();
            if !same_file {
                return Err(BackendError::AlreadyExists(new));
            }
        }
        if let Some(parent) = new.parent() {
            fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
        }
        fs::rename(&old, &new).map_err(|error| BackendError::io(&old, error))
    }

    pub fn move_to_trash(&self, relative: impl AsRef<Path>) -> Result<()> {
        let (_, absolute) = safe_existing_path(&self.root, relative)?;
        trash::delete(&absolute).map_err(|error| BackendError::Process(error.to_string()))
    }

    pub fn delete_permanently(&self, relative: impl AsRef<Path>) -> Result<()> {
        let (_, absolute) = safe_existing_path(&self.root, relative)?;
        let metadata =
            fs::symlink_metadata(&absolute).map_err(|error| BackendError::io(&absolute, error))?;
        if metadata.is_dir() {
            fs::remove_dir_all(&absolute).map_err(|error| BackendError::io(&absolute, error))
        } else {
            fs::remove_file(&absolute).map_err(|error| BackendError::io(&absolute, error))
        }
    }

    pub fn entry(&self, relative: impl AsRef<Path>) -> Result<ProjectEntry> {
        let (relative, absolute) = safe_existing_path(&self.root, relative)?;
        let metadata =
            fs::metadata(&absolute).map_err(|error| BackendError::io(&absolute, error))?;
        let kind = if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            return Err(BackendError::NotFound(absolute));
        };
        Ok(entry_from_metadata(relative, kind, &metadata))
    }

    pub fn search_paths(
        &self,
        query: &str,
        limit: usize,
        include_hidden: bool,
    ) -> Result<(Vec<PathSearchResult>, bool)> {
        let query = query
            .trim()
            .trim_start_matches(['@', '.', '/'])
            .to_lowercase();
        let mut matches = self
            .entries(include_hidden)?
            .into_iter()
            .filter_map(|entry| {
                let path = entry.path.to_lowercase();
                let name = entry.name.to_lowercase();
                if !query.is_empty() && !path.contains(&query) {
                    return None;
                }
                let score = if query.is_empty() {
                    u8::from(entry.kind == EntryKind::File)
                } else if name == query {
                    0
                } else if path == query {
                    1
                } else if name.starts_with(&query) {
                    2
                } else if path.starts_with(&query) {
                    3
                } else if path.contains(&format!("/{query}")) {
                    4
                } else {
                    5
                };
                Some(PathSearchResult { entry, score })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            left.score
                .cmp(&right.score)
                .then_with(|| left.entry.path.cmp(&right.entry.path))
        });
        let truncated = matches.len() > limit;
        matches.truncate(limit);
        Ok((matches, truncated))
    }

    pub fn search_text(
        &self,
        query: &str,
        limit: usize,
        include_hidden: bool,
    ) -> Result<(Vec<TextSearchResult>, bool)> {
        let query = query.trim();
        if query.is_empty() {
            return Ok((Vec::new(), false));
        }
        let needle = query.to_lowercase();
        let mut results = Vec::new();
        let mut truncated = false;
        for entry in self.entries(include_hidden)? {
            if entry.kind != EntryKind::File
                || entry.is_binary
                || entry.size_bytes > MAX_TEXT_SEARCH_BYTES
            {
                continue;
            }
            let (_, path) = safe_existing_path(&self.root, &entry.path)?;
            let content = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(_) => continue,
            };
            for (line_index, line) in content.lines().enumerate() {
                let lower = line.to_lowercase();
                let Some(byte_index) = lower.find(&needle) else {
                    continue;
                };
                let column = lower[..byte_index].chars().count() + 1;
                results.push(TextSearchResult {
                    path: entry.path.clone(),
                    line: line_index + 1,
                    column,
                    preview: line.split_whitespace().collect::<Vec<_>>().join(" "),
                });
                if results.len() >= limit {
                    truncated = true;
                    return Ok((results, truncated));
                }
            }
        }
        Ok((results, truncated))
    }

    pub fn resolve_main_file(&self, current: Option<&str>) -> Result<String> {
        let files = self.entries(true)?;
        let mut main_files = files
            .iter()
            .filter(|entry| {
                entry.kind == EntryKind::File && entry.name.eq_ignore_ascii_case("main.typ")
            })
            .collect::<Vec<_>>();
        main_files.sort_by_key(|entry| entry.path.matches('/').count());
        if let Some(entry) = main_files.first() {
            return Ok(entry.path.clone());
        }
        if let Some(current) = current {
            let normalized = normalize_relative_path(current)?;
            if files
                .iter()
                .any(|entry| entry.path == normalized && entry.extension == ".typ")
            {
                return Ok(normalized);
            }
        }
        files
            .iter()
            .find(|entry| entry.kind == EntryKind::File && entry.extension == ".typ")
            .map(|entry| entry.path.clone())
            .ok_or_else(|| BackendError::NotFound(self.root.join("main.typ")))
    }
}

fn compare_entries_preorder(left: &ProjectEntry, right: &ProjectEntry) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let left_parts = left.path.split('/').collect::<Vec<_>>();
    let right_parts = right.path.split('/').collect::<Vec<_>>();
    for index in 0..left_parts.len().min(right_parts.len()) {
        if left_parts[index].eq_ignore_ascii_case(right_parts[index]) {
            continue;
        }

        // A path component with descendants is necessarily a directory even
        // when the complete entry being compared is a file. This keeps every
        // directory immediately followed by its subtree while still sorting
        // directories before files among siblings.
        let left_is_directory = index + 1 < left_parts.len() || left.kind == EntryKind::Directory;
        let right_is_directory =
            index + 1 < right_parts.len() || right.kind == EntryKind::Directory;
        return match (left_is_directory, right_is_directory) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => left_parts[index]
                .to_ascii_lowercase()
                .cmp(&right_parts[index].to_ascii_lowercase())
                .then_with(|| left_parts[index].cmp(right_parts[index])),
        };
    }

    left_parts
        .len()
        .cmp(&right_parts.len())
        .then_with(|| left.path.cmp(&right.path))
}

fn entry_from_metadata(relative: String, kind: EntryKind, metadata: &fs::Metadata) -> ProjectEntry {
    ProjectEntry {
        name: basename(&relative),
        parent_path: parent_path(&relative),
        extension: if kind == EntryKind::File {
            extension(&relative)
        } else {
            String::new()
        },
        is_hidden: is_hidden(&relative),
        is_binary: kind == EntryKind::File && !is_text_path(&relative),
        last_modified_ms: metadata
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH)
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        size_bytes: metadata.len(),
        path: relative,
        kind,
    }
}

fn persist_replace(temporary: NamedTempFile, path: &Path) -> Result<()> {
    match temporary.persist(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(path).map_err(|remove| BackendError::io(path, remove))?;
            error
                .file
                .persist(path)
                .map(|_| ())
                .map_err(|persist| BackendError::io(path, persist.error))
        }
        Err(error) => Err(BackendError::io(path, error.error)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn fixture() -> (tempfile::TempDir, Project) {
        let temporary = tempdir().unwrap();
        let project = Project::create(temporary.path(), "Paper").unwrap();
        project.create_folder("chapters").unwrap();
        project
            .create_text_file("chapters/one.typ", "= One\nneedle here")
            .unwrap();
        (temporary, project)
    }

    #[test]
    fn excluded_trees_are_pruned_even_outside_git_repositories() {
        let (temporary, project) = fixture();
        for directory in [
            "node_modules",
            "dist",
            "build",
            ".git",
            "chapters/node_modules",
        ] {
            fs::create_dir_all(project.root().join(directory).join("nested")).unwrap();
            fs::write(
                project.root().join(directory).join("nested/secret.typ"),
                "hidden needle",
            )
            .unwrap();
        }
        let entries = project.entries(true).unwrap();
        assert!(!entries
            .iter()
            .any(|entry| entry.path.contains("secret") || entry.path.contains("node_modules")));
        assert!(project
            .search_text("hidden needle", 10, true)
            .unwrap()
            .0
            .is_empty());
        drop(temporary);
    }

    #[test]
    fn supports_crud_and_search() {
        let (_temporary, project) = fixture();
        project
            .duplicate("chapters/one.typ", "chapters/two.typ")
            .unwrap();
        project
            .rename("chapters/two.typ", "chapters/renamed.typ")
            .unwrap();
        project
            .write_text_atomic("chapters/renamed.typ", "changed")
            .unwrap();
        let (paths, _) = project.search_paths("renamed", 10, false).unwrap();
        assert_eq!(paths[0].entry.path, "chapters/renamed.typ");
        let (text, _) = project.search_text("needle", 10, false).unwrap();
        assert_eq!(text[0].line, 2);
        project.delete_permanently("chapters/renamed.typ").unwrap();
        assert!(!project.root().join("chapters/renamed.typ").exists());
    }

    #[test]
    fn create_does_not_overwrite_existing_project() {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("Paper");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("valuable.txt"), "keep").unwrap();
        assert!(matches!(
            Project::create(temporary.path(), "Paper"),
            Err(BackendError::AlreadyExists(_))
        ));
        assert_eq!(
            fs::read_to_string(root.join("valuable.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn entries_are_sorted_in_hierarchical_preorder() {
        let (temporary, project) = fixture();
        project.create_folder("appendix").unwrap();
        project
            .create_text_file("appendix/a.typ", "= Appendix")
            .unwrap();
        project.create_text_file("z.typ", "= Z").unwrap();
        let paths = project
            .entries(false)
            .unwrap()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>();
        let appendix = paths.iter().position(|path| path == "appendix").unwrap();
        let appendix_child = paths
            .iter()
            .position(|path| path == "appendix/a.typ")
            .unwrap();
        let chapters = paths.iter().position(|path| path == "chapters").unwrap();
        let chapter_child = paths
            .iter()
            .position(|path| path == "chapters/one.typ")
            .unwrap();
        assert_eq!(appendix_child, appendix + 1);
        assert_eq!(chapter_child, chapters + 1);
        assert!(chapter_child < paths.iter().position(|path| path == "main.typ").unwrap());
        drop(temporary);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_writes_through_symlinks_outside_project() {
        use std::os::unix::fs::symlink;

        let (_temporary, project) = fixture();
        let outside = tempdir().unwrap();
        symlink(outside.path(), project.root().join("escape")).unwrap();
        assert!(matches!(
            project.create_text_file("escape/secret.typ", "nope"),
            Err(BackendError::OutsideProject(_))
        ));
        assert!(!outside.path().join("secret.typ").exists());
    }
}
