//! Google Fonts family catalog used by the font pickers.
//!
//! The public `fonts.google.com/metadata/fonts` document lists every family
//! without an API key. It is large (several megabytes), so it is reduced to a
//! compact [`FontCatalog`] and cached on disk. Fetching is blocking and must
//! run on a worker thread; searching is cheap and can run anywhere.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::backend::error::{BackendError, Result};
use crate::backend::fonts::{GoogleFontCache, FONT_HTTP};

pub const CATALOG_URL: &str = "https://fonts.google.com/metadata/fonts";
/// Refresh the cached family list weekly; new families are rare.
pub const CATALOG_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CATALOG_FILE: &str = "catalog-v1.json";
const MAX_CATALOG_BYTES: u64 = 32 * 1024 * 1024;
const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontCategory {
    Serif,
    SansSerif,
    Monospace,
    Display,
    Handwriting,
    Other,
}

impl FontCategory {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "serif" => Self::Serif,
            "sans serif" | "sans-serif" => Self::SansSerif,
            "monospace" => Self::Monospace,
            "display" => Self::Display,
            "handwriting" => Self::Handwriting,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Serif => "Serif",
            Self::SansSerif => "Sans serif",
            Self::Monospace => "Monospace",
            Self::Display => "Display",
            Self::Handwriting => "Handwriting",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoogleFontFamily {
    pub family: String,
    pub category: FontCategory,
    /// Static styles such as `400`, `400i`, `700`.
    pub variants: Vec<String>,
    /// Variable axis tags such as `wght` or `opsz`.
    pub axes: Vec<String>,
    /// Popularity rank; lower is more popular.
    pub popularity: u32,
}

impl GoogleFontFamily {
    pub fn has_italic(&self) -> bool {
        self.variants.iter().any(|variant| variant.ends_with('i'))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FontCatalog {
    /// Seconds since the Unix epoch when the catalog was downloaded.
    pub fetched_at: u64,
    pub families: Vec<GoogleFontFamily>,
}

impl FontCatalog {
    /// Parse the raw metadata document, tolerating Google's `)]}'` XSSI guard.
    pub fn from_metadata(text: &str, fetched_at: u64) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Metadata {
            family_metadata_list: Vec<RawFamily>,
        }
        #[derive(Deserialize)]
        struct RawFamily {
            family: String,
            #[serde(default)]
            category: String,
            #[serde(default)]
            fonts: serde_json::Map<String, serde_json::Value>,
            #[serde(default)]
            axes: Vec<RawAxis>,
            #[serde(default)]
            popularity: Option<u32>,
        }
        #[derive(Deserialize)]
        struct RawAxis {
            tag: String,
        }

        let start = text
            .find('{')
            .ok_or_else(|| BackendError::Network("font catalog is not JSON".into()))?;
        let metadata: Metadata =
            serde_json::from_str(&text[start..]).map_err(|source| BackendError::Json {
                path: PathBuf::from(CATALOG_URL),
                source,
            })?;
        let mut families = metadata
            .family_metadata_list
            .into_iter()
            .filter(|raw| !raw.family.trim().is_empty())
            .map(|raw| GoogleFontFamily {
                family: raw.family,
                category: FontCategory::parse(&raw.category),
                variants: raw.fonts.into_iter().map(|(key, _)| key).collect(),
                axes: raw.axes.into_iter().map(|axis| axis.tag).collect(),
                popularity: raw.popularity.unwrap_or(u32::MAX),
            })
            .collect::<Vec<_>>();
        families.sort_by(|left, right| left.family.cmp(&right.family));
        families.dedup_by(|left, right| left.family == right.family);
        Ok(Self {
            fetched_at,
            families,
        })
    }

    pub fn is_stale(&self, now: SystemTime) -> bool {
        let fetched = UNIX_EPOCH + Duration::from_secs(self.fetched_at);
        now.duration_since(fetched)
            .is_ok_and(|age| age >= CATALOG_TTL)
    }

    pub fn find(&self, family: &str) -> Option<&GoogleFontFamily> {
        self.families
            .iter()
            .find(|candidate| candidate.family.eq_ignore_ascii_case(family))
    }
}

/// Where a catalog came from, so the UI can mention offline fallbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogOrigin {
    FreshCache,
    Downloaded,
    /// The download failed; an expired cache is returned instead.
    StaleCache,
}

impl GoogleFontCache {
    fn catalog_path(&self) -> PathBuf {
        self.directory().join(CATALOG_FILE)
    }

    /// Read the on-disk catalog regardless of age, without network access.
    pub fn cached_catalog(&self) -> Option<FontCatalog> {
        read_catalog(&self.catalog_path())
    }

    /// Blocking: returns a fresh cached catalog, or downloads a new one.
    /// When offline, an expired cache is still returned. Never call this on
    /// the GTK main thread.
    pub fn load_catalog(&self) -> Result<(FontCatalog, CatalogOrigin)> {
        self.load_catalog_with(SystemTime::now(), fetch_metadata)
    }

