//! Persistent SVG preview. The FileStore reset/reuse approach follows the local
//! typrst compatible engine; PDF export and CLI subprocesses stay off this path.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::introspection::{Location, PagedPosition, Tag};
use typst::layout::{Abs, Frame, FrameItem, Point, Rect, Transform};
use typst::math::EquationElem;
use typst::syntax::{FileId, RootedPath, Source, Span, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::{LazyHash, Numeric};
use typst::visualize::{FillRule, FixedStroke, Geometry, Paint};
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
    /// Per-page ink extents, measured on the compiler thread at build time.
    content: Vec<PageContent>,
    column: Option<(f64, f64)>,
}

/// Axis-aligned box in page points, origin at the page's top-left corner
/// with y growing downwards: the same space as [`SourceMap::dimensions`]
/// and [`SourceMap::jump`]. Always lies within the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl SourceMap {
    fn new(document: PagedDocument, world: Snapshot) -> Self {
        let content = document
            .pages()
            .iter()
            .map(|page| PageContent::measure(&page.frame))
            .collect::<Vec<_>>();
        let column = content_column(&content);
        Self {
            document,
            world,
            content,
            column,
        }
    }

    pub fn dimensions(&self, page: usize) -> Option<(f64, f64)> {
        let frame = &self.document.pages().get(page)?.frame;
        Some((frame.width().to_pt(), frame.height().to_pt()))
    }

    /// Union of everything visibly drawn on `page` (text ink, stroked shapes,
    /// images, headers, footers, page backgrounds), clipped to the page.
    /// `page.fill` is not part of the frame and never counts. `None` for a
    /// blank page or an out-of-range index.
    pub fn content_bounds(&self, page: usize) -> Option<ContentBounds> {
        let bounds = self.content.get(page)?.bounds?;
        Some(ContentBounds {
            x: bounds.min.x.to_pt(),
            y: bounds.min.y.to_pt(),
            width: (bounds.max.x - bounds.min.x).to_pt(),
            height: (bounds.max.y - bounds.min.y).to_pt(),
        })
    }

    /// Document-wide horizontal content extent `(x_min, x_max)` in page
    /// points from the left page edge, for a zoom that stays put while
    /// scrolling. Measured over the pages that share the most common page
    /// width (the UI renders every page at one widget width, so points on
    /// differently sized pages would map to other pixels), ignoring
    /// edge-to-edge backdrops. See [`content_column`].
    pub fn content_column(&self) -> Option<(f64, f64)> {
        self.column
    }

    pub fn jump(&self, page: usize, x: f64, y: f64) -> Option<SourceLocation> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let position = PagedPosition {
            page: page.checked_add(1)?.try_into().ok()?,
            point: Point::new(Abs::pt(x), Abs::pt(y)),
        };
        let (id, offset) = match typst_ide::jump_from_click(&self.world, &self.document, &position)
        {
            Some(Jump::File(id, offset)) => (id, offset),
            Some(_) => return None,
            None => {
                let frame = &self.document.pages().get(page)?.frame;
                let span = equation_from_click(frame, position.point)?;
                let id = span.id()?;
                let source = self.world.sources.get(&id)?;
                (id, source.find(span)?.offset())
            }
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

/// Ink measured on one page. `column` omits backdrops; see [`content_column`].
#[derive(Debug, Clone, Copy)]
struct PageContent {
    width: Abs,
    bounds: Option<Rect>,
    column: Option<(Abs, Abs)>,
}

impl PageContent {
    fn measure(frame: &Frame) -> Self {
        let page = Rect::from_pos_size(Point::zero(), frame.size());
        // Anything this close to both paper edges is a backdrop, not text column.
        let tolerance = frame.width() * 0.02;
        let mut content = Self {
            width: frame.width(),
            bounds: None,
            column: None,
        };
        visit_ink(
            frame,
            Transform::identity(),
            page,
            &mut |ink, backdrop_candidate| {
                content.bounds = Some(union(content.bounds, ink));
                let backdrop = backdrop_candidate
                    && ink.min.x <= tolerance
                    && ink.max.x >= frame.width() - tolerance;
                if !backdrop {
                    content.column = Some(match content.column {
                        Some((min, max)) => (min.min(ink.min.x), max.max(ink.max.x)),
                        None => (ink.min.x, ink.max.x),
                    });
                }
            },
        );
        content
    }
}

/// Column robustness: take the plain union of every page's horizontal ink,
/// minus shapes and images that reach both paper edges (within 2% of the page
/// width). We deliberately do not use a median or percentile: the point of the
/// mode is that all content stays visible, and a percentile would silently crop
/// the one page with a wide table, an overflowing equation or a margin note.
/// The pages that do break a plain union are almost always decoration: page
/// backgrounds, full-bleed cover images and edge-to-edge colour bands. By
/// construction those touch both edges of the paper, which a margin-bounded
/// text column never does, so dropping them keeps the column tight without
/// hiding information. Text is never discarded, so a full-width page of text
/// still (correctly) widens the column to the full page. Pages whose width
/// differs from the most common page width (a landscape insert) are skipped,
/// because their points map to different preview pixels. If every page is
/// pure backdrop, fall back to their full ink bounds.
fn content_column(pages: &[PageContent]) -> Option<(f64, f64)> {
    let same_width = |a: Abs, b: Abs| (a - b).abs() < Abs::pt(0.01);
    // `rev` makes ties resolve to the earliest page.
    let width = pages
        .iter()
        .rev()
        .filter(|page| page.bounds.is_some())
        .max_by_key(|page| {
            pages
                .iter()
                .filter(|other| other.bounds.is_some() && same_width(page.width, other.width))
                .count()
        })?
        .width;
    let pages = pages.iter().filter(|page| same_width(page.width, width));
    let union = |extents: &mut dyn Iterator<Item = (Abs, Abs)>| {
        extents.reduce(|(a_min, a_max), (b_min, b_max)| (a_min.min(b_min), a_max.max(b_max)))
    };
    let (min, max) = union(&mut pages.clone().filter_map(|page| page.column)).or_else(|| {
        union(
            &mut pages
                .filter_map(|page| page.bounds)
                .map(|bounds| (bounds.min.x, bounds.max.x)),
        )
    })?;
    Some((min.to_pt(), max.to_pt()))
}

fn union(previous: Option<Rect>, next: Rect) -> Rect {
    match previous {
        Some(previous) => Rect::new(previous.min.min(next.min), previous.max.max(next.max)),
        None => next,
    }
}

fn is_visible(paint: &Paint) -> bool {
    match paint {
        Paint::Solid(color) => color.alpha().is_none_or(|alpha| alpha > 0.0),
        Paint::Gradient(_) | Paint::Tiling(_) => true,
    }
}

fn visible_stroke(stroke: Option<&FixedStroke>) -> Option<&FixedStroke> {
    stroke.filter(|stroke| stroke.thickness > Abs::zero() && is_visible(&stroke.paint))
}

/// Calls `emit` with the page-space bounding box of every visible leaf item
/// and whether it may be a backdrop (filled shapes and images, never text).
/// `ts` maps `frame` coordinates to page coordinates; `clip` is the page-space
/// bounding box of all enclosing clips. Rotated clips are approximated by
/// their bounding box, which can only over-estimate the ink.
fn visit_ink(frame: &Frame, ts: Transform, clip: Rect, emit: &mut dyn FnMut(Rect, bool)) {
    for (pos, item) in frame.items() {
        let local = ts.pre_concat(Transform::translate(pos.x, pos.y));
        let (bounds, backdrop_candidate) = match item {
            FrameItem::Group(group) => {
                let ts = local.pre_concat(group.transform);
                let clip = match &group.clip {
                    Some(curve) => match intersect(clip, transformed(curve.bbox(None), ts)) {
                        Some(clip) => clip,
                        None => continue,
                    },
                    None => clip,
                };
                visit_ink(&group.frame, ts, clip, emit);
                continue;
            }
            FrameItem::Text(text) => {
                let stroke = visible_stroke(text.stroke.as_ref());
                if !is_visible(&text.fill) && stroke.is_none() {
                    continue;
                }
                // Ink boxes rather than advances: spaces have no ink, and
                // italic overhangs and accents may leave the advance box.
                let bbox = text.bbox();
                let grow = stroke.map_or(Abs::zero(), |stroke| stroke.thickness / 2.0);
                // Text bounding boxes use a Y-up font coordinate system.
                let bbox = Rect::new(bbox.min.min(bbox.max), bbox.min.max(bbox.max));
                (
                    Rect::new(bbox.min - Point::splat(grow), bbox.max + Point::splat(grow)),
                    false,
                )
            }
            FrameItem::Shape(shape, _) => {
                let stroke = visible_stroke(shape.stroke.as_ref());
                let filled = shape.fill.as_ref().is_some_and(is_visible)
                    && !matches!(shape.geometry, Geometry::Line(_));
                if !filled && stroke.is_none() {
                    continue;
                }
                let bbox = shape.geometry.bbox(stroke);
                if stroke.is_none() && (bbox.size().x.is_zero() || bbox.size().y.is_zero()) {
                    continue;
                }
                (bbox, true)
            }
            FrameItem::Image(_, size, _) => {
                if size.x.is_zero() || size.y.is_zero() {
                    continue;
                }
                (Rect::from_pos_size(Point::zero(), *size), true)
            }
            FrameItem::Link(..) | FrameItem::Tag(..) => continue,
        };
        if !bounds.min.is_finite() || !bounds.max.is_finite() {
            continue;
        }
        if let Some(bounds) = intersect(transformed(bounds, local), clip) {
            emit(bounds, backdrop_candidate);
        }
    }
}

fn transformed(rect: Rect, ts: Transform) -> Rect {
    let corners = [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        Point::new(rect.min.x, rect.max.y),
        rect.max,
    ]
    .map(|point| point.transform_inf(ts));
    Rect::new(
        corners.into_iter().fold(corners[0], Point::min),
        corners.into_iter().fold(corners[0], Point::max),
    )
}

/// Overlap of two boxes, `None` when disjoint. Degenerate overlaps are kept
/// so that axis-aligned hairlines still count.
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let rect = Rect::new(a.min.max(b.min), a.max.min(b.max));
    (rect.min.x <= rect.max.x && rect.min.y <= rect.max.y).then_some(rect)
}

