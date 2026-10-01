use std::ffi::OsStr;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    LazyLock, Mutex,
};
use std::time::{Duration, Instant, SystemTime};

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
pub use super::compile_options::CompileOptions;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(60);

// Probe once per executable revision, while still respecting environment/PATH
// changes and a compiler replaced in place during the app session.
struct CachedTypstTool {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    tool: TypstTool,
}

static DETECTED_TOOL: LazyLock<Mutex<Option<CachedTypstTool>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone)]
pub struct TypstTool {
    executable: PathBuf,
    version: Version,
}

/// Profiles accepted by the pinned Typst 0.15.1 CLI. Typst checks document
/// requirements during compilation; selecting a profile is not certification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfStandard {
    pub label: &'static str,
    cli: &'static str,
}

impl PdfStandard {
    pub const ALL: [Self; 17] = [
        Self {
            label: "PDF 1.4",
            cli: "1.4",
        },
        Self {
            label: "PDF 1.5",
            cli: "1.5",
        },
        Self {
            label: "PDF 1.6",
            cli: "1.6",
        },
        Self {
            label: "PDF 1.7",
            cli: "1.7",
        },
        Self {
            label: "PDF 2.0",
            cli: "2.0",
        },
        Self {
            label: "PDF/A-1b",
            cli: "a-1b",
        },
        Self {
            label: "PDF/A-1a",
            cli: "a-1a",
        },
        Self {
            label: "PDF/A-2b",
            cli: "a-2b",
        },
        Self {
            label: "PDF/A-2u",
            cli: "a-2u",
        },
        Self {
            label: "PDF/A-2a",
            cli: "a-2a",
        },
        Self {
            label: "PDF/A-3b",
            cli: "a-3b",
        },
        Self {
            label: "PDF/A-3u",
            cli: "a-3u",
        },
        Self {
            label: "PDF/A-3a",
            cli: "a-3a",
        },
        Self {
            label: "PDF/A-4",
            cli: "a-4",
        },
        Self {
            label: "PDF/A-4f",
            cli: "a-4f",
        },
        Self {
            label: "PDF/A-4e",
            cli: "a-4e",
        },
        Self {
            label: "PDF/UA-1",
            cli: "ua-1",
        },
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Pdf(Option<PdfStandard>),
    Svg,
    Png,
    Html,
}

impl ExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf(_) => "PDF",
            Self::Svg => "SVG pages",
            Self::Png => "PNG pages",
            Self::Html => "HTML",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf(_) => "pdf",
            Self::Svg | Self::Png => "zip",
            Self::Html => "html",
        }
    }

    fn cli_format(self) -> &'static str {
        match self {
            Self::Pdf(_) => "pdf",
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Html => "html",
        }
    }

    fn is_pages(self) -> bool {
        matches!(self, Self::Svg | Self::Png)
    }
}

#[derive(Debug)]
pub struct ExportArtifact {
    pub format: ExportFormat,
    pub bytes: Vec<u8>,
}

impl ExportArtifact {
    /// Stage in the destination directory so failures never leave a partial
    /// file. Page bundles refuse replacement, including existing symlinks.
    pub fn save(&self, destination: &Path) -> Result<()> {
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| BackendError::io(parent, error))?;
        temporary
            .write_all(&self.bytes)
            .map_err(|error| BackendError::io(temporary.path(), error))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| BackendError::io(temporary.path(), error))?;
        if self.format.is_pages() {
            temporary.persist_noclobber(destination).map_err(|error| {
                if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                    BackendError::AlreadyExists(destination.to_path_buf())
                } else {
                    BackendError::io(destination, error.error)
                }
            })?;
        } else {
            temporary
                .persist(destination)
                .map_err(|error| BackendError::io(destination, error.error))?;
        }
        Ok(())
    }
}

