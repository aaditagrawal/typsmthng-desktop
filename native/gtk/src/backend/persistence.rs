use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use tempfile::NamedTempFile;

use crate::backend::error::{BackendError, Result};
use crate::backend::model::{
    AppMetadata, HomeWorkspace, RecentDocument, RecentProject, UserSettings, WindowState,
};

const APP_IDENTIFIER: &str = "dev.typsmthng.desktop";
const CHANNEL: &str = "stable";
const MAX_RECENT_PROJECTS: usize = 24;
const MAX_RECENT_DOCUMENTS: usize = 12;

#[derive(Debug, Clone)]
pub struct StateStore {
    directory: PathBuf,
}

impl StateStore {
    pub fn discover() -> Result<Self> {
        let base = BaseDirs::new().ok_or_else(|| {
            BackendError::Process("could not determine the user data directory".into())
        })?;
        let app_data = if cfg!(target_os = "macos") {
            base.home_dir().join("Library/Application Support")
        } else if cfg!(target_os = "windows") {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| base.home_dir().join("AppData/Local"))
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| base.home_dir().join(".local/share"))
        };
        Ok(Self::new(app_data.join(APP_IDENTIFIER).join(CHANNEL)))
    }

    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn metadata_path(&self) -> PathBuf {
        self.directory.join("app-state.json")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.directory.join("user-settings.json")
    }

    pub fn load_metadata(&self) -> Result<AppMetadata> {
        let path = self.metadata_path();
        match fs::read(&path) {
            Ok(bytes) => {
                let parsed = serde_json::from_slice::<AppMetadata>(&bytes)
                    .map_err(|source| BackendError::Json { path, source })?;
                Ok(normalize_metadata(parsed))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let metadata = AppMetadata::default();
                self.save_metadata(&metadata)?;
                Ok(metadata)
            }
            Err(error) => Err(BackendError::io(path, error)),
        }
    }

    pub fn save_metadata(&self, metadata: &AppMetadata) -> Result<()> {
        let normalized = normalize_metadata(metadata.clone());
        write_json_atomic(&self.metadata_path(), &normalized)
    }

    pub fn load_settings(&self) -> Result<UserSettings> {
        let path = self.settings_path();
        match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|source| BackendError::Json { path, source })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let settings = UserSettings::default();
                self.save_settings(&settings)?;
                Ok(settings)
            }
            Err(error) => Err(BackendError::io(path, error)),
        }
    }

    pub fn save_settings(&self, settings: &UserSettings) -> Result<()> {
        write_json_atomic(&self.settings_path(), settings)
    }

    pub fn upsert_recent(
        &self,
        root: &Path,
        name: impl Into<String>,
        file_count: usize,
        last_file: Option<String>,
        activate: bool,
    ) -> Result<AppMetadata> {
        let mut metadata = self.load_metadata()?;
        let root = root
            .canonicalize()
            .map_err(|error| BackendError::io(root, error))?;
        let id = root.to_string_lossy().into_owned();
        let existing = metadata
            .recent_projects
            .iter()
            .find(|project| project.root_path == root)
            .cloned();
        let record = RecentProject {
            id,
            root_path: root.clone(),
            name: name.into(),
            favorite: existing.as_ref().is_some_and(|project| project.favorite),
            hidden_files_visible: existing
                .as_ref()
                .is_some_and(|project| project.hidden_files_visible),
            file_count: Some(file_count),
            last_opened_at: now_ms(),
            last_file_path: last_file.or_else(|| existing.as_ref()?.last_file_path.clone()),
            recent_documents: existing
                .map(|project| project.recent_documents)
                .unwrap_or_default(),
        };
        metadata
            .recent_projects
            .retain(|project| project.root_path != root);
        metadata.recent_projects.push(record);
        if activate {
            metadata.reopen_last_project_path = Some(root);
        }
        metadata = normalize_metadata(metadata);
        self.save_metadata(&metadata)?;
        Ok(metadata)
    }

    pub fn remove_recent(&self, root: &Path) -> Result<AppMetadata> {
        let root = canonical_if_present(root);
        self.update_metadata(|metadata| {
            metadata
                .recent_projects
                .retain(|project| project.root_path != root);
            if metadata.reopen_last_project_path.as_ref() == Some(&root) {
                metadata.reopen_last_project_path = None;
            }
        })
    }

    pub fn toggle_favorite(&self, root: &Path) -> Result<AppMetadata> {
        let root = canonical_if_present(root);
        self.update_metadata(|metadata| {
            if let Some(project) = metadata
                .recent_projects
                .iter_mut()
                .find(|project| project.root_path == root)
            {
                project.favorite = !project.favorite;
            }
        })
    }

    pub fn set_hidden_files_visible(&self, root: &Path, visible: bool) -> Result<AppMetadata> {
        let root = canonical_if_present(root);
        self.update_metadata(|metadata| {
            if let Some(project) = metadata
                .recent_projects
                .iter_mut()
                .find(|project| project.root_path == root)
            {
                project.hidden_files_visible = visible;
            }
        })
    }

    pub fn persist_last_file(&self, root: &Path, path: Option<String>) -> Result<AppMetadata> {
        let root = canonical_if_present(root);
        self.update_metadata(|metadata| {
            let Some(project) = metadata
                .recent_projects
                .iter_mut()
                .find(|project| project.root_path == root)
            else {
                return;
            };
            project.last_file_path = path.clone();
            if let Some(path) = path {
                project.recent_documents.push(RecentDocument {
                    path,
                    last_opened_at: now_ms(),
                });
            }
        })
    }

    pub fn set_reopen_project(&self, root: Option<PathBuf>) -> Result<AppMetadata> {
        self.update_metadata(|metadata| metadata.reopen_last_project_path = root)
    }

    pub fn save_window_state(&self, window: WindowState) -> Result<AppMetadata> {
        self.update_metadata(|metadata| metadata.window_state = window)
    }

    pub fn create_workspace(&self, name: String) -> Result<AppMetadata> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(BackendError::InvalidProjectName(name));
        }
        self.update_metadata(|metadata| {
            let now = now_ms();
            let id = format!("workspace-{now}");
            metadata.home_workspaces.push(HomeWorkspace {
                id: id.clone(),
                name,
                created_at: now,
                updated_at: now,
            });
            metadata.selected_home_workspace_id = Some(id);
        })
    }

    pub fn rename_workspace(&self, id: &str, name: String) -> Result<AppMetadata> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(BackendError::InvalidProjectName(name));
        }
        self.update_metadata(|metadata| {
            if let Some(workspace) = metadata
                .home_workspaces
                .iter_mut()
                .find(|item| item.id == id)
            {
                workspace.name = name;
                workspace.updated_at = now_ms();
            }
        })
    }

    pub fn delete_workspace(&self, id: &str) -> Result<AppMetadata> {
        self.update_metadata(|metadata| {
            metadata.home_workspaces.retain(|item| item.id != id);
            metadata
                .project_workspace_assignments
                .retain(|_, workspace| workspace != id);
            if metadata.selected_home_workspace_id.as_deref() == Some(id) {
                metadata.selected_home_workspace_id = None;
            }
        })
    }

    pub fn assign_workspace(&self, root: &Path, workspace: Option<String>) -> Result<AppMetadata> {
        let project = canonical_if_present(root).to_string_lossy().into_owned();
        self.update_metadata(|metadata| {
            if let Some(workspace) = workspace {
                metadata
                    .project_workspace_assignments
                    .insert(project, workspace);
            } else {
                metadata.project_workspace_assignments.remove(&project);
            }
        })
    }

    pub fn select_workspace(&self, id: Option<String>) -> Result<AppMetadata> {
        self.update_metadata(|metadata| metadata.selected_home_workspace_id = id)
    }

    fn update_metadata(&self, update: impl FnOnce(&mut AppMetadata)) -> Result<AppMetadata> {
        let mut metadata = self.load_metadata()?;
        update(&mut metadata);
        metadata = normalize_metadata(metadata);
        self.save_metadata(&metadata)?;
        Ok(metadata)
    }
}

