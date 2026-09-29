//! Persistent SVG preview. The FileStore reset/reuse approach follows the local
//! typrst compatible engine; PDF export and CLI subprocesses stay off this path.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::introspection::PagedPosition;
use typst::layout::{Abs, Point};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_ide::{IdeWorld, Jump};
use typst_kit::datetime::Time;
use typst_kit::diagnostics::{self, DiagnosticFormat, DiagnosticWorld};
use typst_kit::downloader::Downloader;
use typst_kit::files::{FileStore, FsRoot, SystemFiles};
use typst_kit::fonts::{self, FontStore};
use typst_kit::packages::{FsPackages, SystemPackages, UniversePackages};
use typst_layout::PagedDocument;

use super::typst::{compile_entry, parse_diagnostics, CompileOptions, CompileOutput, SvgPage};
use super::{BackendError, Project, Result};

pub struct Preview {
    pub pages: Vec<SvgPage>,
    pub source_map: Arc<SourceMap>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

/// Immutable document and source trees from exactly the same compilation.
/// Widget reuse must update this even if the SVG bytes have not changed.
pub struct SourceMap {
    document: PagedDocument,
    world: Snapshot,
}

impl SourceMap {
    pub fn dimensions(&self, page: usize) -> Option<(f64, f64)> {
        let frame = &self.document.pages().get(page)?.frame;
        Some((frame.width().to_pt(), frame.height().to_pt()))
    }

    pub fn jump(&self, page: usize, x: f64, y: f64) -> Option<SourceLocation> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let position = PagedPosition {
            page: page.checked_add(1)?.try_into().ok()?,
            point: Point::new(Abs::pt(x), Abs::pt(y)),
        };
        let Jump::File(id, offset) =
            typst_ide::jump_from_click(&self.world, &self.document, &position)?
        else {
            return None;
        };
        // Package sources cannot be opened by the project editor.
        if *id.root() != VirtualRoot::Project {
            return None;
        }
        let source = self.world.sources.get(&id)?;
        let line = source.lines().byte_to_line(offset)?;
        let start = source.lines().line_to_byte(line)?;
        Some(SourceLocation {
            path: id.vpath().get_without_slash().to_string(),
            line: line + 1,
            column: source.text().get(start..offset)?.chars().count() + 1,
        })
    }
}

// Reuse the app's Rust TLS HTTP client instead of adding an OpenSSL dependency.
struct PackageDownloader;
impl Downloader for PackageDownloader {
    fn stream(
        &self,
        _: &dyn std::any::Any,
        url: &str,
    ) -> std::io::Result<(Option<usize>, Box<dyn std::io::Read>)> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build()
            .into();
        let response = agent
            .get(url)
            .header(
                "User-Agent",
                concat!("typsmthng/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|error| match error {
                ureq::Error::StatusCode(404) => {
                    std::io::Error::new(std::io::ErrorKind::NotFound, error)
                }
                _ => std::io::Error::other(error),
            })?;
        Ok((None, Box::new(response.into_body().into_reader())))
    }
}

struct Request {
    project: Project,
    main: String,
    options: CompileOptions,
    reply: mpsc::Sender<Result<CompileOutput<Preview>>>,
    font_revision: u64,
}

/// One thread retains Typst's memoization, parsed sources and fonts across edits.
#[derive(Clone)]
pub struct PreviewCompiler {
    requests: mpsc::Sender<Request>,
    font_revision: Arc<AtomicU64>,
}

