use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use regex::Regex;
use semver::Version;
use serde_json::Value;
use tempfile::tempdir;
use wait_timeout::ChildExt;

use crate::backend::error::{BackendError, Result};
use crate::backend::model::{Diagnostic, DiagnosticSeverity};
use crate::backend::paths::{relative_pathbuf, safe_existing_path};
use crate::backend::project::Project;

pub const REQUIRED_TYPST_VERSION: &str = "0.15.1";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct TypstTool {
    executable: PathBuf,
    version: Version,
}

#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    pub font_paths: Vec<PathBuf>,
    pub ignore_system_fonts: bool,
    pub page_preamble: Option<String>,
    pub package_path: Option<PathBuf>,
    pub package_cache_path: Option<PathBuf>,
    pub creation_timestamp: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SvgPage {
    pub page: usize,
    pub svg: String,
    pub width_points: Option<f64>,
    pub height_points: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineNote {
    pub page: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct CompileOutput<T> {
    pub artifact: Option<T>,
    pub diagnostics: Vec<Diagnostic>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

impl<T> CompileOutput<T> {
    pub fn success(&self) -> bool {
        self.artifact.is_some()
            && !self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }
}

impl TypstTool {
    pub fn detect() -> Result<Self> {
        let candidates = typst_candidates();
        let mut incompatible = None;
        for candidate in candidates {
            if !candidate.is_file() {
                continue;
            }
            match Self::probe(candidate) {
                Ok(tool) if tool.is_required_version() => return Ok(tool),
                Ok(tool) => incompatible = Some(tool.version.to_string()),
                Err(_) => continue,
            }
        }
        if let Some(found) = incompatible {
            Err(BackendError::UnsupportedTypstVersion {
                found,
                required: REQUIRED_TYPST_VERSION,
            })
        } else {
            Err(BackendError::TypstNotFound {
                required: REQUIRED_TYPST_VERSION,
            })
        }
    }

    pub fn probe(executable: impl Into<PathBuf>) -> Result<Self> {
        let executable = executable.into();
        let result = run_command(
            Command::new(&executable).arg("--version"),
            Duration::from_secs(5),
        )?;
        if result.status != Some(0) {
            return Err(BackendError::Process(result.stderr));
        }
        let version = parse_version(&result.stdout).ok_or_else(|| {
            BackendError::Process(format!(
                "unrecognized Typst version: {}",
                result.stdout.trim()
            ))
        })?;
        Ok(Self {
            executable,
            version,
        })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn is_required_version(&self) -> bool {
        self.version == Version::new(0, 15, 1)
    }

    pub fn require_supported(&self) -> Result<()> {
        if self.is_required_version() {
            Ok(())
        } else {
            Err(BackendError::UnsupportedTypstVersion {
                found: self.version.to_string(),
                required: REQUIRED_TYPST_VERSION,
            })
        }
    }

    pub fn compile_svg(
        &self,
        project: &Project,
        main: &str,
    ) -> Result<CompileOutput<Vec<SvgPage>>> {
        self.compile_svg_with_options(project, main, &CompileOptions::default())
    }

    pub fn compile_svg_with_options(
        &self,
        project: &Project,
        main: &str,
        options: &CompileOptions,
    ) -> Result<CompileOutput<Vec<SvgPage>>> {
        self.require_supported()?;
        let (main, _) = safe_existing_path(project.root(), main)?;
        let output_dir =
            tempdir().map_err(|error| BackendError::io("temporary render directory", error))?;
        let output_pattern = output_dir.path().join("page-{0p}.svg");
        let mut command = Command::new(&self.executable);
        command
            .arg("compile")
            .arg("--format")
            .arg("svg")
            .arg("--diagnostic-format")
            .arg("short")
            .arg("--root")
            .arg(project.root());
        apply_options(&mut command, options);
        let (entrypoint, _wrapper) = compile_entry(project, &main, options)?;
        command.arg(entrypoint).arg(&output_pattern);
        let started = Instant::now();
        let process = run_command(&mut command, PROCESS_TIMEOUT)?;
        let elapsed = started.elapsed();
        let diagnostics = parse_diagnostics(&process.stderr, project.root());
        let artifact = if process.status == Some(0) {
            let mut paths = fs::read_dir(output_dir.path())
                .map_err(|error| BackendError::io(output_dir.path(), error))?
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension() == Some(OsStr::new("svg")))
                .collect::<Vec<_>>();
            paths.sort_by_key(|path| page_number(path));
            let pages = paths
                .into_iter()
                .enumerate()
                .map(|(index, path)| {
                    let svg = fs::read_to_string(&path)
                        .map_err(|error| BackendError::io(&path, error))?;
                    let (width_points, height_points) = svg_dimensions(&svg);
                    Ok(SvgPage {
                        page: page_number(&path).unwrap_or(index + 1),
                        svg,
                        width_points,
                        height_points,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Some(pages)
        } else {
            None
        };
        Ok(CompileOutput {
            artifact,
            diagnostics,
            stdout: process.stdout,
            stderr: process.stderr,
            elapsed,
        })
    }

    pub fn compile_pdf(&self, project: &Project, main: &str) -> Result<CompileOutput<Vec<u8>>> {
        self.compile_pdf_with_options(project, main, &CompileOptions::default())
    }

    pub fn compile_pdf_with_options(
        &self,
        project: &Project,
        main: &str,
        options: &CompileOptions,
    ) -> Result<CompileOutput<Vec<u8>>> {
        self.require_supported()?;
        let (main, _) = safe_existing_path(project.root(), main)?;
        let output_dir =
            tempdir().map_err(|error| BackendError::io("temporary render directory", error))?;
        let output = output_dir.path().join("document.pdf");
        let mut command = Command::new(&self.executable);
        command
            .arg("compile")
            .arg("--format")
            .arg("pdf")
            .arg("--diagnostic-format")
            .arg("short")
            .arg("--root")
            .arg(project.root());
        apply_options(&mut command, options);
        let (entrypoint, _wrapper) = compile_entry(project, &main, options)?;
        command.arg(entrypoint).arg(&output);
        let started = Instant::now();
        let process = run_command(&mut command, PROCESS_TIMEOUT)?;
        let elapsed = started.elapsed();
        let diagnostics = parse_diagnostics(&process.stderr, project.root());
        let artifact = if process.status == Some(0) {
            Some(fs::read(&output).map_err(|error| BackendError::io(&output, error))?)
        } else {
            None
        };
        Ok(CompileOutput {
            artifact,
            diagnostics,
            stdout: process.stdout,
            stderr: process.stderr,
            elapsed,
        })
    }

    pub fn query_notes(
        &self,
        project: &Project,
        main: &str,
        options: &CompileOptions,
    ) -> Result<Vec<InlineNote>> {
        self.require_supported()?;
        let (main, _) = safe_existing_path(project.root(), main)?;
        let (entrypoint, _wrapper) = compile_entry(project, &main, options)?;
        let mut command = Command::new(&self.executable);
        command.arg("query").arg("--root").arg(project.root());
        apply_options(&mut command, options);
        command
            .arg(entrypoint)
            .arg("<typsmthng-note>")
            .arg("--field")
            .arg("value")
            .arg("--format")
            .arg("json");
        let output = run_command(&mut command, PROCESS_TIMEOUT)?;
        if output.status != Some(0) {
            return Ok(Vec::new());
        }
        let values = serde_json::from_str::<Vec<Value>>(&output.stdout)
            .map_err(|error| BackendError::Network(format!("invalid Typst notes JSON: {error}")))?;
        Ok(values.into_iter().filter_map(inline_note).collect())
    }

    /// Materialize a Typst Universe or local template through the official CLI.
    /// The CLI owns package resolution, caching, and transitive package downloads.
    pub fn init_template(
        &self,
        template_spec: &str,
        destination: impl AsRef<Path>,
        options: &CompileOptions,
    ) -> Result<()> {
        self.require_supported()?;
        let valid_spec =
            Regex::new(r"^@(preview|local)/[a-z0-9][a-z0-9-]*(?::[0-9]+\.[0-9]+\.[0-9]+)?$")
                .unwrap();
        if !valid_spec.is_match(template_spec) {
            return Err(BackendError::Process(format!(
                "invalid Typst template specification: {template_spec}"
            )));
        }
        let destination = destination.as_ref();
        if destination.exists() {
            return Err(BackendError::AlreadyExists(destination.to_path_buf()));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| BackendError::UnsafePath(destination.display().to_string()))?;
        fs::create_dir_all(parent).map_err(|error| BackendError::io(parent, error))?;
        let staging_parent = tempdir().map_err(|error| BackendError::io(parent, error))?;
        let staging = staging_parent.path().join("project");
        let mut command = Command::new(&self.executable);
        command.arg("init").arg(template_spec).arg(&staging);
        if let Some(path) = &options.package_path {
            command.arg("--package-path").arg(path);
        }
        if let Some(path) = &options.package_cache_path {
            command.arg("--package-cache-path").arg(path);
        }
        let output = run_command(&mut command, PROCESS_TIMEOUT)?;
        if output.status != Some(0) {
            return Err(BackendError::Process(output.stderr));
        }
        fs::rename(&staging, destination).map_err(|error| BackendError::io(destination, error))?;
        Ok(())
    }
}

fn inline_note(value: Value) -> Option<InlineNote> {
    let object = value.as_object()?;
    let page = object
        .get("page")
        .and_then(|page| page.as_u64().or_else(|| page.as_str()?.parse::<u64>().ok()))?
        .try_into()
        .ok()?;
    let text = ["text", "note", "body"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))?
        .to_string();
    (page > 0).then_some(InlineNote { page, text })
}

fn apply_options(command: &mut Command, options: &CompileOptions) {
    for path in &options.font_paths {
        command.arg("--font-path").arg(path);
    }
    if let Some(path) = &options.package_path {
        command.arg("--package-path").arg(path);
    }
    if let Some(path) = &options.package_cache_path {
        command.arg("--package-cache-path").arg(path);
    }
    if let Some(timestamp) = options.creation_timestamp {
        command
            .arg("--creation-timestamp")
            .arg(timestamp.to_string());
    }
    if options.ignore_system_fonts {
        command.arg("--ignore-system-fonts");
    }
}

fn compile_entry(
    project: &Project,
    main: &str,
    options: &CompileOptions,
) -> Result<(PathBuf, Option<tempfile::NamedTempFile>)> {
    let main_path = project.root().join(relative_pathbuf(main));
    let Some(preamble) = options.page_preamble.as_deref() else {
        return Ok((main_path, None));
    };
    if preamble.trim().is_empty() {
        return Ok((main_path, None));
    }
    let mut wrapper = tempfile::Builder::new()
        .prefix(".typsmthng-preview-")
        .suffix(".typ")
        .tempfile_in(project.root())
        .map_err(|error| BackendError::io(project.root(), error))?;
    let relative = relative_pathbuf(main)
        .to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"");
    writeln!(wrapper, "{preamble}\n#include \"{relative}\"")
        .map_err(|error| BackendError::io(wrapper.path(), error))?;
    let path = wrapper.path().to_path_buf();
    Ok((path, Some(wrapper)))
}

#[derive(Debug)]
struct ProcessOutput {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_command(command: &mut Command, timeout: Duration) -> Result<ProcessOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| BackendError::Process(error.to_string()))?;
    let stdout = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = String::new();
            pipe.read_to_string(&mut output).map(|_| output)
        })
    });
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = String::new();
            pipe.read_to_string(&mut output).map(|_| output)
        })
    });
    let status = child
        .wait_timeout(timeout)
        .map_err(|error| BackendError::Process(error.to_string()))?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(BackendError::TypstTimeout);
    }
    let join_output = |reader: Option<std::thread::JoinHandle<std::io::Result<String>>>| {
        reader
            .map(|reader| {
                reader
                    .join()
                    .map_err(|_| BackendError::Process("compiler output reader panicked".into()))?
                    .map_err(|error| BackendError::Process(error.to_string()))
            })
            .unwrap_or_else(|| Ok(String::new()))
    };
    let stdout = join_output(stdout)?;
    let stderr = join_output(stderr)?;
    Ok(ProcessOutput {
        status: status.and_then(|status| status.code()),
        stdout,
        stderr,
    })
}