struct EquationBounds {
    location: Location,
    span: Span,
    bounds: Option<Rect>,
}

/// Typst's glyph hit boxes miss descenders, tall math symbols and generated
/// glyphs with detached spans. Equation tags provide a source for the whole
/// formula, including the space between its glyphs and fraction strokes.
fn equation_from_click(frame: &Frame, click: Point) -> Option<Span> {
    // Prefer the deepest, frontmost equation and use its local coordinates.
    for (pos, item) in frame.items().rev() {
        if let FrameItem::Group(group) = item {
            let Some(inverse) = group.transform.invert() else {
                continue;
            };
            let local = (click - *pos).transform_inf(inverse);
            if group
                .clip
                .as_ref()
                .is_some_and(|clip| !clip.contains(FillRule::NonZero, local))
            {
                continue;
            }
            if let Some(span) = equation_from_click(&group.frame, local) {
                return Some(span);
            }
        }
    }

    let mut equations: Vec<EquationBounds> = Vec::new();
    let mut target = None;
    for (pos, item) in frame.items() {
        match item {
            FrameItem::Tag(Tag::Start(elem, _)) if elem.is::<EquationElem>() => {
                let Some(location) = elem.location() else {
                    continue;
                };
                equations.push(EquationBounds {
                    location,
                    span: elem.span(),
                    bounds: None,
                });
            }
            FrameItem::Tag(Tag::End(location, ..)) => {
                if let Some(index) = equations.iter().rposition(|eq| eq.location == *location) {
                    let equation = equations.remove(index);
                    if equation
                        .bounds
                        .is_some_and(|bounds| contains(bounds, click))
                    {
                        target = Some(equation.span);
                    }
                }
            }
            _ if !equations.is_empty() => {
                let bounds = match item {
                    FrameItem::Text(text) => text.bbox(),
                    FrameItem::Group(group) => {
                        let Some(inverse) = group.transform.invert() else {
                            continue;
                        };
                        let local = (click - *pos).transform_inf(inverse);
                        if group
                            .clip
                            .as_ref()
                            .is_some_and(|clip| !clip.contains(FillRule::NonZero, local))
                        {
                            continue;
                        }
                        let size = group.frame.size();
                        let corners = [
                            Point::zero(),
                            Point::with_x(size.x),
                            Point::with_y(size.y),
                            size.to_point(),
                        ]
                        .map(|point| point.transform_inf(group.transform));
                        Rect::new(
                            corners.into_iter().reduce(Point::min)?,
                            corners.into_iter().reduce(Point::max)?,
                        )
                    }
                    FrameItem::Shape(shape, _) => match &shape.geometry {
                        Geometry::Line(to) => Rect::new(Point::zero(), *to),
                        Geometry::Rect(size) => Rect::new(Point::zero(), size.to_point()),
                        Geometry::Curve(curve) => curve.bbox(shape.stroke.as_ref()),
                    },
                    FrameItem::Image(_, size, _) => Rect::new(Point::zero(), size.to_point()),
                    FrameItem::Link(..) | FrameItem::Tag(..) => continue,
                };
                // Text bounding boxes use a Y-up font coordinate system.
                let bounds = Rect::new(
                    bounds.min.min(bounds.max) + *pos,
                    bounds.min.max(bounds.max) + *pos,
                );
                if !bounds.min.is_finite() || !bounds.max.is_finite() {
                    continue;
                }
                for equation in &mut equations {
                    equation.bounds = Some(match equation.bounds {
                        Some(previous) => {
                            Rect::new(previous.min.min(bounds.min), previous.max.max(bounds.max))
                        }
                        None => bounds,
                    });
                }
            }
            _ => {}
        }
    }
    target
}

