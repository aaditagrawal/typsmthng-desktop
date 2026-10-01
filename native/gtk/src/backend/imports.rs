//! Filesystem imports and LaTeX conversion through the project path safeguards.
use super::{convert_latex_to_typst, BackendError, EntryKind, FileContent, Project, Result};
use std::path::{Path, PathBuf};

pub fn import_latex_file(source_path: &Path, parent: &Path, project_name: &str) -> Result<Project> {
    let project = Project::create(parent, project_name)?;
    let source = std::fs::read_to_string(source_path).map_err(|error| {
        BackendError::Process(format!("could not read {}: {error}", source_path.display()))
    })?;
    let converted = convert_latex_to_typst(&source);
    project.write_text_atomic("main.typ", &converted.typst)?;
    project.write_text_atomic("source.tex", &source)?;
    Ok(project)
}

pub fn import_latex_sources(
    sources: &[PathBuf],
    parent: &Path,
    project_name: &str,
) -> Result<(Project, Vec<String>)> {
    let project = Project::create(parent, project_name)?;
    project.delete_permanently("main.typ")?;
    let mut warnings = Vec::new();
    for source in sources {
        if source.is_dir() {
            let mut children = std::fs::read_dir(source)
                .map_err(|error| {
                    BackendError::Process(format!("could not read {}: {error}", source.display()))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| BackendError::Process(error.to_string()))?;
            children.sort_by_key(std::fs::DirEntry::file_name);
            for child in children {
                import_path_tree(
                    &project,
                    &child.path(),
                    Path::new(&child.file_name()),
                    &mut warnings,
                )?;
            }
        } else if let Some(name) = source.file_name() {
            let target = unique_project_path(&project, Path::new(name));
            import_path_tree(&project, source, &target, &mut warnings)?;
        }
    }
    if project.resolve_main_file(None).is_err() {
        project.write_text_atomic(
            "main.typ",
            "= Imported LaTeX project\n\nNo convertible .tex source was selected. Add or import a LaTeX source to continue.\n",
        )?;
        warnings.push("No convertible .tex source was found; a new main.typ was created.".into());
    }
    Ok((project, warnings))
}

pub fn convert_latex_project_if_needed(project: &Project) -> Result<()> {
    let entries = project.entries(true)?;
    for source_path in entries.iter().filter_map(|entry| {
        let lower = entry.path.to_ascii_lowercase();
        (entry.kind == EntryKind::File && (lower.ends_with(".tex") || lower.ends_with(".ltx")))
            .then_some(entry.path.clone())
    }) {
        let source = match project.read_file(&source_path)?.content {
            FileContent::Text(source) => source,
            FileContent::Binary(_) => continue,
        };
        let converted = convert_latex_to_typst(&source);
        let target = Path::new(&source_path).with_extension("typ");
        project.write_text_atomic(&target, &converted.typst)?;
    }
    Ok(())
}

pub fn import_path_tree(
    project: &Project,
    source: &Path,
    target: &Path,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| {
        BackendError::Process(format!("could not inspect {}: {error}", source.display()))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(BackendError::Process(
            "symbolic links are not imported for safety".into(),
        ));
    }
    if metadata.is_dir() {
        project.create_folder(target)?;
        let mut children = std::fs::read_dir(source)
            .map_err(|error| {
                BackendError::Process(format!("could not read {}: {error}", source.display()))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| BackendError::Process(error.to_string()))?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            import_path_tree(
                project,
                &child.path(),
                &target.join(child.file_name()),
                warnings,
            )?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(BackendError::Process("unsupported filesystem entry".into()));
    }

    let bytes = std::fs::read(source).map_err(|error| {
        BackendError::Process(format!("could not read {}: {error}", source.display()))
    })?;
    project.create_binary_file(target, &bytes)?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("tex") || extension.eq_ignore_ascii_case("ltx") {
        let latex = std::str::from_utf8(&bytes).map_err(|error| {
            BackendError::Process(format!("{} is not UTF-8 LaTeX: {error}", source.display()))
        })?;
        let converted = convert_latex_to_typst(latex);
        warnings.extend(
            converted
                .warnings
                .iter()
                .map(|warning| format!("{}: {}", target.display(), warning.message)),
        );
        let typst_target = unique_project_path(project, &target.with_extension("typ"));
        project.write_text_atomic(typst_target, &converted.typst)?;
    }
    Ok(())
}

pub fn unique_project_path(project: &Project, desired: &Path) -> PathBuf {
    if !project.root().join(desired).exists() {
        return desired.to_path_buf();
    }
    let parent = desired.parent().unwrap_or_else(|| Path::new(""));
    let stem = desired
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imported".into());
    let extension = desired.extension().map(|value| value.to_string_lossy());
    for index in 2..10_000 {
        let mut name = format!("{stem}-{index}");
        if let Some(extension) = &extension {
            name.push('.');
            name.push_str(extension);
        }
        let candidate = parent.join(name);
        if !project.root().join(&candidate).exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}-imported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn imports_preserve_original_sources_and_existing_typst_files() {
        let source = tempdir().unwrap();
        let raw = "\\section{Imported}\nHello";
        fs::write(source.path().join("paper.tex"), raw).unwrap();
        let destination = tempdir().unwrap();
        let project = Project::create(destination.path(), "Demo").unwrap();
        project
            .write_text_atomic("paper.typ", "Existing document")
            .unwrap();
        import_path_tree(
            &project,
            &source.path().join("paper.tex"),
            Path::new("paper.tex"),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(project.root().join("paper.tex")).unwrap(),
            raw
        );
        assert_eq!(
            fs::read_to_string(project.root().join("paper.typ")).unwrap(),
            "Existing document"
        );
        assert!(fs::read_to_string(project.root().join("paper-2.typ"))
            .unwrap()
            .contains("Imported"));
        let single = import_latex_file(
            &source.path().join("paper.tex"),
            destination.path(),
            "Single",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(single.root().join("source.tex")).unwrap(),
            raw
        );
        assert!(fs::read_to_string(single.root().join("main.typ"))
            .unwrap()
            .contains("Imported"));
    }

    #[cfg(unix)]
    #[test]
    fn imports_reject_symlinks_without_copying_their_targets() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("outside.tex"), "Outside").unwrap();
        std::os::unix::fs::symlink(
            source.path().join("outside.tex"),
            source.path().join("link.tex"),
        )
        .unwrap();
        let destination = tempdir().unwrap();
        let project = Project::create(destination.path(), "Demo").unwrap();
        assert!(import_path_tree(
            &project,
            &source.path().join("link.tex"),
            Path::new("copied.tex"),
            &mut Vec::new()
        )
        .is_err());
        assert!(!project.root().join("copied.tex").exists());
    }
    #[test]
    fn latex_folder_import_preserves_assets_and_converts_nested_sources() {
        let source = tempdir().unwrap();
        fs::create_dir(source.path().join("images")).unwrap();
        fs::write(source.path().join("images/chart.png"), b"png").unwrap();
        fs::write(
            source.path().join("paper.tex"),
            "\\documentclass{article}\\begin{document}Hello\\includegraphics{images/chart.png}\\end{document}",
        )
        .unwrap();
        let destination = tempdir().unwrap();
        let (project, _) = import_latex_sources(
            &[source.path().to_path_buf()],
            destination.path(),
            "Imported",
        )
        .unwrap();
        assert!(project.root().join("paper.tex").is_file());
        assert!(project.root().join("paper.typ").is_file());
        assert!(project.root().join("images/chart.png").is_file());
    }
}
