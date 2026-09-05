use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub path: String,
    pub name: String,
    pub kind: EntryKind,
    pub parent_path: Option<String>,
    pub extension: String,
    pub is_hidden: bool,
    pub is_binary: bool,
    pub last_modified_ms: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContent {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    pub entry: ProjectEntry,
    pub content: FileContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentDocument {
    pub path: String,
    pub last_opened_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub id: String,
    pub root_path: PathBuf,
    pub name: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub hidden_files_visible: bool,
    pub file_count: Option<usize>,
    pub last_opened_at: u64,
    pub last_file_path: Option<String>,
    #[serde(default)]
    pub recent_documents: Vec<RecentDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeWorkspace {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    #[serde(default)]
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1440,
            height: 900,
            x: None,
            y: None,
            maximized: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct AppMetadata {
    #[serde(default = "metadata_version")]
    pub version: u32,
    #[serde(default)]
    #[serde(alias = "recentVaults")]
    pub recent_projects: Vec<RecentProject>,
    #[serde(alias = "reopenLastVaultPath")]
    pub reopen_last_project_path: Option<PathBuf>,
    #[serde(default)]
    pub home_workspaces: Vec<HomeWorkspace>,
    #[serde(default)]
    pub project_workspace_assignments: BTreeMap<String, String>,
    #[serde(default)]
    pub selected_home_workspace_id: Option<String>,
    #[serde(default)]
    pub window_state: WindowState,
}

const fn metadata_version() -> u32 {
    1
}

impl Default for AppMetadata {
    fn default() -> Self {
        Self {
            version: metadata_version(),
            recent_projects: Vec::new(),
            reopen_last_project_path: None,
            home_workspaces: Vec::new(),
            project_workspace_assignments: BTreeMap::new(),
            selected_home_workspace_id: None,
            window_state: WindowState::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct UserSettings {
    pub font_size: f64,
    pub auto_compile: bool,
    #[serde(alias = "compileDelay")]
    pub compile_delay_ms: u64,
    pub line_wrapping: bool,
    pub line_numbers: bool,
    pub theme: Theme,
    pub vim_mode: bool,
    pub page_size: String,
    pub presentation_notes_layout: String,
    pub presentation_notes_font_size: u32,
    pub system_fonts_enabled: bool,
    pub google_fonts_enabled: bool,
    pub translucent: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            font_size: 15.0,
            auto_compile: true,
            compile_delay_ms: 100,
            line_wrapping: true,
            line_numbers: true,
            theme: Theme::System,
            vim_mode: false,
            page_size: "auto".into(),
            presentation_notes_layout: "auto".into(),
            presentation_notes_font_size: 17,
            system_fonts_enabled: true,
            google_fonts_enabled: true,
            translucent: true,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSearchResult {
    pub entry: ProjectEntry,
    pub score: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSearchResult {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileBundle {
    pub main_path: String,
    pub main_source: String,
    pub extra_text_files: Vec<(String, String)>,
    pub extra_binary_files: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Hint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub path: Option<PathBuf>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}
