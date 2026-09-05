use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::backend::error::{BackendError, Result};

const RELEASE_API: &str =
    "https://api.github.com/repos/aaditagrawal/typsmthng-desktop/releases/latest";
const MAX_RELEASE_RESPONSE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub checksums_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate {
        current: Version,
    },
    Available {
        current: Version,
        latest: Version,
        release_url: String,
        asset: Option<ReleaseAsset>,
    },
}

#[derive(Debug, Clone)]
pub struct UpdateClient {
    endpoint: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

impl Default for UpdateClient {
    fn default() -> Self {
        Self {
            endpoint: std::env::var("TYPSMTHNG_UPDATE_API_URL")
                .unwrap_or_else(|_| RELEASE_API.to_string()),
        }
    }
}

impl UpdateClient {
    pub fn check(&self, current: &str) -> Result<UpdateStatus> {
        let current = Version::parse(current)
            .map_err(|error| BackendError::Network(format!("invalid app version: {error}")))?;
        let mut response = ureq::get(&self.endpoint)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "typsmthng-gtk")
            .call()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let release: GithubRelease = response
            .body_mut()
            .with_config()
            .limit(MAX_RELEASE_RESPONSE)
            .read_json()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        status_from_release(current, release)
    }

    pub fn download(&self, asset: &ReleaseAsset, destination: impl AsRef<Path>) -> Result<PathBuf> {
        let destination = destination.as_ref();
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        let mut response = ureq::get(&asset.download_url)
            .header("Accept", "application/octet-stream")
            .header("User-Agent", "typsmthng-gtk")
            .call()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let mut output =
            NamedTempFile::new_in(parent).map_err(|error| BackendError::io(parent, error))?;
        let mut digest = Sha256::new();
        let mut reader = response.body_mut().as_reader();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| BackendError::Network(error.to_string()))?;
            if read == 0 {
                break;
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| BackendError::io(output.path(), error))?;
            digest.update(&buffer[..read]);
        }
        let actual = format!("{:x}", digest.finalize());
        let checksums_url = asset
            .checksums_url
            .as_deref()
            .ok_or_else(|| BackendError::Network("release does not publish SHA256SUMS".into()))?;
        let mut checksums_response = ureq::get(checksums_url)
            .header("Accept", "application/octet-stream")
            .header("User-Agent", "typsmthng-gtk")
            .call()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let checksums = checksums_response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_string()
            .map_err(|error| BackendError::Network(error.to_string()))?;
        let expected = checksum_for(&checksums, &asset.name).ok_or_else(|| {
            BackendError::Network(format!("SHA256SUMS has no entry for {}", asset.name))
        })?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(BackendError::Network(format!(
                "checksum mismatch for {}",
                asset.name
            )));
        }
        output
            .as_file_mut()
            .sync_all()
            .map_err(|error| BackendError::io(output.path(), error))?;
        #[cfg(unix)]
        if asset.name.to_ascii_lowercase().ends_with(".appimage") {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = output
                .as_file()
                .metadata()
                .map_err(|error| BackendError::io(output.path(), error))?
                .permissions();
            permissions.set_mode(0o755);
            output
                .as_file()
                .set_permissions(permissions)
                .map_err(|error| BackendError::io(output.path(), error))?;
        }
        output
            .persist(destination)
            .map_err(|error| BackendError::io(destination, error.error))?;
        Ok(destination.to_path_buf())
    }
}

fn status_from_release(current: Version, release: GithubRelease) -> Result<UpdateStatus> {
    let latest_text = release.tag_name.trim_start_matches('v');
    let latest = Version::parse(latest_text).map_err(|error| {
        BackendError::Network(format!(
            "release tag {:?} is not a semantic version: {error}",
            release.tag_name
        ))
    })?;
    if latest <= current {
        return Ok(UpdateStatus::UpToDate { current });
    }
    Ok(UpdateStatus::Available {
        current,
        latest,
        release_url: release.html_url,
        asset: choose_asset(release.assets),
    })
}