fn typst_candidates() -> Vec<PathBuf> {
    let executable_name = if cfg!(windows) { "typst.exe" } else { "typst" };
    let mut candidates = Vec::new();
    if let Some(explicit) = std::env::var_os("TYPSMTHNG_TYPST") {
        candidates.push(PathBuf::from(explicit));
    }
    if let Ok(current) = std::env::current_exe() {
        if let Some(directory) = current.parent() {
            candidates.push(directory.join(executable_name));
            candidates.push(directory.join("../Resources").join(executable_name));
            candidates.push(directory.join("../lib/typsmthng").join(executable_name));
        }
    }
    if let Some(path) = find_on_path(executable_name) {
        candidates.push(path);
    }
    candidates.dedup();
    candidates
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
}

fn parse_version(output: &str) -> Option<Version> {
    let version = Regex::new(r"(?m)^typst\s+(\d+\.\d+\.\d+)").ok()?;
    Version::parse(version.captures(output)?.get(1)?.as_str()).ok()
}

pub fn parse_diagnostics(stderr: &str, root: &Path) -> Vec<Diagnostic> {
    let positioned = Regex::new(r"^(.*):(\d+):(\d+):\s*(error|warning|hint):\s*(.+)$").unwrap();
    let unpositioned = Regex::new(r"^(error|warning|hint):\s*(.+)$").unwrap();
    let mut diagnostics = Vec::new();
    for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
        if let Some(captures) = positioned.captures(line) {
            let raw_path = PathBuf::from(captures.get(1).unwrap().as_str());
            let path = normalize_diagnostic_path(&raw_path, root);
            diagnostics.push(Diagnostic {
                severity: severity(captures.get(4).unwrap().as_str()),
                path: Some(path),
                line: captures
                    .get(2)
                    .and_then(|value| value.as_str().parse().ok()),
                column: captures
                    .get(3)
                    .and_then(|value| value.as_str().parse().ok()),
                message: captures.get(5).unwrap().as_str().trim().into(),
            });
        } else if let Some(captures) = unpositioned.captures(line) {
            diagnostics.push(Diagnostic {
                severity: severity(captures.get(1).unwrap().as_str()),
                path: None,
                line: None,
                column: None,
                message: captures.get(2).unwrap().as_str().trim().into(),
            });
        } else if let Some(last) = diagnostics.last_mut() {
            last.message.push('\n');
            last.message.push_str(line.trim());
        }
    }
    diagnostics
}

