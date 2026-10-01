use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{atomic::AtomicBool, Arc};

use super::{BackendError, Result};

const ENVIRONMENT_SETTINGS: [&str; 6] = [
    "TYPST_FONT_PATHS",
    "TYPST_IGNORE_SYSTEM_FONTS",
    "TYPST_IGNORE_EMBEDDED_FONTS",
    "TYPST_PACKAGE_PATH",
    "TYPST_PACKAGE_CACHE_PATH",
    "SOURCE_DATE_EPOCH",
];

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub font_paths: Vec<PathBuf>,
    pub ignore_system_fonts: bool,
    pub ignore_embedded_fonts: bool,
    pub page_preamble: Option<String>,
    pub package_path: Option<PathBuf>,
    pub package_cache_path: Option<PathBuf>,
    pub creation_timestamp: Option<i64>,
    pub cancellation: Option<Arc<AtomicBool>>,
    /// Read Typst's environment defaults until `resolved()` freezes them.
    /// Set to false for a compilation independent of these environment settings.
    pub inherit_environment: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            font_paths: Vec::new(),
            ignore_system_fonts: false,
            ignore_embedded_fonts: false,
            page_preamble: None,
            package_path: None,
            package_cache_path: None,
            creation_timestamp: None,
            cancellation: None,
            inherit_environment: true,
        }
    }
}

impl CompileOptions {
    /// Resolve the same defaults as Typst's CLI. Explicit directories, paths,
    /// timestamps, and true ignore-font flags take precedence over environment
    /// values. The returned snapshot can be shared by preview and export.
    pub fn resolved(&self) -> Result<Self> {
        self.resolve_with(|name| std::env::var_os(name))
    }

    fn resolve_with(&self, environment: impl Fn(&str) -> Option<OsString>) -> Result<Self> {
        let mut options = self.clone();
        if options.inherit_environment {
            if options.font_paths.is_empty() {
                if let Some(paths) = environment("TYPST_FONT_PATHS") {
                    options.font_paths = std::env::split_paths(&paths).collect();
                }
            }
            if !options.ignore_system_fonts {
                options.ignore_system_fonts = environment_bool(
                    "TYPST_IGNORE_SYSTEM_FONTS",
                    environment("TYPST_IGNORE_SYSTEM_FONTS"),
                )?;
            }
            if !options.ignore_embedded_fonts {
                options.ignore_embedded_fonts = environment_bool(
                    "TYPST_IGNORE_EMBEDDED_FONTS",
                    environment("TYPST_IGNORE_EMBEDDED_FONTS"),
                )?;
            }
            options.package_path = options
                .package_path
                .or_else(|| environment("TYPST_PACKAGE_PATH").map(PathBuf::from));
            options.package_cache_path = options
                .package_cache_path
                .or_else(|| environment("TYPST_PACKAGE_CACHE_PATH").map(PathBuf::from));
            if options.creation_timestamp.is_none() {
                if let Some(timestamp) = environment("SOURCE_DATE_EPOCH") {
                    options.creation_timestamp = Some(
                        timestamp
                            .to_str()
                            .and_then(|text| text.parse().ok())
                            .ok_or(BackendError::InvalidTypstConfiguration {
                                setting: "SOURCE_DATE_EPOCH",
                                reason: "expected a signed Unix timestamp",
                            })?,
                    );
                }
            }
        }
        if let Some(timestamp) = options.creation_timestamp {
            typst_kit::datetime::Time::fixed_timestamp(timestamp).map_err(|_| {
                BackendError::InvalidTypstConfiguration {
                    setting: "creation timestamp",
                    reason: "timestamp is out of range",
                }
            })?;
        }
        options.inherit_environment = false;
        Ok(options)
    }

    /// The CLI must not re-read values that the preview has already resolved.
    pub(super) fn apply_to_command(&self, command: &mut Command) {
        debug_assert!(!self.inherit_environment);
        for name in ENVIRONMENT_SETTINGS {
            command.env_remove(name);
        }
        for path in &self.font_paths {
            command.arg("--font-path").arg(path);
        }
        if let Some(path) = &self.package_path {
            command.arg("--package-path").arg(path);
        }
        if let Some(path) = &self.package_cache_path {
            command.arg("--package-cache-path").arg(path);
        }
        if let Some(timestamp) = self.creation_timestamp {
            command.arg(format!("--creation-timestamp={timestamp}"));
        }
        if self.ignore_system_fonts {
            command.arg("--ignore-system-fonts");
        }
        if self.ignore_embedded_fonts {
            command.arg("--ignore-embedded-fonts");
        }
    }
}

fn environment_bool(setting: &'static str, value: Option<OsString>) -> Result<bool> {
    match value.as_deref().and_then(|value| value.to_str()) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        None if value.is_none() => Ok(false),
        _ => Err(BackendError::InvalidTypstConfiguration {
            setting,
            reason: "expected true or false",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_settings_override_environment_and_resolved_options_stay_frozen() {
        let options = CompileOptions {
            font_paths: vec![PathBuf::from("explicit fonts")],
            ignore_system_fonts: true,
            ignore_embedded_fonts: true,
            package_path: Some(PathBuf::from("explicit packages")),
            package_cache_path: Some(PathBuf::from("explicit cache")),
            creation_timestamp: Some(-1),
            ..Default::default()
        };
        let resolved = options
            .resolve_with(|_| Some(OsString::from("invalid environment value")))
            .unwrap();
        assert_eq!(resolved.font_paths, options.font_paths);
        assert_eq!(resolved.package_path, options.package_path);
        assert_eq!(resolved.package_cache_path, options.package_cache_path);
        assert_eq!(resolved.creation_timestamp, Some(-1));
        assert!(resolved.ignore_system_fonts && resolved.ignore_embedded_fonts);
        assert!(!resolved.inherit_environment);
        let frozen = resolved
            .resolve_with(|_| panic!("resolved options must not read the environment again"))
            .unwrap();
        assert_eq!(frozen.creation_timestamp, Some(-1));
    }

    #[test]
    fn invalid_environment_values_and_out_of_range_dates_are_reported() {
        for (name, value) in [
            ("SOURCE_DATE_EPOCH", "not a timestamp"),
            ("SOURCE_DATE_EPOCH", "9223372036854775807"),
            ("TYPST_IGNORE_SYSTEM_FONTS", "1"),
            ("TYPST_IGNORE_EMBEDDED_FONTS", "yes"),
        ] {
            let result = CompileOptions::default()
                .resolve_with(|key| (key == name).then(|| OsString::from(value)));
            assert!(
                matches!(result, Err(BackendError::InvalidTypstConfiguration { .. })),
                "{name}={value} should be rejected"
            );
        }
        assert!(CompileOptions {
            creation_timestamp: Some(i64::MAX),
            inherit_environment: false,
            ..Default::default()
        }
        .resolved()
        .is_err());
    }
}
