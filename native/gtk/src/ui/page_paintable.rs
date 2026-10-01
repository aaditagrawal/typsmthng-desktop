//! Rasterize visible Typst/SVG pages at their allocated display resolution. Rendering
//! runs on two shared workers; GTK keeps the last image while a resize finishes.
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use gtk::{gdk, gdk_pixbuf, prelude::*, subclass::prelude::*};

struct RenderedPage {
    size: (i32, i32),
    texture: gdk::Texture,
}

struct Pixels {
    width: i32,
    height: i32,
    stride: usize,
    format: gdk::MemoryFormat,
    bytes: glib::Bytes,
}

#[derive(Clone)]
enum PageSource {
    Svg(Arc<[u8]>),
    Typst(typst_layout::Page),
}

struct RenderJob {
    page: u64,
    generation: u64,
    latest: Arc<AtomicU64>,
    source: PageSource,
    size: (i32, i32),
    complete: Box<dyn FnOnce(Option<Pixels>) + Send>,
}

struct Workers {
    queue: Mutex<VecDeque<RenderJob>>,
    ready: Condvar,
}

impl Workers {
    fn shared() -> &'static Arc<Self> {
        static WORKERS: OnceLock<Arc<Workers>> = OnceLock::new();
        WORKERS.get_or_init(|| {
            let workers = Arc::new(Self {
                queue: Mutex::new(VecDeque::new()),
                ready: Condvar::new(),
            });
            for index in 0..2 {
                let workers = workers.clone();
                std::thread::Builder::new()
                    .name(format!("preview-raster-{index}"))
                    .spawn(move || loop {
                        let job = {
                            let mut queue = workers.queue.lock().unwrap();
                            while queue.is_empty() {
                                queue = workers.ready.wait(queue).unwrap();
                            }
                            queue.pop_front().unwrap()
                        };
                        if job.latest.load(Ordering::Relaxed) != job.generation {
                            continue;
                        }
                        let pixels = rasterize(&job.source, job.size);
                        if job.latest.load(Ordering::Relaxed) == job.generation {
                            (job.complete)(pixels);
                        }
                    })
                    .expect("start preview raster worker");
            }
            workers
        })
    }

    /// Replace queued requests for the same page and discard cancelled work.
    /// A full queue retries on GTK's next short timeout instead of blocking it.
    fn submit(&self, job: RenderJob) -> bool {
        let mut queue = self.queue.lock().unwrap();
        queue.retain(|queued| {
            queued.page != job.page && queued.latest.load(Ordering::Relaxed) == queued.generation
        });
        if queue.len() >= 64 {
            return false;
        }
        queue.push_back(job);
        self.ready.notify_one();
        true
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PagePaintable {
        pub(super) source: OnceCell<PageSource>,
        pub(super) original: RefCell<Option<gdk::Paintable>>,
        pub(super) dimensions: Cell<(i32, i32)>,
        pub(super) aspect: Cell<f64>,
        pub(super) path: OnceCell<PathBuf>,
        pub(super) owner: OnceCell<glib::WeakRef<gtk::Picture>>,
        pub(super) owner_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub(super) scroll: RefCell<Option<glib::WeakRef<gtk::ScrolledWindow>>>,
        pub(super) scroll_handlers:
            RefCell<Vec<(glib::WeakRef<gtk::Adjustment>, glib::SignalHandlerId)>>,
        pub(super) scale: Cell<f64>,
        pub(super) surface: RefCell<Option<(glib::WeakRef<gdk::Surface>, glib::SignalHandlerId)>>,
        pub(super) active: Cell<bool>,
        pub(super) disposed: Cell<bool>,
        pub(super) id: Cell<u64>,
        pub(super) latest: Arc<AtomicU64>,
        pub(super) pending: Cell<Option<(u64, (i32, i32))>>,
        pub(super) retry: Cell<bool>,
        pub(super) failed: Cell<Option<(i32, i32)>>,
        pub(super) rendered: RefCell<Option<RenderedPage>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PagePaintable {
        const NAME: &'static str = "TypsmthngPagePaintable";
        type Type = super::PagePaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for PagePaintable {
        fn constructed(&self) {
            self.parent_constructed();
            static NEXT_PAGE: AtomicU64 = AtomicU64::new(1);
            self.id.set(NEXT_PAGE.fetch_add(1, Ordering::Relaxed));
            self.active.set(true);
        }

        fn dispose(&self) {
            self.disposed.set(true);
            self.cancel();
            self.disconnect_scroll();
            self.disconnect_surface();
            if let Some(owner) = self.owner.get().and_then(|owner| owner.upgrade()) {
                for handler in self.owner_handlers.take() {
                    owner.disconnect(handler);
                }
            }
            self.original.take();
            self.rendered.take();
        }
    }

    impl PagePaintable {
        pub(super) fn cancel(&self) {
            self.latest.fetch_add(1, Ordering::Relaxed);
            self.pending.set(None);
        }

        fn disconnect_scroll(&self) {
            for (adjustment, handler) in self.scroll_handlers.take() {
                if let Some(adjustment) = adjustment.upgrade() {
                    adjustment.disconnect(handler);
                }
            }
            self.scroll.take();
        }

        fn disconnect_surface(&self) {
            if let Some((surface, handler)) = self.surface.take() {
                if let Some(surface) = surface.upgrade() {
                    surface.disconnect(handler);
                }
            }
        }

        fn watch_surface(&self, owner: &gtk::Picture) {
            let surface = owner.native().and_then(|native| native.surface());
            let previous = self
                .surface
                .borrow()
                .as_ref()
                .and_then(|(weak, _)| weak.upgrade());
            if previous == surface {
                return;
            }
            self.disconnect_surface();
            if let Some(surface) = surface {
                self.scale.set(surface.scale());
                self.cancel();
                let handler = surface.connect_scale_notify({
                    let weak = self.obj().downgrade();
                    move |surface| {
                        if let Some(page) = weak.upgrade() {
                            page.imp().scale.set(surface.scale());
                            page.imp().cancel();
                            page.invalidate_contents();
                        }
                    }
                });
                self.surface.replace(Some((surface.downgrade(), handler)));
            }
        }

        fn watch_scroll(&self, owner: &gtk::Picture) {
            let mut parent = owner.parent();
            let mut scroll = None;
            while let Some(widget) = parent {
                if let Ok(found) = widget.clone().downcast::<gtk::ScrolledWindow>() {
                    scroll = Some(found);
                    break;
                }
                parent = widget.parent();
            }
            let previous = self
                .scroll
                .borrow()
                .as_ref()
                .and_then(|weak| weak.upgrade());
            if previous == scroll {
                return;
            }
            self.disconnect_scroll();
            if let Some(scroll) = scroll {
                self.scroll.replace(Some(scroll.downgrade()));
                for adjustment in [scroll.hadjustment(), scroll.vadjustment()] {
                    let handler = adjustment.connect_value_changed({
                        let weak = self.obj().downgrade();
                        move |_| {
                            if let Some(page) = weak.upgrade() {
                                page.imp().refresh_visibility();
                            }
                        }
                    });
                    self.scroll_handlers
                        .borrow_mut()
                        .push((adjustment.downgrade(), handler));
                    let handler = adjustment.connect_changed({
                        let weak = self.obj().downgrade();
                        move |_| {
                            if let Some(page) = weak.upgrade() {
                                page.imp().refresh_visibility();
                            }
                        }
                    });
                    self.scroll_handlers
                        .borrow_mut()
                        .push((adjustment.downgrade(), handler));
                }
            }
        }

        fn visible(&self, owner: &gtk::Picture) -> bool {
            if !owner.is_mapped() && owner.parent().is_some() {
                return false;
            }
            if owner.is_mapped() && (owner.width() == 0 || owner.height() == 0) {
                // Reparented widgets map before GTK has allocated them again.
                // Their zero-sized transient bounds are not evidence that a
                // previously visible page has left its viewport.
                return self.active.get();
            }
            let scroll = self
                .scroll
                .borrow()
                .as_ref()
                .and_then(|weak| weak.upgrade());
            let Some(scroll) = scroll else { return true };
            // GtkViewport updates its child transform during allocation, after
            // value-changed fires. Compare stable content coordinates with the
            // adjustment values so scrolling invalidates newly visible pages
            // immediately, including children with cached GTK render nodes.
            if let Some(viewport) = scroll
                .child()
                .and_then(|child| child.downcast::<gtk::Viewport>().ok())
            {
                if let Some(content) = viewport.child() {
                    let Some(bounds) = owner.compute_bounds(&content) else {
                        return false;
                    };
                    let rtl = content.direction() == gtk::TextDirection::Rtl;
                    let left = if rtl {
                        content.margin_end()
                    } else {
                        content.margin_start()
                    };
                    let right = if rtl {
                        content.margin_start()
                    } else {
                        content.margin_end()
                    };
                    let aligned = |align, available: f64, size, before, after, reversed: bool| {
                        let slack = (available - f64::from(size + before + after)).max(0.0);
                        f64::from(before)
                            + match align {
                                gtk::Align::Center => (slack / 2.0).floor(),
                                gtk::Align::End if !reversed => slack,
                                gtk::Align::Start if reversed => slack,
                                _ => 0.0,
                            }
                    };
                    let x = scroll.hadjustment().value()
                        - aligned(
                            content.halign(),
                            scroll.hadjustment().upper(),
                            content.width(),
                            left,
                            right,
                            rtl,
                        );
                    let y = scroll.vadjustment().value()
                        - aligned(
                            content.valign(),
                            scroll.vadjustment().upper(),
                            content.height(),
                            content.margin_top(),
                            content.margin_bottom(),
                            false,
                        );
                    // A small approach margin prefetches the next page and avoids
                    // discarding images on tiny reversals while scrolling.
                    let margin = 64.0;
                    return f64::from(bounds.x()) < x + f64::from(viewport.width()) + margin
                        && f64::from(bounds.y()) < y + f64::from(viewport.height()) + margin
                        && f64::from(bounds.x() + bounds.width()) > x - margin
                        && f64::from(bounds.y() + bounds.height()) > y - margin;
                }
            }
            let Some(bounds) = owner.compute_bounds(&scroll) else {
                return false;
            };
            bounds.x() < scroll.width() as f32
                && bounds.y() < scroll.height() as f32
                && bounds.x() + bounds.width() > 0.0
                && bounds.y() + bounds.height() > 0.0
        }

        pub(super) fn refresh_visibility(&self) -> bool {
            let Some(owner) = self.owner.get().and_then(|weak| weak.upgrade()) else {
                return false;
            };
            self.watch_scroll(&owner);
            self.watch_surface(&owner);
            let active = self.visible(&owner);
            if self.active.replace(active) != active {
                if !active {
                    self.cancel();
                    self.rendered.take();
                    self.original.take();
                }
                self.obj().invalidate_contents();
            }
            active
        }

        fn request(&self, size: (i32, i32)) {
            if self.failed.get() == Some(size)
                || self
                    .rendered
                    .borrow()
                    .as_ref()
                    .is_some_and(|page| page.size == size)
            {
                if self.pending.get().is_some() {
                    self.cancel();
                }
                return;
            }
            if self
                .pending
                .get()
                .is_some_and(|(_, pending)| pending == size)
            {
                return;
            }
            let Some(source) = self.source.get() else {
                return;
            };
            let generation = self.latest.fetch_add(1, Ordering::Relaxed) + 1;
            self.pending.set(Some((generation, size)));
            let weak = glib::SendWeakRef::from(self.obj().downgrade());
            let queued = Workers::shared().submit(RenderJob {
                page: self.id.get(),
                generation,
                latest: self.latest.clone(),
                source: source.clone(),
                size,
                complete: Box::new(move |pixels| {
                    glib::idle_add_once(move || {
                        let Some(page) = weak.upgrade() else { return };
                        let imp = page.imp();
                        if imp.disposed.get() || imp.latest.load(Ordering::Relaxed) != generation {
                            return;
                        }
                        imp.pending.set(None);
                        if !imp.refresh_visibility() {
                            return;
                        }
                        if let Some(pixels) = pixels {
                            let texture = gdk::MemoryTexture::new(
                                pixels.width,
                                pixels.height,
                                pixels.format,
                                &pixels.bytes,
                                pixels.stride,
                            );
                            imp.rendered.replace(Some(RenderedPage {
                                size,
                                texture: texture.upcast(),
                            }));
                            // The new texture replaces the initial decode, keeping
                            // intrinsic geometry without retaining a second image.
                            imp.original.take();
                            imp.failed.set(None);
                        } else {
                            imp.failed.set(Some(size));
                        }
                        page.invalidate_contents();
                    });
                }),
            });
            if !queued {
                self.pending.set(None);
                if !self.retry.replace(true) {
                    let weak = self.obj().downgrade();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(16), move || {
                        if let Some(page) = weak.upgrade() {
                            page.imp().retry.set(false);
                            if !page.imp().disposed.get() {
                                page.invalidate_contents();
                            }
                        }
                    });
                }
            }
        }
    }

    impl PaintableImpl for PagePaintable {
        fn current_image(&self) -> gdk::Paintable {
            if let Some(page) = self.rendered.borrow().as_ref() {
                page.texture.clone().upcast()
            } else if let Some(original) = self.original.borrow().as_ref() {
                original.current_image()
            } else {
                let (width, height) = self.dimensions.get();
                gdk::Paintable::new_empty(width, height)
            }
        }

        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }

        fn intrinsic_width(&self) -> i32 {
            self.dimensions.get().0
        }

        fn intrinsic_height(&self) -> i32 {
            self.dimensions.get().1
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            self.aspect.get()
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if self.disposed.get() || !self.refresh_visibility() {
                return;
            }
            let Some(size) = pixel_size(width, height, self.scale.get()) else {
                return;
            };
            self.request(size);
            if let Some(page) = self.rendered.borrow().as_ref() {
                if let Some(snapshot) = snapshot.downcast_ref::<gtk::Snapshot>() {
                    // Draw current pixels at their exact device size rather
                    // than stretching them to fractional fitted page bounds.
                    // Use a regular TextureNode: Cairo's TextureScaleNode
                    // fallback can first flatten at logical resolution.
                    let scale = self.scale.get().max(1.0);
                    if page.size == size
                        && (f64::from(page.texture.width()) - width * scale).abs() <= 1.0
                        && (f64::from(page.texture.height()) - height * scale).abs() <= 1.0
                    {
                        snapshot.push_clip(&gtk::graphene::Rect::new(
                            0.0,
                            0.0,
                            width as f32,
                            height as f32,
                        ));
                        snapshot.append_texture(
                            &page.texture,
                            &gtk::graphene::Rect::new(
                                0.0,
                                0.0,
                                (f64::from(page.texture.width()) / scale) as f32,
                                (f64::from(page.texture.height()) / scale) as f32,
                            ),
                        );
                        snapshot.pop();
                    } else {
                        page.texture.snapshot(snapshot, width, height);
                    }
                } else {
                    page.texture.snapshot(snapshot, width, height);
                }
            } else if let Some(original) = self.original.borrow().as_ref() {
                original.snapshot(snapshot, width, height);
            }
        }
    }
}

