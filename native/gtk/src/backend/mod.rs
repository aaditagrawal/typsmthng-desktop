//! Cross-platform application backend.

#[cfg(feature = "desktop")]
pub mod app_fonts;
pub mod archive;
mod compile_options;
pub mod error;
pub mod font_catalog;
pub mod fonts;
pub mod latex;
pub mod model;
pub mod paths;
pub mod persistence;
pub mod preview;
pub mod project;
pub mod typst;
pub mod universe;
pub mod update;
pub mod update_install;
pub mod watcher;

#[cfg(feature = "desktop")]
pub use app_fonts::{
    ensure_registered, family_is_available, list_local_families, register_app_font_files,
};
pub use archive::{
    export_project, export_projects, import_project, import_projects, ArchiveLimits,
};
pub use error::{BackendError, Result};
pub use font_catalog::{
    search_catalog, CatalogOrigin, FontCatalog, FontCategory, GoogleFontFamily,
};
pub use fonts::{extract_typst_font_families, GoogleFontCache};
pub use latex::{convert_latex_to_typst, ConversionMetadata, ConversionResult, ConversionWarning};
pub use model::*;
pub use persistence::StateStore;
pub use project::Project;
pub use typst::{
    CompileOptions, CompileOutput, ExportArtifact, ExportFormat, InlineNote, PdfStandard, SvgPage,
    TypstTool, REQUIRED_TYPST_VERSION,
};
pub use universe::{UniverseClient, UniverseTemplate};
pub use update::{ReleaseAsset, UpdateClient, UpdateStatus};
pub use watcher::{ExternalEvent, ExternalEventKind, ExternalWatcher, FileFingerprint};
