//! Cross-platform application backend.

pub mod archive;
pub mod autosave;
pub mod error;
pub mod fonts;
pub mod latex;
pub mod model;
pub mod paths;
pub mod persistence;
pub mod project;
pub mod typst;
pub mod universe;
pub mod update;
pub mod watcher;

pub use archive::{
    export_project, export_projects, import_project, import_projects, ArchiveLimits,
};
pub use autosave::{AutosaveQueue, PendingWrite};
pub use error::{BackendError, Result};
pub use fonts::{extract_typst_font_families, GoogleFontCache};
pub use latex::{convert_latex_to_typst, ConversionMetadata, ConversionResult, ConversionWarning};
pub use model::*;
pub use persistence::StateStore;
pub use project::Project;
pub use typst::{
    CompileOptions, CompileOutput, InlineNote, SvgPage, TypstTool, REQUIRED_TYPST_VERSION,
};
pub use universe::{UniverseClient, UniverseTemplate};
pub use update::{ReleaseAsset, UpdateClient, UpdateStatus};
pub use watcher::{
    text_changed_since, ExternalEvent, ExternalEventKind, ExternalWatcher, FileFingerprint,
};