fn normalize_diagnostic_path(path: &Path, root: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    absolute
        .canonicalize()
        .ok()
        .and_then(|path| path.strip_prefix(root).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
}

fn severity(value: &str) -> DiagnosticSeverity {
    match value {
        "warning" => DiagnosticSeverity::Warning,
        "hint" => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Error,
    }
}

fn page_number(path: &Path) -> Option<usize> {
    let name = path.file_stem()?.to_string_lossy();
    name.rsplit('-').next()?.parse().ok()
}

fn svg_dimensions(svg: &str) -> (Option<f64>, Option<f64>) {
    let dimension = |name: &str| {
        Regex::new(&format!(r#"\b{name}="([0-9.]+)(?:pt)?""#))
            .ok()?
            .captures(svg)?
            .get(1)?
            .as_str()
            .parse()
            .ok()
    };
    (dimension("width"), dimension("height"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn parses_versions_and_windows_diagnostic_paths() {
        assert_eq!(
            parse_version("typst 0.15.1 (abc)"),
            Some(Version::new(0, 15, 1))
        );
        let diagnostics = parse_diagnostics(
            "C:\\work\\main.typ:12:7: error: unknown variable: thing\nwarning: follow-up",
            Path::new("/no/root"),
        );
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].line, Some(12));
        assert_eq!(diagnostics[0].message, "unknown variable: thing");
    }

    #[test]
    fn compiles_multiple_svg_pages_and_pdf_with_cli_when_available() {
        let Ok(tool) = TypstTool::detect() else {
            return;
        };
        let directory = tempdir().unwrap();
        let root = directory.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("main.typ"), "First#pagebreak()Second").unwrap();
        let project = Project::open(root).unwrap();
        let svg = tool.compile_svg(&project, "main.typ").unwrap();
        assert!(svg.success(), "{}", svg.stderr);
        assert_eq!(svg.artifact.unwrap().len(), 2);
        let pdf = tool.compile_pdf(&project, "main.typ").unwrap();
        assert!(pdf.artifact.unwrap().starts_with(b"%PDF"));
    }

    #[test]
    fn returns_structured_compiler_diagnostics() {
        let Ok(tool) = TypstTool::detect() else {
            return;
        };
        let directory = tempdir().unwrap();
        let root = directory.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("main.typ"), "#missing-symbol").unwrap();
        let output = tool
            .compile_pdf(&Project::open(root).unwrap(), "main.typ")
            .unwrap();
        assert!(!output.success());
        assert_eq!(output.diagnostics[0].severity, DiagnosticSeverity::Error);
        assert_eq!(output.diagnostics[0].line, Some(1));
    }

    #[test]
    fn queries_inline_presentation_notes_when_cli_is_available() {
        let Ok(tool) = TypstTool::detect() else {
            return;
        };
        let directory = tempdir().unwrap();
        let root = directory.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("main.typ"),
            "#let note(text) = context [#metadata((page: here().page(), text: text)) <typsmthng-note>]\nSlide #note(\"Say hello\")",
        )
        .unwrap();
        let notes = tool
            .query_notes(
                &Project::open(root).unwrap(),
                "main.typ",
                &CompileOptions::default(),
            )
            .unwrap();
        assert_eq!(
            notes,
            vec![InlineNote {
                page: 1,
                text: "Say hello".into()
            }]
        );
    }
}