impl Default for PreviewCompiler {
    fn default() -> Self {
        let (requests, receiver) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("typst-preview".into())
            .spawn(move || {
                let mut engine: Option<Engine> = None;
                let mut font_revision = 0;
                for request in receiver {
                    let started = Instant::now();
                    let result = (|| {
                        let key = Config::new(&request.project, &request.options);
                        if font_revision != request.font_revision
                            || engine.as_ref().is_none_or(|engine| engine.config != key)
                        {
                            font_revision = request.font_revision;
                            engine = Some(Engine::new(key)?);
                        }
                        engine.as_mut().unwrap().compile(
                            &request.project,
                            &request.main,
                            &request.options,
                        )
                    })()
                    .map(|mut output| {
                        output.elapsed = started.elapsed();
                        output
                    });
                    typst::comemo::evict(10);
                    let _ = request.reply.send(result);
                }
            })
            .expect("start preview compiler");
        Self {
            requests,
            font_revision: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl PreviewCompiler {
    pub fn invalidate_fonts(&self) {
        self.font_revision.fetch_add(1, Ordering::Relaxed);
    }

    pub fn compile(
        &self,
        project: &Project,
        main: &str,
        options: &CompileOptions,
    ) -> Result<CompileOutput<Preview>> {
        let (reply, response) = mpsc::channel();
        self.requests
            .send(Request {
                project: project.clone(),
                main: main.into(),
                options: options.clone(),
                reply,
                font_revision: self.font_revision.load(Ordering::Relaxed),
            })
            .map_err(|_| BackendError::Process("Preview compiler stopped".into()))?;
        response
            .recv()
            .map_err(|_| BackendError::Process("Preview compiler stopped".into()))?
    }
}

#[derive(PartialEq, Eq)]
struct Config {
    root: PathBuf,
    font_paths: Vec<PathBuf>,
    ignore_system_fonts: bool,
    package_path: Option<PathBuf>,
    package_cache_path: Option<PathBuf>,
    creation_timestamp: Option<u64>,
}

impl Config {
    fn new(project: &Project, options: &CompileOptions) -> Self {
        Self {
            root: project.root().to_path_buf(),
            font_paths: options.font_paths.clone(),
            ignore_system_fonts: options.ignore_system_fonts,
            package_path: options.package_path.clone(),
            package_cache_path: options.package_cache_path.clone(),
            creation_timestamp: options.creation_timestamp,
        }
    }
}

struct Engine {
    config: Config,
    library: Arc<LazyHash<Library>>,
    fonts: Arc<FontStore>,
    files: FileStore<SystemFiles>,
    main: FileId,
    now: Time,
    wrapper: Option<tempfile::NamedTempFile>,
    wrapper_text: String,
    sources: Mutex<HashMap<FileId, Source>>,
}

impl Engine {
    fn new(config: Config) -> Result<Self> {
        let mut fonts = FontStore::new();
        if !config.ignore_system_fonts {
            fonts.extend(fonts::system());
        }
        fonts.extend(fonts::embedded());
        for path in &config.font_paths {
            fonts.extend(fonts::scan(path));
        }
        let packages = SystemPackages::from_parts(
            config
                .package_path
                .clone()
                .map(FsPackages::new)
                .or_else(FsPackages::system_data),
            config
                .package_cache_path
                .clone()
                .map(FsPackages::new)
                .or_else(FsPackages::system_cache),
            UniversePackages::new(PackageDownloader),
        );
        let now = match config.creation_timestamp {
            Some(timestamp) => Time::fixed_timestamp(
                i64::try_from(timestamp)
                    .map_err(|error| BackendError::Process(error.to_string()))?,
            )
            .map_err(|error| BackendError::Process(error.to_string()))?,
            None => Time::system(),
        };
        Ok(Self {
            files: FileStore::new(SystemFiles::new(FsRoot::new(config.root.clone()), packages)),
            main: RootedPath::new(VirtualRoot::Project, VirtualPath::new("main.typ").unwrap())
                .intern(),
            config,
            library: Arc::new(LazyHash::new(Library::default())),
            fonts: Arc::new(fonts),
            now,
            wrapper: None,
            wrapper_text: String::new(),
            sources: Mutex::new(HashMap::new()),
        })
    }

    fn compile(
        &mut self,
        project: &Project,
        main: &str,
        options: &CompileOptions,
    ) -> Result<CompileOutput<Preview>> {
        let started = Instant::now();
        let cancelled = || {
            options
                .cancellation
                .as_ref()
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
        };
        if cancelled() {
            return Err(BackendError::Process("Compilation superseded".into()));
        }
        super::paths::safe_existing_path(project.root(), main)?;
        // Keep the preamble entry's FileId stable so it too participates in reuse.
        let wrapper_text = format!(
            "{}\n{main}",
            options.page_preamble.as_deref().unwrap_or_default()
        );
        let entry = match &self.wrapper {
            Some(wrapper) if self.wrapper_text == wrapper_text => wrapper.path().to_path_buf(),
            _ => {
                let (entry, wrapper) = compile_entry(project, main, options)?;
                self.wrapper = wrapper;
                self.wrapper_text = wrapper_text;
                entry
            }
        };
        let vpath = VirtualPath::virtualize(&self.config.root, &entry)
            .map_err(|error| BackendError::Process(error.to_string()))?;
        self.main = RootedPath::new(VirtualRoot::Project, vpath).intern();
        self.sources.get_mut().unwrap().clear();
        self.files.reset();
        self.now.reset();
        let result = typst::compile::<PagedDocument>(self);
        let mut stderr = self.diagnostics(&result.warnings);
        let artifact = match result.output {
            Ok(document) if !cancelled() => {
                let pages = document
                    .pages()
                    .iter()
                    .enumerate()
                    .map(|(index, page)| SvgPage {
                        page: index + 1,
                        svg: typst_svg::svg(
                            page,
                            &typst_svg::SvgOptions {
                                render_bleed: false,
                                pretty: false,
                            },
                        ),
                        width_points: Some(page.frame.width().to_pt()),
                        height_points: Some(page.frame.height().to_pt()),
                    })
                    .collect();
                let sources = self.sources.get_mut().unwrap().clone();
                let world = Snapshot {
                    library: self.library.clone(),
                    fonts: self.fonts.clone(),
                    main: self.main,
                    sources,
                };
                Some(Preview {
                    pages,
                    source_map: Arc::new(SourceMap { document, world }),
                })
            }
            Ok(_) => return Err(BackendError::Process("Compilation superseded".into())),
            Err(errors) => {
                stderr.push_str(&self.diagnostics(&errors));
                None
            }
        };
        Ok(CompileOutput {
            artifact,
            diagnostics: parse_diagnostics(&stderr, project.root()),
            stdout: String::new(),
            stderr,
            elapsed: started.elapsed(),
        })
    }

    fn diagnostics(&self, messages: &[SourceDiagnostic]) -> String {
        let mut buffer = diagnostics::termcolor::Buffer::no_color();
        let _ = diagnostics::emit(&mut buffer, self, messages, DiagnosticFormat::Short);
        String::from_utf8_lossy(buffer.as_slice()).into_owned()
    }
}

impl World for Engine {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        let source = self.files.source(id)?;
        self.sources.lock().unwrap().insert(id, source.clone());
        Ok(source)
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.file(id)
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }
    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        self.now.today(offset)
    }
}

impl DiagnosticWorld for Engine {
    fn name(&self, id: FileId) -> String {
        self.files
            .loader()
            .resolve(id)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| id.vpath().get_with_slash().into())
    }
}