struct CompiledFile {
    name: String,
    bytes: Vec<u8>,
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
            let metadata = fs::metadata(&candidate).ok();
            let stamp = metadata
                .as_ref()
                .map(|metadata| (metadata.len(), metadata.modified().ok()));
            if let Some((size, modified)) = stamp {
                let cache = DETECTED_TOOL
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(cached) = cache.as_ref() {
                    if cached.path == candidate
                        && cached.size == size
                        && cached.modified == modified
                    {
                        return Ok(cached.tool.clone());
                    }
                }
            }
            match Self::probe(candidate.clone()) {
                Ok(tool) if tool.is_required_version() => {
                    if let Some((size, modified)) = stamp {
                        *DETECTED_TOOL
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = Some(CachedTypstTool {
                            path: candidate,
                            size,
                            modified,
                            tool: tool.clone(),
                        });
                    }
                    return Ok(tool);
                }
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
        let output = self.compile_files(project, main, ExportFormat::Svg, options)?;
        let pages = output
            .artifact
            .map(|files| {
                files
                    .into_iter()
                    .enumerate()
                    .map(|(index, file)| {
                        let svg = String::from_utf8(file.bytes)
                            .map_err(|error| BackendError::Process(error.to_string()))?;
                        let (width_points, height_points) = svg_dimensions(&svg);
                        Ok(SvgPage {
                            page: page_number(Path::new(&file.name)).unwrap_or(index + 1),
                            svg,
                            width_points,
                            height_points,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        Ok(CompileOutput {
            artifact: pages,
            diagnostics: output.diagnostics,
            stdout: output.stdout,
            stderr: output.stderr,
            elapsed: output.elapsed,
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
        let output = self.compile_export(project, main, ExportFormat::Pdf(None), options)?;
        Ok(CompileOutput {
            artifact: output.artifact.map(|artifact| artifact.bytes),
            diagnostics: output.diagnostics,
            stdout: output.stdout,
            stderr: output.stderr,
            elapsed: output.elapsed,
        })
    }

    pub fn compile_export(
        &self,
        project: &Project,
        main: &str,
        format: ExportFormat,
        options: &CompileOptions,
    ) -> Result<CompileOutput<ExportArtifact>> {
        let output = self.compile_files(project, main, format, options)?;
        let artifact = output
            .artifact
            .map(|files| -> Result<ExportArtifact> {
                let bytes = if format.is_pages() {
                    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
                    let settings = zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated)
                        .unix_permissions(0o644);
                    for file in files {
                        archive.start_file(file.name, settings)?;
                        archive
                            .write_all(&file.bytes)
                            .map_err(|error| BackendError::io("page archive", error))?;
                    }
                    archive.finish()?.into_inner()
                } else {
                    files
                        .into_iter()
                        .next()
                        .ok_or_else(|| BackendError::Process("Typst produced no output".into()))?
                        .bytes
                };
                Ok(ExportArtifact { format, bytes })
            })
            .transpose()?;
        Ok(CompileOutput {
            artifact,
            diagnostics: output.diagnostics,
            stdout: output.stdout,
            stderr: output.stderr,
            elapsed: output.elapsed,
        })
    }

    fn compile_files(
        &self,
        project: &Project,
        main: &str,
        format: ExportFormat,
        options: &CompileOptions,
    ) -> Result<CompileOutput<Vec<CompiledFile>>> {
        self.require_supported()?;
        let options = options.resolved()?;
        let (main, _) = safe_existing_path(project.root(), main)?;
        let output_dir =
            tempdir().map_err(|error| BackendError::io("temporary render directory", error))?;
        let extension = format.cli_format();
        let output = output_dir.path().join(if format.is_pages() {
            format!("page-{{0p}}.{extension}")
        } else {
            format!("document.{extension}")
        });
        let mut command = Command::new(&self.executable);
        command
            .arg("compile")
            .arg("--format")
            .arg(extension)
            .arg("--diagnostic-format")
            .arg("short")
            .arg("--root")
            .arg(project.root());
        if let ExportFormat::Pdf(Some(standard)) = format {
            command.arg("--pdf-standard").arg(standard.cli);
        }
        if format == ExportFormat::Html {
            command.arg("--features").arg("html");
        }
        options.apply_to_command(&mut command);
        let (entrypoint, _wrapper) = compile_entry(project, &main, &options)?;
        command.arg(entrypoint).arg(&output);
        let started = Instant::now();
        let process = run_command_cancellable(
            &mut command,
            PROCESS_TIMEOUT,
            options.cancellation.as_deref(),
        )?;
        let elapsed = started.elapsed();
        let diagnostics = parse_diagnostics(&process.stderr, project.root());
        let artifact = if process.status == Some(0) {
            let mut paths = fs::read_dir(output_dir.path())
                .map_err(|error| BackendError::io(output_dir.path(), error))?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|error| BackendError::io(output_dir.path(), error))?;
            paths.retain(|path| path.extension() == Some(OsStr::new(extension)));
            paths.sort_by_key(|path| page_number(path));
            Some(
                paths
                    .into_iter()
                    .map(|path| {
                        Ok(CompiledFile {
                            name: path.file_name().unwrap().to_string_lossy().into_owned(),
                            bytes: fs::read(&path)
                                .map_err(|error| BackendError::io(&path, error))?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
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
        let options = options.resolved()?;
        let (main, _) = safe_existing_path(project.root(), main)?;
        let (entrypoint, _wrapper) = compile_entry(project, &main, &options)?;
        let mut command = Command::new(&self.executable);
        command.arg("query").arg("--root").arg(project.root());
        options.apply_to_command(&mut command);
        command
            .arg(entrypoint)
            .arg("<typsmthng-note>")
            .arg("--field")
            .arg("value")
            .arg("--format")
            .arg("json");
        let output = run_command_cancellable(
            &mut command,
            PROCESS_TIMEOUT,
            options.cancellation.as_deref(),
        )?;
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

pub(super) fn compile_entry(
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
    run_command_cancellable(command, timeout, None)
}

fn run_command_cancellable(
    command: &mut Command,
    timeout: Duration,
    cancellation: Option<&AtomicBool>,
) -> Result<ProcessOutput> {
    if cancellation.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
        return Err(BackendError::Process("Compilation superseded".into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
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
    let started = Instant::now();
    let mut failure = None;
    let status = loop {
        if cancellation.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            failure = Some(BackendError::Process("Compilation superseded".into()));
            break None;
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            failure = Some(BackendError::TypstTimeout);
            break None;
        }
        let interval = if cancellation.is_some() {
            remaining.min(Duration::from_millis(10))
        } else {
            remaining
        };
        match child.wait_timeout(interval) {
            Ok(Some(status)) => break Some(status),
            Ok(None) => continue,
            Err(error) => {
                failure = Some(BackendError::Process(error.to_string()));
                break None;
            }
        }
    };
    if status.is_none() {
        terminate_process_tree(&mut child);
        // A custom wrapper may have inherited output pipes. Do not wait on
        // readers after failure if the operating system could not kill it.
        return Err(failure.unwrap_or(BackendError::TypstTimeout));
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
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(ProcessOutput {
        status: status.and_then(|status| status.code()),
        stdout,
        stderr,
    })
}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // The child was explicitly placed in its own process group above.
        // Kill the group so a wrapper's compiler cannot retain output pipes.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(mut killer) = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &child.id().to_string()])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if !matches!(killer.wait_timeout(Duration::from_secs(2)), Ok(Some(_))) {
                let _ = killer.kill();
                let _ = killer.wait();
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
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
    static POSITIONED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(.*):(\d+):(\d+):\s*(error|warning|hint):\s*(.+)$").unwrap()
    });
    static UNPOSITIONED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(error|warning|hint):\s*(.+)$").unwrap());
    let mut diagnostics = Vec::new();
    for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
        if let Some(captures) = POSITIONED.captures(line) {
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
        } else if let Some(captures) = UNPOSITIONED.captures(line) {
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
    static WIDTH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\bwidth="([0-9.]+)(?:pt)?""#).unwrap());
    static HEIGHT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\bheight="([0-9.]+)(?:pt)?""#).unwrap());
    // Only inspect the root element. Descendant image/rectangle dimensions are
    // unrelated, and a page can contain megabytes of paths after this header.
    let Some(start) = svg.find("<svg") else {
        return (None, None);
    };
    let Some(end) = svg[start..].find('>') else {
        return (None, None);
    };
    let header = &svg[start..start + end];
    let dimension = |pattern: &Regex| pattern.captures(header)?.get(1)?.as_str().parse().ok();
    (dimension(&WIDTH), dimension(&HEIGHT))
}

#[cfg(test)]
mod tests {
    fn export_fixture(source: &str) -> (tempfile::TempDir, Project) {
        let directory = tempdir().unwrap();
        let root = directory.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("main.typ"), source).unwrap();
        (directory, Project::open(root).unwrap())
    }

    #[test]
    fn exports_all_pdf_profiles_with_pinned_cli() {
        let tool = TypstTool::detect().unwrap();
        let (_directory, project) = export_fixture(
            "#set document(title: \"Export test\")\n#set text(lang: \"en\")\n= Document\nHello",
        );
        for standard in PdfStandard::ALL {
            let output = tool
                .compile_export(
                    &project,
                    "main.typ",
                    ExportFormat::Pdf(Some(standard)),
                    &CompileOptions::default(),
                )
                .unwrap();
            assert!(output.success(), "{}: {}", standard.label, output.stderr);
            let artifact = output.artifact.unwrap();
            assert!(artifact.bytes.starts_with(b"%PDF"));
            if standard.cli.chars().next().unwrap().is_ascii_digit() {
                assert!(artifact
                    .bytes
                    .starts_with(format!("%PDF-{}", standard.cli).as_bytes()));
            }
        }
    }

    #[test]
    fn exports_all_svg_and_png_pages_as_ordered_zip() {
        let tool = TypstTool::detect().unwrap();
        let (_directory, project) = export_fixture(&["Page"; 12].join("#pagebreak()"));
        for format in [ExportFormat::Svg, ExportFormat::Png] {
            let output = tool
                .compile_export(&project, "main.typ", format, &CompileOptions::default())
                .unwrap();
            assert!(output.success(), "{}", output.stderr);
            let mut archive =
                zip::ZipArchive::new(Cursor::new(output.artifact.unwrap().bytes)).unwrap();
            assert_eq!(archive.len(), 12);
            for index in 0..12 {
                let mut page = archive.by_index(index).unwrap();
                assert_eq!(
                    page.name(),
                    format!("page-{:02}.{}", index + 1, format.cli_format())
                );
                let mut bytes = Vec::new();
                page.read_to_end(&mut bytes).unwrap();
                match format {
                    ExportFormat::Svg => {
                        assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"))
                    }
                    ExportFormat::Png => assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n")),
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn html_export_enables_experimental_feature_and_returns_structure() {
        let tool = TypstTool::detect().unwrap();
        let (_directory, project) = export_fixture("= Web heading\nHello *web*.");
        let output = tool
            .compile_export(
                &project,
                "main.typ",
                ExportFormat::Html,
                &CompileOptions::default(),
            )
            .unwrap();
        assert!(output.success(), "{}", output.stderr);
        let html = String::from_utf8(output.artifact.unwrap().bytes).unwrap();
        assert!(
            html.contains("<h2") && html.contains("Web heading"),
            "{html}"
        );
        assert!(html.contains("<strong>web</strong>"));
    }

    #[test]
    fn export_profile_requirements_fail_with_diagnostics_and_no_artifact() {
        let tool = TypstTool::detect().unwrap();
        let (_directory, project) = export_fixture("Hello");
        let standard = PdfStandard::ALL
            .iter()
            .find(|standard| standard.cli == "ua-1")
            .copied()
            .unwrap();
        let output = tool
            .compile_export(
                &project,
                "main.typ",
                ExportFormat::Pdf(Some(standard)),
                &CompileOptions::default(),
            )
            .unwrap();
        assert!(!output.success());
        assert!(output.artifact.is_none());
        assert!(output.stderr.contains("title"), "{}", output.stderr);
        assert!(output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error));
    }

    #[test]
    fn export_reuses_packages_page_settings_signed_timestamp_and_cancellation() {
        let tool = TypstTool::detect().unwrap();
        let (directory, project) =
            export_fixture("#import \"@local/export-test:0.1.0\": greeting\n#greeting");
        let packages = directory.path().join("packages");
        let package = packages.join("local/export-test/0.1.0");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("typst.toml"),
            "[package]\nname = \"export-test\"\nversion = \"0.1.0\"\nentrypoint = \"lib.typ\"",
        )
        .unwrap();
        fs::write(
            package.join("lib.typ"),
            "#let greeting = [Configured export]",
        )
        .unwrap();
        let options = CompileOptions {
            package_path: Some(packages),
            package_cache_path: Some(directory.path().join("cache")),
            creation_timestamp: Some(-1),
            page_preamble: Some("#set page(width: 200pt, height: 300pt)".into()),
            ignore_system_fonts: true,
            inherit_environment: false,
            ..Default::default()
        };
        let output = tool
            .compile_export(&project, "main.typ", ExportFormat::Pdf(None), &options)
            .unwrap();
        assert!(output.success(), "{}", output.stderr);
        let bytes = output.artifact.unwrap().bytes;
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("D:19691231235959"), "signed timestamp missing");
        let svg = tool
            .compile_svg_with_options(&project, "main.typ", &options)
            .unwrap();
        assert!(svg.success(), "{}", svg.stderr);
        let page = &svg.artifact.unwrap()[0];
        assert_eq!(
            (page.width_points, page.height_points),
            (Some(200.0), Some(300.0))
        );
        assert_eq!(
            fs::read_dir(project.root()).unwrap().count(),
            1,
            "entry wrapper must be removed"
        );
        let invalid = CompileOptions {
            creation_timestamp: Some(i64::MAX),
            ..options.clone()
        };
        assert!(matches!(
            tool.compile_export(&project, "main.typ", ExportFormat::Png, &invalid),
            Err(BackendError::InvalidTypstConfiguration { .. })
        ));
        let cancelled = CompileOptions {
            cancellation: Some(std::sync::Arc::new(AtomicBool::new(true))),
            ..options
        };
        assert!(tool
            .compile_export(&project, "main.typ", ExportFormat::Html, &cancelled)
            .is_err());
    }

    #[test]
    fn page_bundle_save_refuses_existing_files_and_single_exports_are_atomic() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("pages.zip");
        let artifact = ExportArtifact {
            format: ExportFormat::Svg,
            bytes: b"complete bundle".to_vec(),
        };
        artifact.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), artifact.bytes);
        let replacement = ExportArtifact {
            format: ExportFormat::Png,
            bytes: b"replacement".to_vec(),
        };
        assert!(matches!(
            replacement.save(&path),
            Err(BackendError::AlreadyExists(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), artifact.bytes);
        let missing_parent = directory.path().join("missing/document.pdf");
        let pdf = ExportArtifact {
            format: ExportFormat::Pdf(None),
            bytes: b"complete PDF".to_vec(),
        };
        assert!(pdf.save(&missing_parent).is_err());
        assert!(!missing_parent.exists());
        pdf.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), pdf.bytes);
        assert_eq!(
            fs::read_dir(directory.path()).unwrap().count(),
            1,
            "staging files must be removed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn page_bundle_save_refuses_symlinks_including_dangling_links() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("target");
        let link = directory.path().join("pages.zip");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let artifact = ExportArtifact {
            format: ExportFormat::Svg,
            bytes: vec![1, 2, 3],
        };
        assert!(matches!(
            artifact.save(&link),
            Err(BackendError::AlreadyExists(_))
        ));
        assert!(!target.exists());
        fs::write(&target, b"keep").unwrap();
        assert!(artifact.save(&link).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn superseded_process_is_killed_without_waiting_for_timeout() {
        let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = cancellation.clone();
        let signal = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = super::run_command_cancellable(
            std::process::Command::new("sleep").arg("10"),
            std::time::Duration::from_secs(15),
            Some(&cancellation),
        );
        signal.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("superseded"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_wrapper_does_not_wait_for_descendant_output_pipes() {
        let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = cancellation.clone();
        let signal = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = super::run_command_cancellable(
            std::process::Command::new("sh").args(["-c", "sleep 10 & wait"]),
            std::time::Duration::from_secs(15),
            Some(&cancellation),
        );
        signal.join().unwrap();
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn cancelled_command_is_not_spawned() {
        let cancellation = std::sync::atomic::AtomicBool::new(true);
        let result = super::run_command_cancellable(
            &mut std::process::Command::new("nonexistent-compiler-test"),
            std::time::Duration::from_secs(1),
            Some(&cancellation),
        );
        assert!(result.unwrap_err().to_string().contains("superseded"));
    }

    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    #[ignore = "manual performance measurement requiring Typst 0.15.1"]
    fn benchmark_typst_preview() {
        let tool = TypstTool::detect().expect("Typst 0.15.1 required for preview benchmark");
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "benchmark").unwrap();
        fs::write(
            project.root().join("main.typ"),
            ["= A page\nHello from the preview."; 10].join("\n#pagebreak()\n"),
        )
        .unwrap();
        let mut timings = Vec::new();
        for _ in 0..10 {
            let started = Instant::now();
            let output = tool.compile_svg(&project, "main.typ").unwrap();
            assert!(output.success(), "{}", output.stderr);
            assert_eq!(output.artifact.unwrap().len(), 10);
            timings.push(started.elapsed());
        }
        timings.sort();
        eprintln!(
            "10-page SVG compile, 10 runs: median {:?}, max {:?}",
            timings[5], timings[9]
        );
    }

    #[test]
    fn svg_dimensions_belong_to_the_page_not_its_children() {
        assert_eq!(
            svg_dimensions(
                r#"<svg width="595.28pt" height="841.89pt"><rect width="10" height="20"/></svg>"#
            ),
            (Some(595.28), Some(841.89))
        );
        assert_eq!(
            svg_dimensions(r#"<svg viewBox="0 0 100 200"><rect width="10" height="20"/></svg>"#),
            (None, None)
        );
        assert_eq!(svg_dimensions("not SVG"), (None, None));
    }

    #[test]
    #[ignore = "manual performance measurement"]
    fn benchmark_svg_page_headers() {
        let svg = format!(
            "<svg width=\"595pt\" height=\"842pt\">{}</svg>",
            "<path d=\"M 0 0\"/>".repeat(100_000)
        );
        svg_dimensions(&svg);
        let started = Instant::now();
        for _ in 0..1000 {
            std::hint::black_box(svg_dimensions(std::hint::black_box(&svg)));
        }
        eprintln!("1000 large SVG page headers: {:?}", started.elapsed());
        let started = Instant::now();
        for _ in 0..1000 {
            for name in ["width", "height"] {
                let pattern = Regex::new(&format!(r#"\b{name}="([0-9.]+)(?:pt)?""#)).unwrap();
                std::hint::black_box(pattern.captures(std::hint::black_box(&svg)));
            }
        }
        eprintln!(
            "1000 previous per-page regex builds: {:?}",
            started.elapsed()
        );
    }

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
    fn compiles_multiple_svg_pages_and_pdf_with_cli() {
        let tool = TypstTool::detect().expect("install Typst 0.15.1 or set TYPSMTHNG_TYPST");
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
        let tool = TypstTool::detect().expect("install Typst 0.15.1 or set TYPSMTHNG_TYPST");
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
    fn queries_inline_presentation_notes_with_cli() {
        let tool = TypstTool::detect().expect("install Typst 0.15.1 or set TYPSMTHNG_TYPST");
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
