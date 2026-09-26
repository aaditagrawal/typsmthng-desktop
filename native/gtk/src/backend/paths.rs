use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::backend::error::{BackendError, Result};

const TEXT_EXTENSIONS: &[&str] = &[
    "typ", "bib", "bibtex", "bbl", "ris", "enw", "nbib", "cff", "biblatex", "csl", "csv", "json",
    "toml", "yaml", "yml", "xml", "txt", "md", "markdown", "tex", "ltx", "sty", "cls", "bst",
    "clo", "def", "fd", "svg", "html", "css", "js", "ts", "cfg", "ini", "log", "rs", "py", "rb",
    "sh", "bat", "ps1",
];

pub fn normalize_relative_path(input: impl AsRef<Path>) -> Result<String> {
    let input = input.as_ref();
    if input.as_os_str().is_empty() || input.is_absolute() {
        return Err(BackendError::UnsafePath(input.display().to_string()));
    }

    let mut segments = Vec::new();
    for component in input.components() {
        match component {
            Component::Normal(segment) => {
                let text = segment.to_string_lossy();
                if text.is_empty() || text.contains('\0') {
                    return Err(BackendError::UnsafePath(input.display().to_string()));
                }
                #[cfg(windows)]
                if text.contains(':') {
                    return Err(BackendError::UnsafePath(input.display().to_string()));
                }
                segments.push(text.into_owned());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BackendError::UnsafePath(input.display().to_string()));
            }
        }
    }
    if segments.is_empty() {
        return Err(BackendError::UnsafePath(input.display().to_string()));
    }
    Ok(segments.join("/"))
}

pub fn canonical_project_root(root: &Path) -> Result<PathBuf> {
    let canonical = root
        .canonicalize()
        .map_err(|error| BackendError::io(root, error))?;
    if !canonical.is_dir() {
        return Err(BackendError::NotFound(root.to_path_buf()));
    }
    Ok(canonical)
}

pub fn safe_existing_path(root: &Path, relative: impl AsRef<Path>) -> Result<(String, PathBuf)> {
    let relative = normalize_relative_path(relative)?;
    let joined = root.join(relative_pathbuf(&relative));
    let canonical = joined
        .canonicalize()
        .map_err(|error| BackendError::io(&joined, error))?;
    if canonical == root || !canonical.starts_with(root) {
        return Err(BackendError::OutsideProject(canonical));
    }
    Ok((relative, canonical))
}

pub fn safe_write_path(root: &Path, relative: impl AsRef<Path>) -> Result<(String, PathBuf)> {
    let relative = normalize_relative_path(relative)?;
    let joined = root.join(relative_pathbuf(&relative));
    let mut ancestor = joined.parent();
    while let Some(path) = ancestor {
        if path.exists() {
            let canonical = path
                .canonicalize()
                .map_err(|error| BackendError::io(path, error))?;
            if canonical != root && !canonical.starts_with(root) {
                return Err(BackendError::OutsideProject(canonical));
            }
            return Ok((relative, joined));
        }
        ancestor = path.parent();
    }
    Err(BackendError::OutsideProject(joined))
}

pub fn relative_path_from_root(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    normalize_relative_path(relative).ok()
}

pub fn relative_pathbuf(normalized: &str) -> PathBuf {
    normalized.split('/').collect()
}

pub fn is_hidden(relative: &str) -> bool {
    relative.split('/').any(|segment| segment.starts_with('.'))
}

pub fn extension(relative: &str) -> String {
    Path::new(relative)
        .extension()
        .and_then(OsStr::to_str)
        .map(|extension| format!(".{}", extension.to_ascii_lowercase()))
        .unwrap_or_default()
}

pub fn is_text_path(relative: &str) -> bool {
    let ext = Path::new(relative)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase);
    ext.as_deref()
        .is_some_and(|extension| TEXT_EXTENSIONS.contains(&extension))
}

pub fn parent_path(relative: &str) -> Option<String> {
    relative
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
}

pub fn basename(relative: &str) -> String {
    relative.rsplit('/').next().unwrap_or(relative).to_owned()
}

pub fn sanitize_project_name(input: &str) -> Result<String> {
    let trimmed = input.trim();
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or_default();
    let sanitized: String = name
        .chars()
        .map(|character| {
            if matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let sanitized = sanitized.trim();
    if sanitized.is_empty() || matches!(sanitized, "." | "..") {
        return Err(BackendError::InvalidProjectName(input.into()));
    }
    Ok(sanitized.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_absolute_and_parent_paths() {
        assert!(normalize_relative_path("../secret").is_err());
        assert!(normalize_relative_path("a/../../secret").is_err());
        assert!(normalize_relative_path("/etc/passwd").is_err());
        assert_eq!(
            normalize_relative_path("./chapters/one.typ").unwrap(),
            "chapters/one.typ"
        );
    }

    #[test]
    fn identifies_text_and_hidden_paths() {
        assert!(is_text_path("main.TYP"));
        assert!(!is_text_path("photo.png"));
        assert!(is_hidden("assets/.draft/one.typ"));
    }
}
