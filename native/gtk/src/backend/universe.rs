use std::collections::HashMap;

use semver::Version;
use serde::Deserialize;

use crate::backend::error::{BackendError, Result};

const UNIVERSE_INDEX: &str = "https://packages.typst.org/preview/index.json";
const MAX_INDEX_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniverseTemplate {
    pub name: String,
    pub version: Version,
    pub description: String,
    pub spec: String,
}

#[derive(Debug, Clone)]
pub struct UniverseClient {
    endpoint: String,
}

#[derive(Debug, Deserialize)]
struct IndexEntry {
    name: String,
    version: Version,
    #[serde(default)]
    description: String,
    template: Option<TemplateMetadata>,
}

#[derive(Debug, Deserialize)]
struct TemplateMetadata {
    entrypoint: String,
}

impl Default for UniverseClient {
    fn default() -> Self {
        Self {
            endpoint: std::env::var("TYPSMTHNG_UNIVERSE_INDEX_URL")
                .unwrap_or_else(|_| UNIVERSE_INDEX.to_string()),
        }
    }
}

impl UniverseClient {
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<UniverseTemplate>> {
        let mut response = ureq::get(&self.endpoint)
            .header("Accept", "application/json")
            .header("User-Agent", "typsmthng-gtk")
            .call()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let entries: Vec<IndexEntry> = response
            .body_mut()
            .with_config()
            .limit(MAX_INDEX_SIZE)
            .read_json()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        Ok(search_entries(entries, query, limit))
    }
}

fn search_entries(entries: Vec<IndexEntry>, query: &str, limit: usize) -> Vec<UniverseTemplate> {
    let query = query.trim().to_ascii_lowercase();
    let mut latest = HashMap::<String, IndexEntry>::new();
    for entry in entries.into_iter().filter(|entry| {
        entry
            .template
            .as_ref()
            .is_some_and(|template| !template.entrypoint.is_empty())
    }) {
        if !query.is_empty()
            && !entry.name.to_ascii_lowercase().contains(&query)
            && !entry.description.to_ascii_lowercase().contains(&query)
        {
            continue;
        }
        let replace = latest
            .get(&entry.name)
            .is_none_or(|current| entry.version > current.version);
        if replace {
            latest.insert(entry.name.clone(), entry);
        }
    }
    let mut results = latest
        .into_values()
        .map(|entry| UniverseTemplate {
            spec: format!("@preview/{}:{}", entry.name, entry.version),
            name: entry.name,
            version: entry.version,
            description: entry.description,
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.name.cmp(&right.name));
    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keeps_latest_template_version_only() {
        let entries = vec![
            IndexEntry {
                name: "paper-kit".into(),
                version: Version::new(1, 0, 0),
                description: "Paper".into(),
                template: Some(TemplateMetadata {
                    entrypoint: "main.typ".into(),
                }),
            },
            IndexEntry {
                name: "paper-kit".into(),
                version: Version::new(1, 2, 0),
                description: "Paper".into(),
                template: Some(TemplateMetadata {
                    entrypoint: "main.typ".into(),
                }),
            },
            IndexEntry {
                name: "library-only".into(),
                version: Version::new(9, 0, 0),
                description: "Paper helpers".into(),
                template: None,
            },
        ];
        let found = search_entries(entries, "paper", 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].spec, "@preview/paper-kit:1.2.0");
    }
}