    fn load_catalog_with(
        &self,
        now: SystemTime,
        fetch: impl FnOnce() -> Result<String>,
    ) -> Result<(FontCatalog, CatalogOrigin)> {
        let cached = self.cached_catalog();
        if let Some(catalog) = cached.as_ref().filter(|catalog| !catalog.is_stale(now)) {
            return Ok((catalog.clone(), CatalogOrigin::FreshCache));
        }
        let downloaded = fetch().and_then(|text| {
            let seconds = now
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or_default();
            FontCatalog::from_metadata(&text, seconds)
        });
        match (downloaded, cached) {
            (Ok(catalog), _) => {
                self.write_catalog(&catalog)?;
                Ok((catalog, CatalogOrigin::Downloaded))
            }
            (Err(_), Some(stale)) => Ok((stale, CatalogOrigin::StaleCache)),
            (Err(error), None) => Err(error),
        }
    }

    fn write_catalog(&self, catalog: &FontCatalog) -> Result<()> {
        let directory = self.directory();
        fs::create_dir_all(directory).map_err(|error| BackendError::io(directory, error))?;
        let path = self.catalog_path();
        let mut temporary =
            NamedTempFile::new_in(directory).map_err(|error| BackendError::io(directory, error))?;
        serde_json::to_writer(&mut temporary, catalog).map_err(|source| BackendError::Json {
            path: path.clone(),
            source,
        })?;
        temporary
            .persist(&path)
            .map_err(|error| BackendError::io(&path, error.error))?;
        Ok(())
    }
}

fn read_catalog(path: &Path) -> Option<FontCatalog> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice::<FontCatalog>(&bytes)
        .ok()
        .filter(|catalog| !catalog.families.is_empty())
}

fn fetch_metadata() -> Result<String> {
    let mut response = FONT_HTTP
        .get(CATALOG_URL)
        .header("User-Agent", "Mozilla/5.0 typsmthng/0.1")
        .config()
        .timeout_global(Some(CATALOG_TIMEOUT))
        .build()
        .call()
        .map_err(|error| BackendError::Network(error.to_string()))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_CATALOG_BYTES)
        .read_to_string()
        .map_err(|error| BackendError::Network(error.to_string()))
}

/// Rank families for a picker query.
///
/// Tiers: exact name, name prefix, word prefix, substring (ignoring spaces),
/// then in-order fuzzy subsequence. Ties sort by popularity then name. An
/// empty query lists every family in the category by popularity.
pub fn search_catalog<'a>(
    catalog: &'a FontCatalog,
    query: &str,
    category: Option<FontCategory>,
) -> Vec<&'a GoogleFontFamily> {
    let query = query.trim().to_lowercase();
    let compact_query = compact(&query);
    let mut ranked = catalog
        .families
        .iter()
        .filter(|family| category.is_none_or(|category| family.category == category))
        .filter_map(|family| {
            match_tier(&family.family, &query, &compact_query).map(|tier| (tier, family))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_tier, left), (right_tier, right)| {
        left_tier
            .cmp(right_tier)
            .then(left.popularity.cmp(&right.popularity))
            .then_with(|| left.family.cmp(&right.family))
    });
    ranked.into_iter().map(|(_, family)| family).collect()
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .collect()
}

