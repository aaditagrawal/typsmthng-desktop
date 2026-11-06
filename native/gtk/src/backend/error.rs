use std::path::PathBuf;

use std::error::Error;
use std::fmt;

pub type Result<T> = std::result::Result<T, BackendError>;

#[derive(Debug)]
pub enum BackendError {
    UnsafePath(String),
    OutsideProject(PathBuf),
    AlreadyExists(PathBuf),
    NotFound(PathBuf),
    InvalidProjectName(String),
    InvalidArchive(String),
    ArchiveLimit(&'static str),
    TypstNotFound {
        required: &'static str,
    },
    UnsupportedTypstVersion {
        found: String,
        required: &'static str,
    },
    InvalidTypstConfiguration {
        setting: &'static str,
        reason: &'static str,
    },
    TypstTimeout,
    Process(String),
    Network(String),
    FontRegistration(String),
    CompileFailed,
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Notify(notify::Error),
    Zip(zip::result::ZipError),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafePath(path) => write!(f, "unsafe project path: {path}"),
            Self::OutsideProject(path) => {
                write!(f, "path is outside the project root: {}", path.display())
            }
            Self::AlreadyExists(path) => {
                write!(f, "a file or folder already exists at {}", path.display())
            }
            Self::NotFound(path) => write!(f, "path does not exist: {}", path.display()),
            Self::InvalidProjectName(name) => write!(f, "invalid project name: {name}"),
            Self::InvalidArchive(reason) => write!(f, "invalid archive: {reason}"),
            Self::ArchiveLimit(limit) => {
                write!(f, "archive exceeds the configured {limit} limit")
            }
            Self::TypstNotFound { required } => write!(
                f,
                "Typst was not found; install Typst {required} or set TYPSMTHNG_TYPST"
            ),
            Self::UnsupportedTypstVersion { found, required } => write!(
                f,
                "Typst {found} is not supported; this build requires Typst {required}"
            ),
            Self::InvalidTypstConfiguration { setting, reason } => {
                write!(f, "invalid Typst setting {setting}: {reason}")
            }
            Self::TypstTimeout => f.write_str("Typst process timed out"),
            Self::Process(reason) => write!(f, "Typst process failed to start: {reason}"),
            Self::Network(reason) => write!(f, "network request failed: {reason}"),
            Self::FontRegistration(reason) => write!(f, "font registration failed: {reason}"),
            Self::CompileFailed => f.write_str("Typst compilation failed"),
            Self::Json { path, source } => {
                write!(f, "JSON error in {}: {source}", path.display())
            }
            Self::Io { path, source } => write!(f, "I/O error at {}: {source}", path.display()),
            Self::Notify(error) => fmt::Display::fmt(error, f),
            Self::Zip(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl Error for BackendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            // Transparent wrappers expose the wrapped error's source.
            Self::Notify(error) => error.source(),
            Self::Zip(error) => error.source(),
            _ => None,
        }
    }
}

impl From<notify::Error> for BackendError {
    fn from(error: notify::Error) -> Self {
        Self::Notify(error)
    }
}

impl From<zip::result::ZipError> for BackendError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Zip(error)
    }
}

impl BackendError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_preserve_context_and_leaf_errors_have_no_source() {
        let errors = [
            (
                BackendError::UnsafePath("../draft".into()),
                "unsafe project path: ../draft",
            ),
            (
                BackendError::OutsideProject("outside".into()),
                "path is outside the project root: outside",
            ),
            (
                BackendError::AlreadyExists("draft".into()),
                "a file or folder already exists at draft",
            ),
            (
                BackendError::NotFound("draft".into()),
                "path does not exist: draft",
            ),
            (
                BackendError::InvalidProjectName("bad/name".into()),
                "invalid project name: bad/name",
            ),
            (
                BackendError::InvalidArchive("bad header".into()),
                "invalid archive: bad header",
            ),
            (
                BackendError::ArchiveLimit("file count"),
                "archive exceeds the configured file count limit",
            ),
            (
                BackendError::TypstNotFound { required: "0.15.1" },
                "Typst was not found; install Typst 0.15.1 or set TYPSMTHNG_TYPST",
            ),
            (
                BackendError::UnsupportedTypstVersion {
                    found: "0.14".into(),
                    required: "0.15.1",
                },
                "Typst 0.14 is not supported; this build requires Typst 0.15.1",
            ),
            (
                BackendError::InvalidTypstConfiguration {
                    setting: "date",
                    reason: "invalid date",
                },
                "invalid Typst setting date: invalid date",
            ),
            (BackendError::TypstTimeout, "Typst process timed out"),
            (
                BackendError::Process("permission denied".into()),
                "Typst process failed to start: permission denied",
            ),
            (
                BackendError::Network("offline".into()),
                "network request failed: offline",
            ),
            (
                BackendError::FontRegistration("bad font".into()),
                "font registration failed: bad font",
            ),
            (BackendError::CompileFailed, "Typst compilation failed"),
        ];
        for (error, message) in errors {
            assert_eq!(error.to_string(), message);
            assert!(error.source().is_none());
        }
    }

    #[test]
    fn structured_errors_keep_their_source_and_transparent_wrappers_delegate() {
        let io = BackendError::io("draft.typ", std::io::Error::other("disk full"));
        assert_eq!(io.to_string(), "I/O error at draft.typ: disk full");
        assert!(io.source().unwrap().is::<std::io::Error>());
        let source = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let expected = format!("JSON error in state.json: {source}");
        let json = BackendError::Json {
            path: "state.json".into(),
            source,
        };
        assert_eq!(json.to_string(), expected);
        assert!(json.source().unwrap().is::<serde_json::Error>());

        let notify =
            notify::Error::io(std::io::Error::other("watch denied")).add_path("draft.typ".into());
        let expected = notify.to_string();
        let source = notify.source().map(ToString::to_string);
        let wrapped = BackendError::from(notify);
        assert_eq!(wrapped.to_string(), expected);
        assert_eq!(wrapped.source().map(ToString::to_string), source);

        let zip = zip::result::ZipError::Io(std::io::Error::other("archive read failed"));
        let expected = zip.to_string();
        let source = zip.source().map(ToString::to_string);
        let wrapped = BackendError::from(zip);
        assert_eq!(wrapped.to_string(), expected);
        assert_eq!(wrapped.source().map(ToString::to_string), source);
        assert!(wrapped.source().unwrap().is::<std::io::Error>());
        let wrapped = BackendError::from(zip::result::ZipError::FileNotFound);
        assert!(wrapped.source().is_none());
    }
}