glib::wrapper! {
    pub struct PagePaintable(ObjectSubclass<imp::PagePaintable>)
        @implements gdk::Paintable;
}

/// Retain source bytes so unchanged pages still resize after compiler caches
/// disappear. Non-SVG files retain GtkPicture's ordinary image loader.
pub fn load(picture: &gtk::Picture, path: &Path) {
    load_page(picture, path, None);
}

/// Compiled previews use Typst's glyph rasterizer; SVG/image imports keep the
/// platform loader. The SVG still supplies identity, geometry, and hyperlinks.
pub fn load_page(picture: &gtk::Picture, path: &Path, page: Option<typst_layout::Page>) {
    if is_file(picture, path) {
        return;
    }
    let svg = std::fs::read(path).ok();
    let geometry = svg
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(svg_geometry);
    let Some((dimensions, aspect)) = geometry else {
        // Unknown SVG geometry and other formats retain GTK's image support.
        picture.set_filename(Some(path));
        return;
    };
    let original = picture
        .paintable()
        .map(|paintable| paintable.current_image());
    let paintable: PagePaintable = glib::Object::new();
    let imp = paintable.imp();
    imp.dimensions.set(dimensions);
    imp.aspect.set(aspect);
    imp.original.replace(original);
    let source = page.map_or_else(
        || PageSource::Svg(Arc::from(svg.unwrap())),
        PageSource::Typst,
    );
    assert!(imp.source.set(source).is_ok(), "new page paintable");
    imp.path.set(path.to_path_buf()).expect("new source path");
    imp.owner.set(picture.downgrade()).expect("new owner");
    imp.scale.set(f64::from(picture.scale_factor()));
    let handler = picture.connect_scale_factor_notify({
        let weak = paintable.downgrade();
        move |picture| {
            if let Some(page) = weak.upgrade() {
                page.imp().scale.set(
                    picture
                        .native()
                        .and_then(|native| native.surface())
                        .map_or(f64::from(picture.scale_factor()), |surface| surface.scale()),
                );
                page.imp().cancel();
                page.invalidate_contents();
            }
        }
    });
    imp.owner_handlers.borrow_mut().push(handler);
    let handler = picture.connect_map({
        let weak = paintable.downgrade();
        move |_| {
            if let Some(page) = weak.upgrade() {
                page.invalidate_contents();
            }
        }
    });
    imp.owner_handlers.borrow_mut().push(handler);
    let handler = picture.connect_unmap({
        let weak = paintable.downgrade();
        move |_| {
            // Partial recompiles synchronously remove and reappend unchanged
            // page widgets. Preserve their image across that brief unmap.
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(page) = weak.upgrade() {
                    page.imp().refresh_visibility();
                }
            });
        }
    });
    imp.owner_handlers.borrow_mut().push(handler);
    picture.set_paintable(Some(&paintable));
}

