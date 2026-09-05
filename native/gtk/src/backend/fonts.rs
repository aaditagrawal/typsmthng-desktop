use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use directories::BaseDirs;
use regex::Regex;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::backend::error::{BackendError, Result};
use crate::backend::{EntryKind, FileContent, Project};

const GOOGLE_VARIANTS: &str = "100,100italic,200,200italic,300,300italic,400,400italic,500,500italic,600,600italic,700,700italic,800,800italic,900,900italic";
static FAILED_FAMILIES: LazyLock<Mutex<HashMap<PathBuf, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static FONT_HTTP: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
});
const RETRY_DELAY: Duration = Duration::from_secs(60);
const MAX_FONT_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GoogleFontCache {
    directory: PathBuf,
}

/// Result of background font discovery. Only changes to actual font files
/// require another preview; manifest writes alone do not affect rendering.
#[derive(Debug, Clone)]
pub struct FontPreparation {
    pub directory: Option<PathBuf>,
    pub changed: bool,
}

type FontRevision = Vec<(OsString, u64, Option<SystemTime>)>;

impl Default for GoogleFontCache {
    fn default() -> Self {
        let directory = BaseDirs::new()
            .map(|base| base.cache_dir().join("dev.typsmthng.desktop/google-fonts"))
            .unwrap_or_else(|| std::env::temp_dir().join("typsmthng-google-fonts"));
        Self { directory }
    }
}

impl GoogleFontCache {
    /// Returns an existing cache without discovery, network access, or directory creation.
    pub fn cached_directory(&self) -> Option<PathBuf> {
        self.directory.is_dir().then(|| self.directory.clone())
    }

    /// Run on a worker thread; schedule a fresh preview only when `changed` is true.
    pub fn prepare_project_report(
        &self,
        project: &Project,
        current_source: &str,
    ) -> Result<FontPreparation> {
        let before = self.font_revision()?;
        self.prepare_project(project, current_source)?;
        let after = self.font_revision()?;
        Ok(FontPreparation {
            directory: self.cached_directory(),
            changed: before != after,
        })
    }

    fn font_revision(&self) -> Result<FontRevision> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(BackendError::io(&self.directory, error)),
        };
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| BackendError::io(&self.directory, error))?;
            let path = entry.path();
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
                })
            {
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|error| BackendError::io(&path, error))?;
            if metadata.is_file() {
                files.push((entry.file_name(), metadata.len(), metadata.modified().ok()));
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    }

    pub fn prepare_project(
        &self,
        project: &Project,
        current_source: &str,
    ) -> Result<Option<PathBuf>> {
        let mut families = extract_typst_font_families(current_source)
            .into_iter()
            .collect::<BTreeSet<_>>();
        for entry in project.entries(false)? {
            if entry.kind != EntryKind::File || !entry.path.ends_with(".typ") {
                continue;
            }
            if let Ok(file) = project.read_file(&entry.path) {
                if let FileContent::Text(text) = file.content {
                    families.extend(extract_typst_font_families(&text));
                }
            }
        }
        if families.is_empty() {
            return Ok(None);
        }
        fs::create_dir_all(&self.directory)
            .map_err(|error| BackendError::io(&self.directory, error))?;
        let missing = families
            .into_iter()
            .take(12)
            .filter(|family| !self.family_is_cached(&self.family_manifest(family)))
            .collect::<Vec<_>>();
        // Network misses are independent. Keep concurrency bounded and wait for
        // whole families before compiling so font selection remains deterministic.
        prepare_in_parallel(&missing, |family| {
            self.prepare_family(family, || self.download_family(family));
        });
        Ok(Some(self.directory.clone()))
    }

    fn prepare_family(&self, family: &str, download: impl FnOnce() -> Result<()>) {
        let manifest = self.family_manifest(family);
        if self.family_is_cached(&manifest) {
            return;
        }
        {
            let mut failures = FAILED_FAMILIES
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            failures.retain(|_, failed_at| failed_at.elapsed() < RETRY_DELAY);
            if failures.contains_key(&manifest) {
                return;
            }
        }
        if download().is_err() {
            FAILED_FAMILIES
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(manifest, Instant::now());
        }
    }

    fn family_manifest(&self, family: &str) -> PathBuf {
        let hash = format!("{:x}", Sha256::digest(family.as_bytes()));
        self.directory.join(format!(".family-{hash}.json"))
    }

    fn family_is_cached(&self, manifest: &Path) -> bool {
        let Ok(bytes) = fs::read(manifest) else {
            return false;
        };
        let Ok(urls) = serde_json::from_slice::<Vec<String>>(&bytes) else {
            return false;
        };
        !urls.is_empty() && urls.iter().all(|url| self.font_destination(url).is_file())
    }

    fn download_family(&self, family: &str) -> Result<()> {
        let encoded = url::form_urlencoded::byte_serialize(family.as_bytes()).collect::<String>();
        let css_url = format!(
            "https://fonts.googleapis.com/css?family={encoded}:{GOOGLE_VARIANTS}&display=swap"
        );
        let mut response = FONT_HTTP
            .get(&css_url)
            .header("User-Agent", "Mozilla/5.0 typsmthng/0.1")
            .call()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let css = response
            .body_mut()
            .with_config()
            .limit(512 * 1024)
            .read_to_string()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let urls = Regex::new(r"url\((https://fonts\.gstatic\.com/[^)]+)\)")
            .unwrap()
            .captures_iter(&css)
            .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_string()))
            .collect::<BTreeSet<_>>();
        if urls.is_empty() {
            return Err(BackendError::Network(format!(
                "No downloadable fonts for {family}"
            )));
        }
        for url in &urls {
            self.download_font(url)?;
        }
        let manifest = self.family_manifest(family);
        let mut temporary = NamedTempFile::new_in(&self.directory)
            .map_err(|error| BackendError::io(&self.directory, error))?;
        serde_json::to_writer(&mut temporary, &urls)
            .map_err(|error| BackendError::Network(error.to_string()))?;
        temporary
            .persist(&manifest)
            .map_err(|error| BackendError::io(&manifest, error.error))?;
        Ok(())
    }

    fn font_destination(&self, url: &str) -> PathBuf {
        let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
        let extension = Path::new(url)
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| matches!(*value, "ttf" | "otf"))
            .unwrap_or("ttf");
        self.directory.join(format!("{hash}.{extension}"))
    }

    fn download_font(&self, url: &str) -> Result<()> {
        let destination = self.font_destination(url);
        if destination.is_file() {
            return Ok(());
        }
        let mut response = FONT_HTTP
            .get(url)
            .header("User-Agent", "typsmthng-gtk")
            .call()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let mut temporary = NamedTempFile::new_in(&self.directory)
            .map_err(|error| BackendError::io(&self.directory, error))?;
        std::io::copy(
            &mut response
                .body_mut()
                .with_config()
                .limit(MAX_FONT_BYTES)
                .reader(),
            &mut temporary,
        )
        .map_err(|error| BackendError::io(&destination, error))?;
        temporary
            .persist(&destination)
            .map_err(|error| BackendError::io(&destination, error.error))?;
        Ok(())
    }
}