fn contains(bounds: Rect, point: Point) -> bool {
    bounds.min.x <= point.x
        && point.x <= bounds.max.x
        && bounds.min.y <= point.y
        && point.y <= bounds.max.y
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
                    source_map: Arc::new(SourceMap::new(document, world)),
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
    fn source_jumps_cover_equation_bounds_and_generated_symbols() {
        let (_dir, project) = fixture("#set page(width: 240pt, height: 240pt, margin: 20pt)\n$ integral_0^1 x dif x $\n$ frac(a, b) $\n$ sqrt(frac(a, b)) $\nInline $g+h$ text");
        let preview = compiled(&PreviewCompiler::default(), &project);
        // Coordinates use the pinned compiler's embedded fonts, without any
        // system-font substitution. Every point misses Typst's own hit test.
        for (x, y, line, column) in [
            (102., 42., 2, 1),  // integral below its baseline
            (106., 24., 2, 1),  // integral above the font-size hit box
            (129., 32., 2, 1),  // generated differential d, with no source span
            (120., 70., 3, 1),  // gap between numerator and fraction bar
            (113., 120., 4, 1), // lower part of the radical
            (52., 144., 5, 8),  // inline g descender
            (55., 140., 5, 8),  // gap between inline math glyphs
        ] {
            let position = PagedPosition {
                page: 1.try_into().unwrap(),
                point: Point::new(Abs::pt(x), Abs::pt(y)),
            };
            assert_eq!(
                typst_ide::jump_from_click(
                    &preview.source_map.world,
                    &preview.source_map.document,
                    &position
                ),
                None,
                "expected an upstream hit-test miss at {x}, {y}"
            );
            assert_eq!(
                preview.source_map.jump(0, x, y),
                Some(SourceLocation {
                    path: "main.typ".into(),
                    line,
                    column,
                })
            );
        }
        // Exact glyph hits still use Typst's more precise source offsets.
        assert_eq!(preview.source_map.jump(0, 120., 64.).unwrap().column, 8);
        for (x, y) in [
            (99., 37.),
            (20., 50.),
            (120., 90.),
            (100., 67.),
            (200., 200.),
        ] {
            assert_eq!(preview.source_map.jump(0, x, y), None);
        }
    }

    fn integral_descender(frame: &Frame) -> Option<Point> {
        for (pos, item) in frame.items() {
            match item {
                FrameItem::Text(text) if text.text == "∫" => {
                    return Some(*pos + Point::new(Abs::pt(6.), Abs::pt(5.)));
                }
                FrameItem::Group(group) => {
                    if let Some(point) = integral_descender(&group.frame) {
                        return Some(point.transform_inf(group.transform) + *pos);
                    }
                }
                _ => {}
            }
        }
        None
    }

    #[test]
    fn equation_fallback_tracks_transformed_imports_and_source_only_edits() {
        let (_dir, project) =
            fixture("#set page(width: 240pt, height: 180pt, margin: 20pt)\n#include \"part.typ\"");
        let source = "// included\n#rotate(20deg, reflow: true)[$ integral_0^1 x dif x $]";
        fs::write(project.root().join("part.typ"), source).unwrap();
        let compiler = PreviewCompiler::default();
        let first = compiled(&compiler, &project);
        let point = integral_descender(&first.source_map.document.pages()[0].frame).unwrap();
        let expected = SourceLocation {
            path: "part.typ".into(),
            line: 2,
            column: 30,
        };
        assert_eq!(
            first.source_map.jump(0, point.x.to_pt(), point.y.to_pt()),
            Some(expected)
        );
        fs::write(
            project.root().join("part.typ"),
            format!("// inserted\n{source}"),
        )
        .unwrap();
        let second = compiled(&compiler, &project);
        assert_eq!(first.pages, second.pages);
        assert_eq!(
            second
                .source_map
                .jump(0, point.x.to_pt(), point.y.to_pt())
                .unwrap()
                .line,
            3
        );
        assert_eq!(
            first
                .source_map
                .jump(0, point.x.to_pt(), point.y.to_pt())
                .unwrap()
                .line,
            2
        );
    }

    #[test]
    fn equation_fallback_checks_clips_in_transformed_group_coordinates() {
        use typst::layout::{GroupItem, Size, Transform};
        use typst::visualize::Curve;

        let (_dir, project) = fixture(
            "#set page(width: 240pt, height: 180pt, margin: 20pt)\n$ integral_0^1 x dif x $",
        );
        let preview = compiled(&PreviewCompiler::default(), &project);
        let original = &preview.source_map.document.pages()[0].frame;
        let expected = original
            .items()
            .find_map(|(_, item)| match item {
                FrameItem::Tag(Tag::Start(elem, _)) if elem.is::<EquationElem>() => {
                    Some(elem.span())
                }
                _ => None,
            })
            .unwrap();
        let offset = Point::new(Abs::pt(200.), Abs::pt(50.));
        let click = Point::new(Abs::pt(102.), Abs::pt(42.)) + offset;

        // Cover tags inside the transformed group and tags surrounding it.
        for tags_inside in [true, false] {
            let mut frame = if tags_inside {
                let mut wrapper = Frame::soft(original.size());
                wrapper.push(
                    Point::zero(),
                    FrameItem::Group(GroupItem::new(original.clone())),
                );
                wrapper
            } else {
                original.clone()
            };
            frame.retain(|item| {
                if let FrameItem::Group(group) = item {
                    group.transform = Transform::translate(offset.x, offset.y);
                    group.clip = Some(Curve::rect(group.frame.size()));
                }
                true
            });
            assert_eq!(
                equation_from_click(&frame, click),
                Some(expected),
                "tags_inside={tags_inside}"
            );
            frame.retain(|item| {
                if let FrameItem::Group(group) = item {
                    group.clip = Some(Curve::rect(Size::splat(Abs::pt(1.))));
                }
                true
            });
            assert_eq!(equation_from_click(&frame, click), None);
        }
    }

    #[test]
    fn equation_fallback_respects_clipping_and_empty_equations() {
        let (_dir, project) = fixture("#set page(width: 240pt, height: 180pt, margin: 20pt)\n#box(width: 12pt, height: 12pt, clip: true)[$ integral_0^1 x dif x $]");
        let compiler = PreviewCompiler::default();
        let clipped = compiled(&compiler, &project);
        let point = integral_descender(&clipped.source_map.document.pages()[0].frame).unwrap();
        assert_eq!(
            clipped.source_map.jump(0, point.x.to_pt(), point.y.to_pt()),
            None
        );
        fs::write(
            project.root().join("main.typ"),
            "#set page(width: 240pt, height: 180pt, margin: 20pt)\n$ \" \" $",
        )
        .unwrap();
        let empty = compiled(&compiler, &project);
        for (x, y) in [(0., 0.), (100., 30.), (120., 50.), (220., 160.)] {
            assert_eq!(empty.source_map.jump(0, x, y), None);
        }
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

    fn source_map(source: &str) -> Arc<SourceMap> {
        let (_dir, project) = fixture(source);
        compiled(&PreviewCompiler::default(), &project).source_map
    }

    fn assert_near(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: {actual} is not within {tolerance} of {expected}"
        );
    }

    fn assert_bounds(actual: Option<ContentBounds>, expected: [f64; 4]) {
        let actual = actual.expect("content bounds");
        let actual = [actual.x, actual.y, actual.width, actual.height];
        for (name, (actual, expected)) in ["x", "y", "width", "height"]
            .into_iter()
            .zip(actual.into_iter().zip(expected))
        {
            assert_near(actual, expected, 1e-6, name);
        }
    }

    // Typst's default margin is 2.5/21 of the shorter page side.
    const A4_WIDTH: f64 = 595.2755905511812;
    const A4_MARGIN: f64 = 2.5 / 21.0 * A4_WIDTH;

    #[test]
    fn content_bounds_follow_default_a4_margins() {
        let map = source_map("#set par(justify: true)\n#lorem(400)");
        assert_near(A4_MARGIN, 70.866, 0.001, "default margin");
        let bounds = map.content_bounds(0).unwrap();
        // Ink starts at the margin; hanging punctuation may overhang slightly.
        assert_near(bounds.x, A4_MARGIN, 0.5, "left");
        assert_near(bounds.x + bounds.width, A4_WIDTH - A4_MARGIN, 2.0, "right");
        assert_near(bounds.y, A4_MARGIN, 1.0, "top");
        assert!(bounds.y + bounds.height < 841.89 - A4_MARGIN);
        let (min, max) = map.content_column().unwrap();
        assert_eq!(min, bounds.x);
        assert_near(max, bounds.x + bounds.width, 1e-9, "column");
        // Ragged text is narrower: the column follows ink, not the margins.
        let ragged = source_map("#lorem(400)").content_column().unwrap();
        assert_near(ragged.0, A4_MARGIN, 0.5, "ragged left");
        assert!(ragged.1 < max && ragged.1 > A4_WIDTH - A4_MARGIN - 20.0);
    }

    #[test]
    fn content_bounds_follow_custom_margins_and_ignore_page_fill() {
        let source = "#set page(margin: 1in)\n#set par(justify: true)\n#lorem(300)";
        let plain = source_map(source);
        let (min, max) = plain.content_column().unwrap();
        assert_near(min, 72.0, 0.5, "left");
        assert_near(max, A4_WIDTH - 72.0, 2.0, "right");
        let filled = source_map(&format!("#set page(fill: gray)\n{source}"));
        assert_eq!(filled.content_bounds(0), plain.content_bounds(0));
        assert_eq!(filled.content_column(), plain.content_column());
    }

    #[test]
    fn content_bounds_include_wide_equations_clipped_to_the_page() {
        let terms = (0..40).map(|i| format!("x_{i}")).collect::<Vec<_>>();
        let map = source_map(&format!("Text\n$ {} $", terms.join(" + ")));
        let bounds = map.content_bounds(0).unwrap();
        // The overflowing equation spills into both margins, but never past
        // the paper, which the SVG preview does not show.
        assert_eq!(bounds.x, 0.0);
        assert_near(bounds.width, A4_WIDTH, 1e-9, "width");
        assert_eq!(map.content_column(), Some((0.0, A4_WIDTH)));
    }

    #[test]
    fn page_number_only_and_empty_pages() {
        let map = source_map("#set page(numbering: \"1\")\n#pagebreak()");
        for page in 0..2 {
            let bounds = map.content_bounds(page).unwrap();
            assert_near(bounds.x + bounds.width / 2.0, A4_WIDTH / 2.0, 1.0, "center");
            assert!(bounds.y > 841.89 - A4_MARGIN && bounds.width < 10.0);
        }
        let (min, max) = map.content_column().unwrap();
        assert!(min > 290.0 && max < 305.0);

        let map = source_map("");
        assert_eq!(map.dimensions(0), Some((A4_WIDTH, 841.8897637795276)));
        assert_eq!(map.content_bounds(0), None);
        assert_eq!(map.content_bounds(1), None);
        assert_eq!(map.content_column(), None);
        // Invisible paint, missing strokes and whitespace are not content.
        let map = source_map(
            "#text(fill: rgb(0, 0, 0, 0))[Hidden] #h(1em) \n#rect(fill: none, stroke: none)\n#rect(width: 0pt, height: 10pt, fill: red, stroke: none)\n#line(length: 10pt, stroke: 0pt)\n#link(\"https://example.com\")[#h(5pt)]",
        );
        assert_eq!(map.content_bounds(0), None);
        assert_eq!(map.content_column(), None);
    }

    #[test]
    fn content_bounds_apply_group_transforms_clips_and_placement() {
        let page = "#set page(width: 300pt, height: 300pt, margin: 50pt)\n";
        // A 100x20 box rotated about its centre (150, 60) becomes 20x100.
        let map = source_map(&format!(
            "{page}#align(center, rotate(90deg, box(width: 100pt, height: 20pt, fill: black)))"
        ));
        assert_bounds(map.content_bounds(0), [140.0, 10.0, 20.0, 100.0]);
        // Scaling about the centre (70, 70) of a 40pt square.
        let map = source_map(&format!(
            "{page}#scale(x: 200%, y: 50%, box(width: 40pt, height: 40pt, fill: black))"
        ));
        assert_bounds(map.content_bounds(0), [30.0, 60.0, 80.0, 20.0]);
        // Clips bound oversized children; the stroke is half outside the rect.
        let map = source_map(&format!(
            "{page}#box(width: 20pt, height: 20pt, clip: true, rect(width: 200pt, height: 200pt, fill: black))\n#place(bottom + right, rect(width: 10pt, height: 10pt, stroke: 2pt))"
        ));
        assert_bounds(map.content_bounds(0), [50.0, 50.0, 201.0, 201.0]);
        // Placed margin notes widen the column; so do stroked lines.
        let map = source_map(&format!(
            "{page}#place(dx: -40pt, box(width: 10pt, height: 10pt, fill: black))\n#line(length: 100pt, stroke: 4pt)"
        ));
        let bounds = map.content_bounds(0).unwrap();
        assert_near(bounds.x, 10.0, 1e-6, "margin note");
        assert_near(bounds.x + bounds.width, 150.0, 1e-6, "line end");
        assert_eq!(map.content_column(), Some((bounds.x, 150.0)));
    }

    #[test]
    fn two_column_layout_spans_both_columns() {
        let map = source_map("#set page(columns: 2)\n#set par(justify: true)\n#lorem(900)");
        let (min, max) = map.content_column().unwrap();
        assert_near(min, A4_MARGIN, 0.5, "left");
        assert_near(max, A4_WIDTH - A4_MARGIN, 2.0, "right");
    }

    #[test]
    fn content_column_ignores_backdrops_and_differently_sized_pages() {
        let (_dir, project) = fixture(concat!(
            "#set par(justify: true)\n",
            "#page(margin: 0pt, image(\"cover.svg\", width: 100%))\n",
            "#page(background: rect(width: 100%, height: 100%, fill: gray))[#lorem(300)]\n",
            "#page(flipped: true, margin: 5pt)[#lorem(300)]\n",
            "#lorem(300)",
        ));
        fs::write(
            project.root().join("cover.svg"),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>"#,
        )
        .unwrap();
        let map = compiled(&PreviewCompiler::default(), &project).source_map;
        // Page bounds still report everything drawn, backdrops included.
        let cover = map.content_bounds(0).unwrap();
        assert_eq!((cover.x, cover.width), (0.0, A4_WIDTH));
        assert_bounds(
            map.content_bounds(1),
            [0.0, 0.0, A4_WIDTH, 841.8897637795276],
        );
        let wide = map.content_bounds(2).unwrap();
        assert!(wide.width > 800.0);
        // The column is the text column of the portrait text pages.
        let (min, max) = map.content_column().unwrap();
        assert_near(min, A4_MARGIN, 0.5, "left");
        assert_near(max, A4_WIDTH - A4_MARGIN, 2.0, "right");

        // A page of pure backdrop falls back to its full extent.
        let map = source_map("#set page(background: rect(width: 100%, height: 100%, fill: gray))");
        assert_eq!(map.content_column(), Some((0.0, A4_WIDTH)));
        // Text is never treated as a backdrop, even edge to edge.
        let map = source_map("#set page(margin: 2pt)\n#set par(justify: true)\n#lorem(900)");
        let (min, max) = map.content_column().unwrap();
        assert!(min < 3.0 && max > A4_WIDTH - 4.0);
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