fn normalize_metadata(mut metadata: AppMetadata) -> AppMetadata {
    metadata.version = 1;
    for project in &mut metadata.recent_projects {
        project
            .recent_documents
            .sort_by_key(|document| std::cmp::Reverse(document.last_opened_at));
        let mut seen = HashSet::new();
        project
            .recent_documents
            .retain(|document| seen.insert(document.path.clone()));
        project.recent_documents.truncate(MAX_RECENT_DOCUMENTS);
    }
    metadata.recent_projects.sort_by(|left, right| {
        right
            .favorite
            .cmp(&left.favorite)
            .then_with(|| right.last_opened_at.cmp(&left.last_opened_at))
    });
    metadata.recent_projects.truncate(MAX_RECENT_PROJECTS);
    let workspace_ids = metadata
        .home_workspaces
        .iter()
        .map(|workspace| workspace.id.clone())
        .collect::<HashSet<_>>();
    let project_ids = metadata
        .recent_projects
        .iter()
        .map(|project| project.root_path.to_string_lossy().into_owned())
        .collect::<HashSet<_>>();
    metadata
        .project_workspace_assignments
        .retain(|project, workspace| {
            project_ids.contains(project) && workspace_ids.contains(workspace)
        });
    if metadata
        .selected_home_workspace_id
        .as_ref()
        .is_some_and(|id| !workspace_ids.contains(id))
    {
        metadata.selected_home_workspace_id = None;
    }
    metadata
}

