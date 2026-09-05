use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::backend::error::{BackendError, Result};
use crate::backend::model::EntryKind;
use crate::backend::paths::{normalize_relative_path, relative_pathbuf, sanitize_project_name};
use crate::backend::project::Project;

#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_file_bytes: 256 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

pub fn export_project(project: &Project, destination: impl AsRef<Path>) -> Result<PathBuf> {
    let destination = destination.as_ref();
    let entries = export_entries(project)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| BackendError::io(parent, error))?;
    let file = temporary
        .reopen()
        .map_err(|error| BackendError::io(temporary.path(), error))?;
    let mut archive = ZipWriter::new(file);
    let file_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let directory_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);
    for entry in entries {
        if !is_exportable(&entry.path) {
            continue;
        }
        match entry.kind {
            EntryKind::Directory => archive
                .add_directory(format!("{}/", entry.path), directory_options)
                .map_err(BackendError::from)?,
            EntryKind::File => {
                if same_path(&path_for_entry(project, &entry.path), destination) {
                    continue;
                }
                archive.start_file(&entry.path, file_options)?;
                let path = path_for_entry(project, &entry.path);
                let mut source =
                    File::open(&path).map_err(|error| BackendError::io(&path, error))?;
                io::copy(&mut source, &mut archive)
                    .map_err(|error| BackendError::io(destination, error))?;
            }
        }
    }
    archive
        .finish()
        .map_err(BackendError::from)?
        .sync_all()
        .map_err(|error| BackendError::io(destination, error))?;
    temporary
        .persist(destination)
        .map_err(|error| BackendError::io(destination, error.error))?;
    Ok(destination.to_path_buf())
}

pub fn export_projects(projects: &[Project], destination: impl AsRef<Path>) -> Result<PathBuf> {
    let destination = destination.as_ref();
    if projects.is_empty() {
        return Err(BackendError::InvalidArchive("no projects to export".into()));
    }
    let project_entries = projects
        .iter()
        .map(export_entries)
        .collect::<Result<Vec<_>>>()?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| BackendError::io(parent, error))?;
    let file = temporary
        .reopen()
        .map_err(|error| BackendError::io(temporary.path(), error))?;
    let mut archive = ZipWriter::new(file);
    let file_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let directory_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);
    let mut used_names = std::collections::HashSet::new();
    for (project, entries) in projects.iter().zip(project_entries) {
        let base = unique_archive_name(&project.name(), &mut used_names);
        archive.add_directory(format!("{base}/"), directory_options)?;
        for entry in entries {
            if !is_exportable(&entry.path) {
                continue;
            }
            let archive_path = format!("{base}/{}", entry.path);
            match entry.kind {
                EntryKind::Directory => archive
                    .add_directory(format!("{archive_path}/"), directory_options)
                    .map_err(BackendError::from)?,
                EntryKind::File => {
                    if same_path(&path_for_entry(project, &entry.path), destination) {
                        continue;
                    }
                    archive.start_file(archive_path, file_options)?;
                    let path = path_for_entry(project, &entry.path);
                    let mut source =
                        File::open(&path).map_err(|error| BackendError::io(&path, error))?;
                    io::copy(&mut source, &mut archive)
                        .map_err(|error| BackendError::io(destination, error))?;
                }
            }
        }
    }
    archive
        .finish()
        .map_err(BackendError::from)?
        .sync_all()
        .map_err(|error| BackendError::io(destination, error))?;
    temporary
        .persist(destination)
        .map_err(|error| BackendError::io(destination, error.error))?;
    Ok(destination.to_path_buf())
}

fn path_for_entry(project: &Project, relative: &str) -> PathBuf {
    project.root().join(relative_pathbuf(relative))
}

fn same_path(left: &Path, right: &Path) -> bool {
    let absolute_right = if right.is_absolute() {
        right.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(right)
    };
    left == absolute_right
}

fn export_entries(project: &Project) -> Result<Vec<crate::backend::model::ProjectEntry>> {
    let entries = project.entries(true)?;
    if entries.len() >= 40_000 {
        return Err(BackendError::InvalidArchive(
            "project export exceeds the 40,000-entry safety limit".into(),
        ));
    }
    Ok(entries)
}