fn prepare_in_parallel(families: &[String], prepare: impl Fn(&str) + Sync) {
    if families.len() <= 1 {
        for family in families {
            prepare(family);
        }
        return;
    }
    const MAX_WORKERS: usize = 4;
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..families.len().min(MAX_WORKERS) {
            let prepare = &prepare;
            let next = &next;
            scope.spawn(move || {
                while let Some(family) =
                    families.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    prepare(family);
                }
            });
        }
    });
}

pub fn extract_typst_font_families(source: &str) -> Vec<String> {
    static FONT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"font\s*:\s*(?:\([^)]*\)|\[[^]]*\]|[^,\n;]+)"#).unwrap());
    static QUOTED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"["']([^"']{2,80})["']"#).unwrap());
    let ignored = [
        "serif",
        "sans-serif",
        "monospace",
        "system-ui",
        "emoji",
        "math",
    ];
    let mut families = BTreeSet::new();
    for expression in FONT.find_iter(source) {
        for capture in QUOTED.captures_iter(expression.as_str()) {
            let family = capture[1].trim();
            if !ignored.contains(&family.to_ascii_lowercase().as_str())
                && !family.ends_with(".ttf")
                && !family.ends_with(".otf")
            {
                families.insert(family.to_string());
            }
        }
    }
    families.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_lookup_and_report_are_read_only_without_font_requests() {
        let directory = tempfile::tempdir().unwrap();
        let cache = GoogleFontCache {
            directory: directory.path().join("cache"),
        };
        let project = Project::create(directory.path(), "project").unwrap();
        assert!(cache.cached_directory().is_none());
        let report = cache
            .prepare_project_report(&project, "Plain text")
            .unwrap();
        assert!(!report.changed);
        assert!(report.directory.is_none());
        assert!(!cache.directory.exists());
    }

    #[test]
    fn font_revision_tracks_fonts_but_not_manifests_or_partial_downloads() {
        let directory = tempfile::tempdir().unwrap();
        let cache = GoogleFontCache {
            directory: directory.path().to_path_buf(),
        };
        let initial = cache.font_revision().unwrap();
        fs::write(directory.path().join(".family-example.json"), "[]").unwrap();
        fs::write(directory.path().join(".tmp-download"), "bytes").unwrap();
        assert_eq!(initial, cache.font_revision().unwrap());
        let font = directory.path().join("example.ttf");
        fs::write(&font, "font").unwrap();
        let added = cache.font_revision().unwrap();
        assert_ne!(initial, added);
        assert_eq!(cache.cached_directory().as_deref(), Some(directory.path()));
        fs::write(&font, "longer-font").unwrap();
        let resized = cache.font_revision().unwrap();
        assert_ne!(added, resized);
        filetime::set_file_mtime(&font, filetime::FileTime::from_unix_time(1, 0)).unwrap();
        assert_ne!(resized, cache.font_revision().unwrap());
        fs::rename(&font, directory.path().join("renamed.otf")).unwrap();
        assert_ne!(resized, cache.font_revision().unwrap());
        fs::remove_file(directory.path().join("renamed.otf")).unwrap();
        assert_eq!(initial, cache.font_revision().unwrap());
    }

    #[test]
    fn cold_font_families_use_four_workers_and_finish_before_returning() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let families = (0..12).map(|index| index.to_string()).collect::<Vec<_>>();
        let completed = AtomicUsize::new(0);
        let started = AtomicUsize::new(0);
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        prepare_in_parallel(&families, |_| {
            maximum.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            let position = started.fetch_add(1, Ordering::SeqCst);
            if position < 4 {
                let deadline = Instant::now() + Duration::from_secs(3);
                while started.load(Ordering::SeqCst) < 4 {
                    assert!(Instant::now() < deadline, "font requests were serialized");
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            completed.fetch_add(1, Ordering::SeqCst);
            active.fetch_sub(1, Ordering::SeqCst);
        });
        assert_eq!(maximum.load(Ordering::SeqCst), 4);
        assert_eq!(completed.load(Ordering::SeqCst), families.len());
    }

    #[test]
    fn warm_family_skips_download_and_missing_fonts_invalidate_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let cache = GoogleFontCache {
            directory: directory.path().to_path_buf(),
        };
        let url = "https://fonts.gstatic.com/example.ttf";
        let manifest = cache.family_manifest("Cached Example");
        fs::write(&manifest, serde_json::to_vec(&[url]).unwrap()).unwrap();
        fs::write(cache.font_destination(url), b"cached-font").unwrap();
        cache.prepare_family("Cached Example", || {
            panic!("warm cache must not request network")
        });
        fs::remove_file(cache.font_destination(url)).unwrap();
        let requested = std::cell::Cell::new(false);
        cache.prepare_family("Cached Example", || {
            requested.set(true);
            Ok(())
        });
        assert!(requested.get(), "deleted cache files must be fetched again");
    }

    #[test]
    fn failed_family_lookup_is_throttled_but_retries_after_delay() {
        let directory = tempfile::tempdir().unwrap();
        let cache = GoogleFontCache {
            directory: directory.path().to_path_buf(),
        };
        cache.prepare_family("Offline Example", || {
            Err(BackendError::Network("offline".into()))
        });
        cache.prepare_family("Offline Example", || {
            panic!("must not retry on every keystroke")
        });
        FAILED_FAMILIES.lock().unwrap().insert(
            cache.family_manifest("Offline Example"),
            Instant::now() - RETRY_DELAY,
        );
        let requested = std::cell::Cell::new(false);
        cache.prepare_family("Offline Example", || {
            requested.set(true);
            Ok(())
        });
        assert!(requested.get());
    }

    #[test]
    #[ignore = "manual performance measurement"]
    fn benchmark_warm_font_preparation() {
        let directory = tempfile::tempdir().unwrap();
        let cache = GoogleFontCache {
            directory: directory.path().join("cache"),
        };
        fs::create_dir_all(&cache.directory).unwrap();
        let project = Project::create(directory.path(), "project").unwrap();
        let source = "#set text(font: \"Cached Example\")";
        fs::write(project.root().join("main.typ"), source).unwrap();
        let url = "https://fonts.gstatic.com/example.ttf";
        fs::write(
            cache.family_manifest("Cached Example"),
            serde_json::to_vec(&[url]).unwrap(),
        )
        .unwrap();
        fs::write(cache.font_destination(url), b"cached-font").unwrap();
        cache.prepare_project(&project, source).unwrap();
        let started = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(cache.prepare_project(&project, source).unwrap());
        }
        eprintln!(
            "100 warm project font preparations: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn extracts_declared_font_families() {
        assert_eq!(
            extract_typst_font_families(
                "#set text(font: (\"Inter\", \"Noto Sans\"))\n#text(font: \"serif\")[x]"
            ),
            vec!["Inter", "Noto Sans"]
        );
    }
}
