use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, BackendError>;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("unsafe project path: {0}")]
    UnsafePath(String),
    #[error("path is outside the project root: {0}")]
    OutsideProject(PathBuf),
    #[error("a file or folder already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("path does not exist: {0}")]
    NotFound(PathBuf),
    #[error("invalid project name: {0}")]
    InvalidProjectName(String),
    #[error("invalid archive: {0}")]
    InvalidArchive(String),
    #[error("archive exceeds the configured {0} limit")]
    ArchiveLimit(&'static str),
    #[error("Typst was not found; install Typst {required} or set TYPSMTHNG_TYPST")]
    TypstNotFound { required: &'static str },
    #[error("Typst {found} is not supported; this build requires Typst {required}")]
    UnsupportedTypstVersion {
        found: String,
        required: &'static str,
    },
    #[error("Typst process timed out")]
    TypstTimeout,
    #[error("Typst process failed to start: {0}")]
    Process(String),
    #[error("network request failed: {0}")]
    Network(String),
    #[error("Typst compilation failed")]
    CompileFailed,
    #[error("JSON error in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Notify(#[from] notify::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}

impl BackendError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