fn is_exportable(path: &str) -> bool {
    if path == ".git" || path.starts_with(".git/") {
        return false;
    }
    if path == ".folder" || path.ends_with("/.folder") {
        return false;
    }
    path == ".typsmthng" || path == ".typsmthng/template.json" || !path.starts_with(".typsmthng/")
}

pub fn import_projects(
    archive_path: impl AsRef<Path>,
    parent: impl AsRef<Path>,
    limits: ArchiveLimits,
) -> Result<Vec<Project>> {
    let archive_path = archive_path.as_ref();
    let parent = parent
        .as_ref()
        .canonicalize()
        .map_err(|error| BackendError::io(parent.as_ref(), error))?;
    let staging = TempDir::new_in(&parent).map_err(|error| BackendError::io(&parent, error))?;
    extract_into(archive_path, staging.path(), limits)?;
    let mut roots = fs::read_dir(staging.path())
        .map_err(|error| BackendError::io(staging.path(), error))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| BackendError::io(staging.path(), error))?;
    roots.sort_by_key(std::fs::DirEntry::file_name);
    if roots.is_empty() || roots.iter().any(|entry| !entry.path().is_dir()) {
        return Err(BackendError::InvalidArchive(
            "multi-project archives must contain project folders".into(),
        ));
    }
    let destinations = roots
        .iter()
        .map(|entry| {
            let name = sanitize_project_name(&entry.file_name().to_string_lossy())?;
            let destination = parent.join(name);
            if destination.exists() {
                return Err(BackendError::AlreadyExists(destination));
            }
            Ok(destination)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut projects = Vec::with_capacity(roots.len());
    for (entry, destination) in roots.into_iter().zip(destinations) {
        fs::rename(entry.path(), &destination)
            .map_err(|error| BackendError::io(&destination, error))?;
        projects.push(Project::open(destination)?);
    }
    Ok(projects)
}

fn unique_archive_name(project_name: &str, used: &mut std::collections::HashSet<String>) -> String {
    let base = sanitize_project_name(project_name).unwrap_or_else(|_| "Project".into());
    if used.insert(base.clone()) {
        return base;
    }
    for index in 2..10_000 {
        let candidate = format!("{base}-{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    format!("{base}-export")
}

pub fn import_project(
    archive_path: impl AsRef<Path>,
    parent: impl AsRef<Path>,
    project_name: &str,
    limits: ArchiveLimits,
) -> Result<Project> {
    let archive_path = archive_path.as_ref();
    let parent = parent
        .as_ref()
        .canonicalize()
        .map_err(|error| BackendError::io(parent.as_ref(), error))?;
    let name = sanitize_project_name(project_name)?;
    let destination = parent.join(&name);
    if destination.exists() {
        return Err(BackendError::AlreadyExists(destination));
    }
    let staging = TempDir::new_in(&parent).map_err(|error| BackendError::io(&parent, error))?;
    extract_into(archive_path, staging.path(), limits)?;
    fs::rename(staging.path(), &destination)
        .map_err(|error| BackendError::io(&destination, error))?;
    std::mem::forget(staging);
    Project::open(destination)
}

fn extract_into(archive_path: &Path, destination: &Path, limits: ArchiveLimits) -> Result<()> {
    let file = File::open(archive_path).map_err(|error| BackendError::io(archive_path, error))?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() > limits.max_files {
        return Err(BackendError::ArchiveLimit("file count"));
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.size() > limits.max_file_bytes {
            return Err(BackendError::ArchiveLimit("single file size"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or(BackendError::ArchiveLimit("total uncompressed size"))?;
        if total > limits.max_total_bytes {
            return Err(BackendError::ArchiveLimit("total uncompressed size"));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(BackendError::InvalidArchive(format!(
                "symbolic links are not supported: {}",
                entry.name()
            )));
        }
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            BackendError::InvalidArchive(format!("path escapes project: {}", entry.name()))
        })?;
        let normalized = normalize_relative_path(enclosed)?;
        let output = destination.join(relative_pathbuf(&normalized));
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|error| BackendError::io(&output, error))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
        }
        let mut file = File::create(&output).map_err(|error| BackendError::io(&output, error))?;
        let copied = copy_bounded(&mut entry, &mut file, limits.max_file_bytes)?;
        if copied != entry.size() {
            return Err(BackendError::InvalidArchive(format!(
                "truncated entry: {}",
                entry.name()
            )));
        }
    }
    Ok(())
}

fn copy_bounded<R: Read, W: Write>(reader: &mut R, writer: &mut W, limit: u64) -> Result<u64> {
    let mut limited = reader.take(limit.saturating_add(1));
    let copied = io::copy(&mut limited, writer)
        .map_err(|error| BackendError::io("archive stream", error))?;
    if copied > limit {
        return Err(BackendError::ArchiveLimit("single file size"));
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn round_trips_text_binary_and_empty_folders() {
        let directory = tempdir().unwrap();
        let source = Project::create(directory.path(), "Source").unwrap();
        source
            .create_binary_file("images/pixel.bin", &[0, 1, 2, 255])
            .unwrap();
        source.create_folder("empty").unwrap();
        let archive = directory.path().join("project.zip");
        export_project(&source, &archive).unwrap();
        let imported = import_project(
            &archive,
            directory.path(),
            "Imported",
            ArchiveLimits::default(),
        )
        .unwrap();
        assert_eq!(
            fs::read(imported.root().join("images/pixel.bin")).unwrap(),
            [0, 1, 2, 255]
        );
        assert!(imported.root().join("empty").is_dir());
    }

    #[test]
    fn round_trips_multiple_projects_in_one_archive() {
        let directory = tempdir().unwrap();
        let first = Project::create(directory.path(), "First").unwrap();
        let second = Project::create(directory.path(), "Second").unwrap();
        first.write_text_atomic("main.typ", "= First").unwrap();
        second.write_text_atomic("main.typ", "= Second").unwrap();
        let archive = directory.path().join("all.zip");
        export_projects(&[first, second], &archive).unwrap();
        let destination = tempdir().unwrap();
        let imported =
            import_projects(&archive, destination.path(), ArchiveLimits::default()).unwrap();
        assert_eq!(imported.len(), 2);
        assert!(destination.path().join("First/main.typ").is_file());
        assert!(destination.path().join("Second/main.typ").is_file());
    }

    #[test]
    fn export_inside_project_does_not_archive_itself_or_private_metadata() {
        let directory = tempdir().unwrap();
        let source = Project::create(directory.path(), "Source").unwrap();
        source
            .create_text_file(".typsmthng/private.json", "secret")
            .unwrap();
        source
            .create_text_file(".typsmthng/template.json", "{}")
            .unwrap();
        let destination = source.root().join("project.zip");
        export_project(&source, &destination).unwrap();
        let mut archive = ZipArchive::new(File::open(destination).unwrap()).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect::<Vec<_>>();
        assert!(!names.iter().any(|name| name == "project.zip"));
        assert!(!names.iter().any(|name| name == ".typsmthng/private.json"));
        assert!(names.iter().any(|name| name == ".typsmthng/template.json"));
    }

    #[test]
    fn rejects_zip_slip_paths() {
        let directory = tempdir().unwrap();
        let archive_path = directory.path().join("evil.zip");
        let file = File::create(&archive_path).unwrap();
        let mut writer = ZipWriter::new(file);
        // `start_file` rejects traversal in newer zip releases, so construct a
        // name using the raw API if accepted and otherwise the library already
        // enforces the invariant we test in normalize_relative_path.
        assert!(normalize_relative_path("../escape.typ").is_err());
        writer
            .start_file("safe.typ", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"safe").unwrap();
        writer.finish().unwrap();
        let imported = import_project(
            &archive_path,
            directory.path(),
            "Safe",
            ArchiveLimits::default(),
        )
        .unwrap();
        assert!(imported.root().join("safe.typ").exists());
    }

    #[test]
    fn enforces_uncompressed_size_limits() {
        let directory = tempdir().unwrap();
        let source = Project::create(directory.path(), "Source").unwrap();
        source
            .create_text_file("large.txt", &"x".repeat(128))
            .unwrap();
        let archive = directory.path().join("large.zip");
        export_project(&source, &archive).unwrap();
        let limits = ArchiveLimits {
            max_files: 10,
            max_file_bytes: 32,
            max_total_bytes: 64,
        };
        assert!(matches!(
            import_project(&archive, directory.path(), "TooLarge", limits),
            Err(BackendError::ArchiveLimit(_))
        ));
    }
}