/// Typst emits numeric root dimensions in points. Parsing only the root header
/// avoids decoding every page on the GTK thread just to obtain its geometry.
fn svg_geometry(svg: &str) -> Option<((i32, i32), f64)> {
    static DIMENSIONS: OnceLock<regex::Regex> = OnceLock::new();
    let header = svg.split_once("<svg")?.1.split_once('>')?.0;
    let dimensions = DIMENSIONS.get_or_init(|| {
        regex::Regex::new(r#"(?:^|\s)(width|height)\s*=\s*["']([^"']+)["']"#).unwrap()
    });
    let mut width = None;
    let mut height = None;
    for attribute in dimensions.captures_iter(header) {
        let length = attribute[2].trim();
        let (number, unit) = if let Some(value) = length.strip_suffix("pt") {
            (value, 96.0 / 72.0)
        } else if let Some(value) = length.strip_suffix("px") {
            (value, 1.0)
        } else {
            (length, 1.0)
        };
        let value = number.trim().parse::<f64>().ok()? * unit;
        if !value.is_finite() || value <= 0.0 {
            return None;
        }
        if &attribute[1] == "width" {
            width = Some(value);
        } else {
            height = Some(value);
        }
    }
    let (width, height) = (width?, height?);
    let aspect = width / height;
    if !aspect.is_finite() || aspect <= 0.0 {
        return None;
    }
    Some((
        (
            width.round().clamp(1.0, f64::from(i32::MAX)) as i32,
            height.round().clamp(1.0, f64::from(i32::MAX)) as i32,
        ),
        aspect,
    ))
}

pub fn is_file(picture: &gtk::Picture, path: &Path) -> bool {
    picture.paintable().is_some_and(|paintable| {
        if let Some(page) = paintable.downcast_ref::<PagePaintable>() {
            page.imp().path.get().is_some_and(|loaded| loaded == path)
        } else {
            picture.file().and_then(|file| file.path()).as_deref() == Some(path)
        }
    })
}

/// A changed page may briefly show an old static image while workers decode it.
/// Source clicks must wait until the image belongs to the current SVG bytes.
pub fn is_current(picture: &gtk::Picture) -> bool {
    picture.paintable().is_some_and(|paintable| {
        paintable
            .downcast_ref::<PagePaintable>()
            .is_none_or(|page| page.imp().rendered.borrow().is_some())
    })
}

fn pixel_size(width: f64, height: f64, scale: f64) -> Option<(i32, i32)> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let width = width * scale.max(1.0);
    let height = height * scale.max(1.0);
    // Bound each image to 32 megapixels and GPU-compatible dimensions.
    let reduction = (32_000_000.0 / (width * height))
        .sqrt()
        .min(16_384.0 / width)
        .min(16_384.0 / height)
        .min(1.0);
    Some((
        (width * reduction).ceil().max(1.0) as i32,
        (height * reduction).ceil().max(1.0) as i32,
    ))
}