fn canonical_if_present(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| BackendError::UnsafePath(path.display().to_string()))?;
    fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
    let mut temporary =
        NamedTempFile::new_in(parent).map_err(|error| BackendError::io(parent, error))?;
    serde_json::to_writer_pretty(&mut temporary, value).map_err(|source| BackendError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(b"\n")
        .map_err(|error| BackendError::io(path, error))?;
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|error| BackendError::io(path, error))?;
    persist_replace(temporary, path)
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

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn settings_round_trip_and_preserve_unknown_fields() {
        let directory = tempdir().unwrap();
        let store = StateStore::new(directory.path());
        let mut settings = UserSettings::default();
        settings.extra.insert("futureOption".into(), true.into());
        store.save_settings(&settings).unwrap();
        assert_eq!(store.load_settings().unwrap(), settings);
    }

    #[test]
    fn recents_are_favorite_first_and_documents_are_deduplicated() {
        let directory = tempdir().unwrap();
        let store = StateStore::new(directory.path().join("state"));
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        store.upsert_recent(&first, "First", 1, None, true).unwrap();
        store
            .upsert_recent(&second, "Second", 2, None, true)
            .unwrap();
        store.toggle_favorite(&first).unwrap();
        store
            .persist_last_file(&first.canonicalize().unwrap(), Some("main.typ".into()))
            .unwrap();
        store
            .persist_last_file(&first.canonicalize().unwrap(), Some("main.typ".into()))
            .unwrap();
        let state = store.load_metadata().unwrap();
        assert_eq!(state.recent_projects[0].name, "First");
        assert_eq!(state.recent_projects[0].recent_documents.len(), 1);
    }

    #[test]
    fn loads_legacy_electrobun_metadata_names() {
        let directory = tempdir().unwrap();
        let store = StateStore::new(directory.path());
        fs::write(
            store.metadata_path(),
            r#"{"version":1,"recentVaults":[],"reopenLastVaultPath":null,"windowState":{"width":800,"height":600}}"#,
        )
        .unwrap();
        assert_eq!(store.load_metadata().unwrap().window_state.width, 800);
    }

    #[test]
    fn workspaces_filter_projects_without_owning_their_files() {
        let directory = tempdir().unwrap();
        let project = directory.path().join("paper");
        fs::create_dir(&project).unwrap();
        let store = StateStore::new(directory.path().join("state"));
        store
            .upsert_recent(&project, "Paper", 1, None, false)
            .unwrap();
        let state = store.create_workspace("Research".into()).unwrap();
        let workspace = state.home_workspaces[0].id.clone();
        store
            .assign_workspace(&project, Some(workspace.clone()))
            .unwrap();
        let state = store.load_metadata().unwrap();
        assert_eq!(state.selected_home_workspace_id, Some(workspace.clone()));
        assert_eq!(
            state.project_workspace_assignments.values().next(),
            Some(&workspace)
        );
        assert!(project.is_dir());
    }
}