struct Snapshot {
    library: Arc<LazyHash<Library>>,
    fonts: Arc<FontStore>,
    main: FileId,
    sources: HashMap<FileId, Source>,
}

impl World for Snapshot {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        self.sources
            .get(&id)
            .cloned()
            .ok_or_else(|| FileError::NotFound(Path::new(id.vpath().get_with_slash()).into()))
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.source(id)
            .map(|source| Bytes::from_string(source.text().to_string()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

impl IdeWorld for Snapshot {
    fn upcast(&self) -> &dyn World {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn options() -> CompileOptions {
        CompileOptions {
            ignore_system_fonts: true,
            creation_timestamp: Some(0),
            ..Default::default()
        }
    }

    fn fixture(source: &str) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.typ"), source).unwrap();
        let project = Project::open(dir.path()).unwrap();
        (dir, project)
    }

    fn compiled(compiler: &PreviewCompiler, project: &Project) -> Preview {
        let output = compiler.compile(project, "main.typ", &options()).unwrap();
        assert!(output.success(), "{}", output.stderr);
        output.artifact.unwrap()
    }

    fn find_jump(
        map: &SourceMap,
        page: usize,
        path: &str,
        line: usize,
    ) -> (f64, f64, SourceLocation) {
        let (width, height) = map.dimensions(page).unwrap();
        for y in (0..height as usize).step_by(2) {
            for x in (0..width as usize).step_by(2) {
                if let Some(location) = map.jump(page, x as f64, y as f64) {
                    if location.path == path && location.line == line {
                        return (x as f64, y as f64, location);
                    }
                }
            }
        }
        panic!("No jump to {path}:{line} on page {page}");
    }

    #[test]
    fn persistent_preview_revalidates_dependencies_and_recovers_after_errors() {
        let (_dir, project) = fixture("#include \"part.typ\"\n#read(\"data.txt\")");
        fs::write(project.root().join("part.typ"), "Alpha").unwrap();
        fs::write(project.root().join("data.txt"), "First").unwrap();
        let compiler = PreviewCompiler::default();
        let a = compiled(&compiler, &project);
        assert_eq!(a.pages, compiled(&compiler, &project).pages);
        fs::write(project.root().join("part.typ"), "Bravo").unwrap();
        let b = compiled(&compiler, &project);
        assert_ne!(a.pages, b.pages);
        fs::write(project.root().join("data.txt"), "Other").unwrap();
        assert_ne!(b.pages, compiled(&compiler, &project).pages);
        fs::remove_file(project.root().join("part.typ")).unwrap();
        let failure = compiler.compile(&project, "main.typ", &options()).unwrap();
        assert!(!failure.success());
        assert_eq!(failure.diagnostics[0].path, Some(PathBuf::from("main.typ")));
        fs::write(project.root().join("part.typ"), "Fixed").unwrap();
        assert!(compiler
            .compile(&project, "main.typ", &options())
            .unwrap()
            .success());
        let (_other, other) = fixture("Another root");
        assert_ne!(
            compiled(&compiler, &other).pages,
            compiled(&compiler, &project).pages
        );
    }

    #[test]
    fn source_jumps_track_math_includes_unicode_and_source_only_edits() {
        let (_dir, project) = fixture("#set page(width: 240pt, height: 180pt, margin: 20pt)\nHello café λ\n\n$ x^2 + y^2 = z^2 $\n#pagebreak()\n#include \"part.typ\"");
        fs::write(
            project.root().join("part.typ"),
            "// included\n#rotate(12deg)[Included formula $a+b$]",
        )
        .unwrap();
        let compiler = PreviewCompiler::default();
        let first = compiled(&compiler, &project);
        find_jump(&first.source_map, 0, "main.typ", 2);
        let (x, y, old_location) = find_jump(&first.source_map, 0, "main.typ", 4);
        find_jump(&first.source_map, 1, "part.typ", 2);
        // Comments move source lines without changing any preview pixels.
        let source = fs::read_to_string(project.root().join("main.typ")).unwrap();
        fs::write(
            project.root().join("main.typ"),
            format!("// inserted\n{source}"),
        )
        .unwrap();
        let second = compiled(&compiler, &project);
        assert_eq!(first.pages, second.pages);
        assert_eq!(second.source_map.jump(0, x, y).unwrap().line, 5);
        assert_eq!(first.source_map.jump(0, x, y), Some(old_location));
        assert!(second.source_map.jump(20, x, y).is_none());
        assert!(second.source_map.jump(0, f64::NAN, y).is_none());
        // Validate GTK character columns rather than UTF-8 byte columns.
        let source = &second
            .source_map
            .world
            .sources
            .values()
            .find(|s| s.text().contains("Hello"))
            .unwrap();
        for y in 20..45 {
            for x in 20..130 {
                if let Some(location) = second.source_map.jump(0, x as f64, y as f64) {
                    if location.line == 3 {
                        assert!(
                            location.column
                                <= source.text().lines().nth(2).unwrap().chars().count() + 1
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn preamble_does_not_redirect_jumps_to_temporary_wrapper() {
        let (_dir, project) = fixture("$ integral_0^1 x dif x $\n");
        let mut options = options();
        options.page_preamble = Some("#set page(width: 240pt, height: 180pt, margin: 20pt)".into());
        let compiler = PreviewCompiler::default();
        let output = compiler.compile(&project, "main.typ", &options).unwrap();
        assert!(output.success(), "{}", output.stderr);
        let preview = output.artifact.unwrap();
        find_jump(&preview.source_map, 0, "main.typ", 1);
        assert_eq!(preview.source_map.dimensions(0), Some((240.0, 180.0)));
        assert_eq!(
            preview.pages,
            compiler
                .compile(&project, "main.typ", &options)
                .unwrap()
                .artifact
                .unwrap()
                .pages
        );
    }

    #[test]
    #[ignore = "manual release benchmark requiring Typst 0.15.1"]
    fn benchmark_incremental_preview_against_cli() {
        let tool = super::super::TypstTool::detect().unwrap();
        let (_dir, project) = fixture("");
        let compiler = PreviewCompiler::default();
        let mut options = options();
        options.ignore_system_fonts = std::env::var_os("TYPSMTHNG_BENCH_SYSTEM_FONTS").is_none();
        eprintln!("system fonts: {}", !options.ignore_system_fonts);
        let mut cli = Vec::new();
        let mut warm = Vec::new();
        for revision in 0..12 {
            let source = (0..10)
                .map(|page| {
                    format!(
                        "= Page {page}\nHello café. $x^2 + y^2 = z^2$\nRevision {}",
                        if page == 4 { revision } else { 0 }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n#pagebreak()\n");
            fs::write(project.root().join("main.typ"), source).unwrap();
            let start = Instant::now();
            let old = tool
                .compile_svg_with_options(&project, "main.typ", &options)
                .unwrap();
            let old_elapsed = start.elapsed();
            let start = Instant::now();
            let new = compiler.compile(&project, "main.typ", &options).unwrap();
            let new_elapsed = start.elapsed();
            assert!(old.success(), "{}", old.stderr);
            assert!(new.success(), "{}", new.stderr);
            // Exporter whitespace may differ. Compare after removing whitespace
            // between tags, keeping text/attributes intact.
            let compact = |svg: &str| {
                svg.split(">\n")
                    .map(str::trim_start)
                    .collect::<Vec<_>>()
                    .join(">")
            };
            let old_pages = old.artifact.unwrap();
            let new_pages = new.artifact.unwrap().pages;
            assert_eq!(old_pages.len(), new_pages.len());
            for (old, new) in old_pages.iter().zip(&new_pages) {
                assert!(
                    compact(&old.svg) == compact(&new.svg),
                    "SVG differs on page {}",
                    old.page
                );
            }
            if revision == 0 {
                eprintln!("cold: CLI {old_elapsed:?}, persistent {new_elapsed:?}");
            } else {
                cli.push(old_elapsed);
                warm.push(new_elapsed);
            }
        }
        cli.sort();
        warm.sort();
        eprintln!("10-page single-page edits, 11 warm runs: CLI median {:?}, persistent median {:?}, persistent max {:?}", cli[5], warm[5], warm[10]);
    }
}
