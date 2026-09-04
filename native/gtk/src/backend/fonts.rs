use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use regex::Regex;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::backend::error::{BackendError, Result};
use crate::backend::{EntryKind, FileContent, Project};

const GOOGLE_VARIANTS: &str = "100,100italic,200,200italic,300,300italic,400,400italic,500,500italic,600,600italic,700,700italic,800,800italic,900,900italic";
const MAX_FONT_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GoogleFontCache {
    directory: PathBuf,
}

impl Default for GoogleFontCache {
    fn default() -> Self {
        let directory = BaseDirs::new()
            .map(|base| base.cache_dir().join("dev.typsmthng.desktop/google-fonts"))
            .unwrap_or_else(|| std::env::temp_dir().join("typsmthng-google-fonts"));
        Self { directory }
    }
}

impl GoogleFontCache {
    pub fn prepare_project(
        &self,
        project: &Project,
        current_source: &str,
    ) -> Result<Option<PathBuf>> {
        let mut source = current_source.to_string();
        for entry in project.entries(false)? {
            if entry.kind != EntryKind::File || !entry.path.ends_with(".typ") {
                continue;
            }
            if let Ok(file) = project.read_file(&entry.path) {
                if let FileContent::Text(text) = file.content {
                    source.push('\n');
                    source.push_str(&text);
                }
            }
        }
        let families = extract_typst_font_families(&source);
        if families.is_empty() {
            return Ok(None);
        }
        fs::create_dir_all(&self.directory)
            .map_err(|error| BackendError::io(&self.directory, error))?;
        for family in families.into_iter().take(12) {
            let _ = self.download_family(&family);
        }
        Ok(Some(self.directory.clone()))
    }

    fn download_family(&self, family: &str) -> Result<()> {
        let encoded = url::form_urlencoded::byte_serialize(family.as_bytes()).collect::<String>();
        let css_url = format!(
            "https://fonts.googleapis.com/css?family={encoded}:{GOOGLE_VARIANTS}&display=swap"
        );
        let mut response = ureq::get(&css_url)
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
        for url in urls {
            self.download_font(&url)?;
        }
        Ok(())
    }

    fn download_font(&self, url: &str) -> Result<()> {
        let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
        let extension = Path::new(url)
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| matches!(*value, "ttf" | "otf"))
            .unwrap_or("ttf");
        let destination = self.directory.join(format!("{hash}.{extension}"));
        if destination.is_file() {
            return Ok(());
        }
        let mut response = ureq::get(url)
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

pub fn extract_typst_font_families(source: &str) -> Vec<String> {
    let font = Regex::new(r#"font\s*:\s*(?:\([^)]*\)|\[[^]]*\]|[^,\n;]+)"#).unwrap();
    let quoted = Regex::new(r#"["']([^"']{2,80})["']"#).unwrap();
    let ignored = [
        "serif",
        "sans-serif",
        "monospace",
        "system-ui",
        "emoji",
        "math",
    ];
    let mut families = BTreeSet::new();
    for expression in font.find_iter(source) {
        for capture in quoted.captures_iter(expression.as_str()) {
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
    fn extracts_declared_font_families() {
        assert_eq!(
            extract_typst_font_families(
                "#set text(font: (\"Inter\", \"Noto Sans\"))\n#text(font: \"serif\")[x]"
            ),
            vec!["Inter", "Noto Sans"]
        );
    }
}