fn choose_asset(assets: Vec<GithubAsset>) -> Option<ReleaseAsset> {
    let checksums_url = assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case("SHA256SUMS"))
        .map(|asset| asset.browser_download_url.clone());
    let suffix = if cfg!(target_os = "windows") {
        ".exe"
    } else if cfg!(target_os = "macos") {
        ".dmg"
    } else {
        ".appimage"
    };
    assets
        .into_iter()
        .filter(|asset| {
            let name = asset.name.to_ascii_lowercase();
            if !name.ends_with(suffix) {
                return false;
            }
            if cfg!(target_os = "macos") {
                name.contains("universal")
                    || if cfg!(target_arch = "aarch64") {
                        name.contains("macos-arm64") || name.contains("macos-aarch64")
                    } else {
                        name.contains("macos-x64") || name.contains("macos-x86_64")
                    }
            } else {
                true
            }
        })
        .max_by_key(|asset| {
            let name = asset.name.to_ascii_lowercase();
            usize::from(name.starts_with("typsmthng-") && !name.starts_with("stable-"))
        })
        .map(|asset| ReleaseAsset {
            name: asset.name,
            download_url: asset.browser_download_url,
            checksums_url,
        })
}

fn checksum_for<'a>(manifest: &'a str, name: &str) -> Option<&'a str> {
    manifest.lines().find_map(|line| {
        let (checksum, filename) = line.split_once(char::is_whitespace)?;
        let filename = filename.trim_start().trim_start_matches('*');
        (filename == name
            && checksum.len() == 64
            && checksum.bytes().all(|c| c.is_ascii_hexdigit()))
        .then_some(checksum)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str, assets: &[&str]) -> GithubRelease {
        GithubRelease {
            tag_name: version.to_string(),
            html_url: "https://example.test/release".to_string(),
            assets: assets
                .iter()
                .map(|name| GithubAsset {
                    name: (*name).to_string(),
                    browser_download_url: format!("https://example.test/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn compares_semantic_versions() {
        let status = status_from_release(Version::new(1, 2, 3), release("v1.3.0", &[])).unwrap();
        assert!(matches!(
            status,
            UpdateStatus::Available { latest, .. } if latest == Version::new(1, 3, 0)
        ));
        assert!(matches!(
            status_from_release(Version::new(1, 3, 0), release("1.3.0", &[])).unwrap(),
            UpdateStatus::UpToDate { .. }
        ));
    }

    #[test]
    fn picks_the_native_installer_asset() {
        let assets = vec![
            GithubAsset {
                name: "typsmthng-linux-x86_64.AppImage".into(),
                browser_download_url: "linux".into(),
            },
            GithubAsset {
                name: "SHA256SUMS".into(),
                browser_download_url: "checksums".into(),
            },
            GithubAsset {
                name: "typsmthng-windows-x86_64.exe".into(),
                browser_download_url: "windows".into(),
            },
            GithubAsset {
                name: "typsmthng-macos-universal.dmg".into(),
                browser_download_url: "macos".into(),
            },
        ];
        let selected = choose_asset(assets).unwrap();
        if cfg!(target_os = "windows") {
            assert_eq!(selected.download_url, "windows");
        } else if cfg!(target_os = "macos") {
            assert_eq!(selected.download_url, "macos");
        } else {
            assert_eq!(selected.download_url, "linux");
        }
        assert_eq!(selected.checksums_url.as_deref(), Some("checksums"));
    }

    #[test]
    fn parses_checksum_manifest_entries() {
        let hash = "a".repeat(64);
        let manifest = format!("{hash}  typsmthng-linux-x86_64.AppImage\n");
        assert_eq!(
            checksum_for(&manifest, "typsmthng-linux-x86_64.AppImage"),
            Some(hash.as_str())
        );
    }
}