fn rasterize(source: &PageSource, (width, height): (i32, i32)) -> Option<Pixels> {
    if let PageSource::Typst(page) = source {
        // Keep the page transform uniform so text uses Typst's subpixel glyph
        // rasterizer, rather than its general path fallback. Rounding can make
        // the resulting height differ by one pixel from the requested bounds.
        let options = typst_render::RenderOptions {
            pixel_per_pt: (f64::from(width) / page.frame.width().to_pt())
                .min(f64::from(height) / page.frame.height().to_pt())
                .into(),
            render_bleed: false,
        };
        let pixmap = typst_render::render(page, &options);
        return Some(Pixels {
            width: pixmap.width() as i32,
            height: pixmap.height() as i32,
            stride: pixmap.width() as usize * 4,
            format: gdk::MemoryFormat::R8g8b8a8Premultiplied,
            bytes: glib::Bytes::from_owned(pixmap.take()),
        });
    }
    let PageSource::Svg(svg) = source else {
        unreachable!()
    };
    let loader = gdk_pixbuf::PixbufLoader::with_type("svg").ok()?;
    loader.set_size(width, height);
    let written = loader.write(svg);
    let closed = loader.close();
    written.ok()?;
    closed.ok()?;
    let pixbuf = loader.pixbuf()?;
    Some(Pixels {
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: usize::try_from(pixbuf.rowstride()).ok()?,
        format: if pixbuf.has_alpha() {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        },
        bytes: pixbuf.read_pixel_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn drive_until(done: impl Fn() -> bool) {
        let context = glib::MainContext::default();
        let until = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(done(), "asynchronous page render did not finish");
    }

    fn drive(duration: Duration) {
        let until = Instant::now() + duration;
        drive_until(|| Instant::now() >= until);
    }

    fn paintable(picture: &gtk::Picture) -> PagePaintable {
        picture.paintable().unwrap().downcast().unwrap()
    }

    fn rendered_size(page: &PagePaintable) -> Option<(i32, i32)> {
        page.imp().rendered.borrow().as_ref().map(|page| page.size)
    }

    fn compiled(source: &str) -> typsmthng_gtk::backend::preview::Preview {
        use typsmthng_gtk::backend::{preview::PreviewCompiler, CompileOptions, Project};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.typ"), source).unwrap();
        PreviewCompiler::default()
            .compile(
                &Project::open(dir.path()).unwrap(),
                "main.typ",
                &CompileOptions {
                    ignore_system_fonts: true,
                    ..Default::default()
                },
            )
            .unwrap()
            .artifact
            .unwrap()
    }

    #[test]
    fn native_page_pixels_preserve_transparency_and_display_bounds() {
        let preview = compiled(
            "#set page(width: 120pt, height: 80pt, margin: 0pt, fill: none)\n\
             #rect(width: 30pt, height: 30pt, fill: rgb(255, 0, 0, 50%), stroke: none)",
        );
        let source = PageSource::Typst(preview.source_map.page(0).unwrap());
        for scale in [1, 2, 3] {
            let pixels = rasterize(&source, (240 * scale, 160 * scale)).unwrap();
            assert_eq!((pixels.width, pixels.height), (240 * scale, 160 * scale));
            assert_eq!(pixels.format, gdk::MemoryFormat::R8g8b8a8Premultiplied);
            let pixel = |x: usize, y: usize| {
                &pixels.bytes[y * pixels.stride + x * 4..y * pixels.stride + x * 4 + 4]
            };
            assert_eq!(pixel(10, 10), &[128, 0, 0, 128]);
            assert_eq!(
                pixel(100 * scale as usize, 50 * scale as usize),
                &[0, 0, 0, 0]
            );
        }
        let pixels = rasterize(&source, (241, 160)).unwrap();
        assert!(pixels.width <= 241 && pixels.height <= 160);
    }

    #[test]
    fn raster_sizes_are_bounded_at_large_zoom() {
        for (width, height, scale) in [
            (800.0, 1131.0, 2.0),
            (40000.0, 60000.0, 4.0),
            (1e12, 1.0, 1.0),
        ] {
            let (w, h) = pixel_size(width, height, scale).unwrap();
            assert!(w <= 16384 && h <= 16384);
            // Ceil can add at most one extra row and column to the area limit.
            assert!(i64::from(w) * i64::from(h) <= 32_000_000 + i64::from(w + h));
        }
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(pixel_size(invalid, 100.0, 1.0).is_none());
        }
        assert_eq!(pixel_size(480.0, 320.0, 1.25), Some((600, 400)));
        assert_eq!(pixel_size(480.0, 320.0, 1.5), Some((720, 480)));
    }

    #[test]
    #[ignore = "manual raster benchmark; run in release with --nocapture"]
    fn benchmark_preview_rasterization() {
        let mut text = String::from(
            "#set page(paper: \"a4\", margin: 24mm)\n#set text(size: 11pt)\n= Text and mathematics 0\n",
        );
        for _ in 0..30 {
            text.push_str("Subpixel typography, ligatures: affine office. $ integral_0^1 x^2 dif x = frac(1,3), sqrt(x^2+y^2) $\n\n");
        }
        let preview = compiled(&text);
        let sources = [
            (
                "svg",
                PageSource::Svg(Arc::from(preview.pages[0].svg.as_bytes())),
            ),
            (
                "native",
                PageSource::Typst(preview.source_map.page(0).unwrap()),
            ),
        ];
        for scale in [1, 2] {
            let size = pixel_size(794.0, 1123.0, f64::from(scale)).unwrap();
            for (name, source) in &sources {
                let cold = Instant::now();
                let pixels = rasterize(source, size).unwrap();
                let cold = cold.elapsed();
                let mut times = Vec::new();
                for _ in 0..11 {
                    let start = Instant::now();
                    std::hint::black_box(rasterize(source, size).unwrap());
                    times.push(start.elapsed());
                }
                times.sort();
                // Measure changed content too, rather than crediting redraws
                // of an identical page as live-edit performance.
                use typsmthng_gtk::backend::{preview::PreviewCompiler, CompileOptions, Project};
                let dir = tempfile::tempdir().unwrap();
                let project = Project::open(dir.path()).unwrap();
                let compiler = PreviewCompiler::default();
                let mut edits = Vec::new();
                for revision in 1..=11 {
                    std::fs::write(
                        dir.path().join("main.typ"),
                        text.replace("mathematics 0", &format!("mathematics {revision}")),
                    )
                    .unwrap();
                    let changed = compiler
                        .compile(
                            &project,
                            "main.typ",
                            &CompileOptions {
                                ignore_system_fonts: true,
                                ..Default::default()
                            },
                        )
                        .unwrap()
                        .artifact
                        .unwrap();
                    let source = if *name == "native" {
                        PageSource::Typst(changed.source_map.page(0).unwrap())
                    } else {
                        PageSource::Svg(Arc::from(changed.pages[0].svg.as_bytes()))
                    };
                    let start = Instant::now();
                    std::hint::black_box(rasterize(&source, size).unwrap());
                    edits.push(start.elapsed());
                }
                edits.sort();
                println!(
                    "{name} scale={scale}: cold={cold:?}, redraw median={:?}, max={:?}, edit median={:?}, edit max={:?}, bytes={}",
                    times[5],
                    times[10],
                    edits[5],
                    edits[10],
                    pixels.bytes.len()
                );
                if let Some(dir) = std::env::var_os("TYPSMTHNG_RASTER_ARTIFACT_DIR") {
                    let dir = PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    // This fixture has an opaque white page, so native
                    // premultiplied and straight RGBA bytes are identical.
                    gdk_pixbuf::Pixbuf::from_bytes(
                        &pixels.bytes,
                        gdk_pixbuf::Colorspace::Rgb,
                        pixels.format != gdk::MemoryFormat::R8g8b8,
                        8,
                        pixels.width,
                        pixels.height,
                        pixels.stride as i32,
                    )
                    .savev(dir.join(format!("{name}-{scale}.png")), "png", &[])
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn svg_headers_preserve_exact_page_geometry() {
        let (dimensions, aspect) = svg_geometry(
            r#"<svg viewBox="0 0 595.275590551 841.88976378" width="595.275590551pt" height="841.88976378pt"></svg>"#,
        ).unwrap();
        assert_eq!(dimensions, (794, 1123));
        assert!((aspect - 595.275590551 / 841.88976378).abs() < 1e-14);
        assert_eq!(
            svg_geometry("<svg width='120px' height='80'></svg>"),
            Some(((120, 80), 1.5))
        );
        assert!(svg_geometry("<svg width='100%' height='100%'></svg>").is_none());
        assert!(svg_geometry("<svg width='-1' height='80'></svg>").is_none());
        assert!(svg_geometry("<svg width='NaN' height='80'></svg>").is_none());
        assert_eq!(
            svg_geometry("<svg width='120' height='80' stroke-width='2' data-width='1'></svg>"),
            Some(((120, 80), 1.5))
        );
    }

    #[test]
    #[ignore = "requires a GTK display; run under Xvfb at GDK_SCALE=1 and 2"]
    fn native_svg_pages_follow_zoom_and_display_scale() {
        gtk::init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.svg");
        std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="white"/><path d="M0 0L120 80" stroke="black"/></svg>"#).unwrap();
        let picture = gtk::Picture::new();
        picture.set_size_request(480, 320);
        load(&picture, &path);
        let window = gtk::Window::new();
        window.set_child(Some(&picture));
        window.present();
        let page = paintable(&picture);
        drive_until(|| rendered_size(&page).is_some());
        let scale = picture.scale_factor();
        assert_eq!(page.imp().scale.get(), f64::from(scale));
        assert_eq!(page.intrinsic_width(), 120);
        assert_eq!(page.intrinsic_height(), 80);
        assert_eq!(rendered_size(&page), Some((480 * scale, 320 * scale)));
        assert!(page.imp().original.borrow().is_none());
        let first = page
            .imp()
            .rendered
            .borrow()
            .as_ref()
            .unwrap()
            .texture
            .clone();
        let generation = page.imp().latest.load(Ordering::Relaxed);
        page.snapshot(&gtk::Snapshot::new(), 480.0, 320.0);
        assert_eq!(page.imp().latest.load(Ordering::Relaxed), generation);
        assert_eq!(
            page.imp().rendered.borrow().as_ref().unwrap().texture,
            first
        );
        let snapshot = gtk::Snapshot::new();
        page.snapshot(&snapshot, 480.0, 320.0);
        let node = snapshot.to_node().unwrap();
        assert!(node.downcast_ref::<gtk::gsk::TextureNode>().is_some());
        assert_eq!(node.bounds().width(), 480.0);
        // Zooming out to the cached size must cancel a pending larger request.
        let snapshot = gtk::Snapshot::new();
        page.snapshot(&snapshot, 720.0, 480.0);
        assert_eq!(snapshot.to_node().unwrap().bounds().width(), 720.0);
        assert!(page.imp().pending.get().is_some());
        page.snapshot(&gtk::Snapshot::new(), 480.0, 320.0);
        assert!(page.imp().pending.get().is_none());
        drive(Duration::from_millis(40));
        assert_eq!(
            page.imp().rendered.borrow().as_ref().unwrap().texture,
            first
        );
        let immutable = page.current_image();
        assert!(immutable
            .flags()
            .contains(gdk::PaintableFlags::STATIC_CONTENTS));
        assert!(immutable.flags().contains(gdk::PaintableFlags::STATIC_SIZE));
        load(&picture, &path);
        assert_eq!(
            picture.paintable().unwrap(),
            page.clone().upcast::<gdk::Paintable>()
        );
        // Source paths disappear after recompilation. Zoom still uses retained bytes.
        std::fs::remove_file(&path).unwrap();
        picture.set_size_request(960, 640);
        drive_until(|| rendered_size(&page) == Some((960 * scale, 640 * scale)));
        assert_ne!(
            page.imp().rendered.borrow().as_ref().unwrap().texture,
            first
        );
        page.imp().scale.set(f64::from(scale + 1));
        page.imp().cancel();
        page.invalidate_contents();
        drive_until(|| rendered_size(&page) == Some((960 * (scale + 1), 640 * (scale + 1))));
        window.destroy();

        // A real scroll viewport must cull its mapped, offscreen GtkBox children.
        std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="white"/></svg>"#).unwrap();
        let window = gtk::Window::new();
        let scroll = gtk::ScrolledWindow::new();
        let pages = gtk::Box::new(gtk::Orientation::Vertical, 8);
        pages.set_halign(gtk::Align::Center);
        pages.set_margin_top(24);
        pages.set_margin_bottom(48);
        pages.set_margin_start(24);
        pages.set_margin_end(24);
        scroll.set_child(Some(&pages));
        window.set_child(Some(&scroll));
        window.set_default_size(500, 400);
        let pictures: Vec<_> = (0..20)
            .map(|_| {
                let picture = gtk::Picture::new();
                picture.set_size_request(480, 320);
                load(&picture, &path);
                pages.append(&picture);
                picture
            })
            .collect();
        window.present();
        drive_until(|| rendered_size(&paintable(&pictures[0])).is_some());
        drive(Duration::from_millis(40));
        let cached = || {
            pictures
                .iter()
                .filter(|picture| rendered_size(&paintable(picture)).is_some())
                .count()
        };
        assert!(
            cached() <= 2,
            "offscreen pages must not have high-resolution textures"
        );
        for picture in pictures.iter().skip(2) {
            let page = paintable(picture);
            assert!(page.imp().pending.get().is_none());
            assert!(page.imp().original.borrow().is_none());
        }
        let first = paintable(&pictures[0])
            .imp()
            .rendered
            .borrow()
            .as_ref()
            .unwrap()
            .texture
            .clone();
        // Partial compilation reuses unchanged widgets through remove/reappend.
        for picture in &pictures {
            pages.remove(picture);
        }
        for picture in &pictures {
            pages.append(picture);
        }
        drive(Duration::from_millis(40));
        assert_eq!(
            paintable(&pictures[0])
                .imp()
                .rendered
                .borrow()
                .as_ref()
                .unwrap()
                .texture,
            first
        );
        let boundary = pictures[0].height() + pages.margin_top() - 16;
        scroll.vadjustment().set_value(f64::from(boundary));
        drive(Duration::from_millis(40));
        assert!(rendered_size(&paintable(&pictures[0])).is_some());
        // Check the approach margin's outer boundary too: omitting the content
        // margin would evict this page 24 pixels before it should be discarded.
        scroll.vadjustment().set_value(f64::from(boundary + 64));
        drive(Duration::from_millis(40));
        assert!(rendered_size(&paintable(&pictures[0])).is_some());
        let offset = pictures[10].compute_bounds(&pages).unwrap().y() + pages.margin_top() as f32;
        scroll.vadjustment().set_value(f64::from(offset));
        drive_until(|| rendered_size(&paintable(&pictures[10])).is_some());
        assert!(rendered_size(&paintable(&pictures[0])).is_none());
        assert!(
            cached() <= 3,
            "scrolling must evict pages that leave the viewport"
        );
        window.destroy();

        // Existing static content remains visible while a changed page decodes.
        let seeded = gtk::Picture::for_paintable(&first);
        load(&seeded, &path);
        assert!(!is_current(&seeded));
        assert_eq!(
            paintable(&seeded).imp().original.borrow().as_ref().unwrap(),
            &first.clone().upcast::<gdk::Paintable>()
        );
        paintable(&seeded).snapshot(&gtk::Snapshot::new(), 240.0, 160.0);
        drive_until(|| rendered_size(&paintable(&seeded)).is_some());
        assert!(paintable(&seeded).imp().original.borrow().is_none());
        assert!(is_current(&seeded));

        // Worker completions hold weak references and do not retain closed pages.
        let temporary = gtk::Picture::new();
        load(&temporary, &path);
        let temporary_page = paintable(&temporary);
        let weak = temporary_page.downgrade();
        temporary_page.snapshot(&gtk::Snapshot::new(), 1200.0, 800.0);
        drop(temporary_page);
        drop(temporary);
        drive(Duration::from_millis(40));
        assert!(weak.upgrade().is_none());

        // The direct renderer uses the same asynchronous lifecycle. Fonts and
        // frames survive the compiler snapshot and its temporary SVG cache.
        let preview = compiled("#set page(width: 120pt, height: 80pt, margin: 8pt)\nSmall text $ frac(a,b) + sqrt(2) $");
        let native_path = dir.path().join("native.svg");
        std::fs::write(&native_path, &preview.pages[0].svg).unwrap();
        let picture = gtk::Picture::new();
        picture.set_size_request(480, 320);
        load_page(&picture, &native_path, preview.source_map.page(0));
        let page = paintable(&picture);
        assert!(matches!(
            page.imp().source.get(),
            Some(PageSource::Typst(_))
        ));
        drop(preview);
        std::fs::remove_file(native_path).unwrap();
        let window = gtk::Window::new();
        window.set_child(Some(&picture));
        window.present();
        drive_until(|| rendered_size(&page).is_some());
        assert_eq!(rendered_size(&page), Some((480 * scale, 320 * scale)));
        assert!(is_current(&picture));
        window.destroy();

        // PNG and unsupported SVG units retain GTK's loader and file identity.
        let png = dir.path().join("image.png");
        let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 4, 3).unwrap();
        pixels.fill(0xff000080);
        pixels.savev(&png, "png", &[]).unwrap();
        let ordinary = gtk::Picture::new();
        load(&ordinary, &png);
        assert!(ordinary
            .paintable()
            .unwrap()
            .downcast_ref::<PagePaintable>()
            .is_none());
        assert!(is_file(&ordinary, &png));
        assert!(is_current(&ordinary));
        let unusual = dir.path().join("units.svg");
        std::fs::write(&unusual, r#"<svg xmlns="http://www.w3.org/2000/svg" width="1in" height="1in"><rect width="100%" height="100%" fill="red"/></svg>"#).unwrap();
        load(&ordinary, &unusual);
        assert!(ordinary
            .paintable()
            .unwrap()
            .downcast_ref::<PagePaintable>()
            .is_none());
        assert!(is_file(&ordinary, &unusual));
    }
}