fn match_tier(name: &str, query: &str, compact_query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    let name = name.to_lowercase();
    if name == query {
        return Some(0);
    }
    if name.starts_with(query) {
        return Some(1);
    }
    if name
        .split(|character: char| character.is_whitespace() || character == '-')
        .any(|word| word.starts_with(query))
    {
        return Some(2);
    }
    let compact_name = compact(&name);
    if compact_query.is_empty() || compact_name.contains(compact_query) {
        return Some(3);
    }
    if compact_query.chars().count() >= 3 && is_subsequence(compact_query, &compact_name) {
        return Some(4);
    }
    None
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut haystack = haystack.chars();
    needle
        .chars()
        .all(|wanted| haystack.any(|character| character == wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#")]}'
{"axisRegistry": [], "familyMetadataList": [
  {"family": "Roboto", "category": "Sans Serif", "fonts": {"400": {}, "400i": {}, "700": {}},
   "axes": [{"tag": "wght", "min": 100.0, "max": 900.0}], "popularity": 1},
  {"family": "Roboto Mono", "category": "Monospace", "fonts": {"400": {}, "700": {}},
   "axes": [], "popularity": 10},
  {"family": "JetBrains Mono", "category": "Monospace", "fonts": {"400": {}, "400i": {}},
   "popularity": 60},
  {"family": "Noto Serif Roboto", "category": "Serif", "fonts": {"400": {}}, "popularity": 5},
  {"family": "Fraunces", "category": "Serif", "fonts": {"400": {}}, "popularity": 300},
  {"family": "Caveat", "category": "Handwriting", "fonts": {"400": {}}, "popularity": 90},
  {"family": "Mystery", "category": "Unknown", "fonts": {}}
], "promotedScript": null}"#;

    fn catalog() -> FontCatalog {
        FontCatalog::from_metadata(FIXTURE, 1_000).unwrap()
    }

    fn names<'a>(families: &[&'a GoogleFontFamily]) -> Vec<&'a str> {
        families
            .iter()
            .map(|family| family.family.as_str())
            .collect()
    }

    #[test]
    fn parses_metadata_behind_xssi_guard() {
        let catalog = catalog();
        assert_eq!(catalog.families.len(), 7);
        let roboto = catalog.find("roboto").unwrap();
        assert_eq!(roboto.category, FontCategory::SansSerif);
        assert_eq!(roboto.axes, vec!["wght"]);
        assert!(roboto.has_italic());
        assert!(!catalog.find("Roboto Mono").unwrap().has_italic());
        let mystery = catalog.find("Mystery").unwrap();
        assert_eq!(mystery.category, FontCategory::Other);
        assert_eq!(mystery.popularity, u32::MAX);
        assert!(FontCatalog::from_metadata("<html>", 0).is_err());
    }

    #[test]
    fn ranks_prefix_then_word_prefix_then_substring_then_popularity() {
        let catalog = catalog();
        assert_eq!(
            names(&search_catalog(&catalog, "rob", None)),
            vec!["Roboto", "Roboto Mono", "Noto Serif Roboto"]
        );
        assert_eq!(
            names(&search_catalog(&catalog, "mono", None)),
            vec!["Roboto Mono", "JetBrains Mono"]
        );
        assert_eq!(
            names(&search_catalog(&catalog, "jetbrainsmono", None)),
            vec!["JetBrains Mono"]
        );
        assert_eq!(
            names(&search_catalog(&catalog, "frnc", None)),
            vec!["Fraunces"]
        );
        assert!(search_catalog(&catalog, "zzz", None).is_empty());
    }

    #[test]
    fn filters_by_category_and_lists_all_for_empty_query() {
        let catalog = catalog();
        assert_eq!(
            names(&search_catalog(&catalog, "", Some(FontCategory::Monospace))),
            vec!["Roboto Mono", "JetBrains Mono"]
        );
        assert_eq!(
            names(&search_catalog(&catalog, "  ", None))[..3],
            ["Roboto", "Noto Serif Roboto", "Roboto Mono"]
        );
        assert_eq!(
            names(&search_catalog(
                &catalog,
                "roboto",
                Some(FontCategory::Serif)
            )),
            vec!["Noto Serif Roboto"]
        );
    }

    #[test]
    fn catalog_cache_honours_ttl_and_falls_back_offline() {
        let directory = tempfile::tempdir().unwrap();
        let cache = GoogleFontCache::with_directory(directory.path().join("cache"));
        let offline = || Err(BackendError::Network("offline".into()));
        assert!(cache.load_catalog_with(SystemTime::now(), offline).is_err());

        let fetched = UNIX_EPOCH + Duration::from_secs(1_000);
        let (downloaded, origin) = cache
            .load_catalog_with(fetched, || Ok(FIXTURE.to_string()))
            .unwrap();
        assert_eq!(origin, CatalogOrigin::Downloaded);
        assert_eq!(cache.cached_catalog().unwrap(), downloaded);

        let (_, origin) = cache
            .load_catalog_with(fetched + Duration::from_secs(60), || {
                panic!("fresh cache must not hit the network")
            })
            .unwrap();
        assert_eq!(origin, CatalogOrigin::FreshCache);

        let expired = fetched + CATALOG_TTL;
        let (stale, origin) = cache.load_catalog_with(expired, offline).unwrap();
        assert_eq!(origin, CatalogOrigin::StaleCache);
        assert_eq!(stale.families.len(), 7);

        let (_, origin) = cache
            .load_catalog_with(expired, || Ok(FIXTURE.to_string()))
            .unwrap();
        assert_eq!(origin, CatalogOrigin::Downloaded);
        assert!(!cache.cached_catalog().unwrap().is_stale(expired));
    }

    #[test]
    #[ignore = "requires network access to fonts.google.com"]
    fn live_catalog_downloads_and_parses() {
        let text = fetch_metadata().unwrap();
        let catalog = FontCatalog::from_metadata(&text, 0).unwrap();
        assert!(catalog.families.len() > 1_000, "{}", catalog.families.len());
        let mono = search_catalog(&catalog, "jetbrains", Some(FontCategory::Monospace));
        assert_eq!(mono.first().unwrap().family, "JetBrains Mono");
        assert!(search_catalog(&catalog, "", None)[0].popularity < 10);
    }
}
