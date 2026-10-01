use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, graphene, prelude::*};

use super::{model::UiSettings, page_paintable};

#[derive(Clone)]
pub(super) struct CenteredScrolling {
    editor: glib::WeakRef<sourceview5::View>,
    settings: std::rc::Weak<RefCell<UiSettings>>,
    restoring: Rc<Cell<Option<(i32, f64)>>>,
    centering: Rc<Cell<bool>>,
    generation: Rc<Cell<u64>>,
}

impl CenteredScrolling {
    pub fn new(editor: &sourceview5::View, settings: &Rc<RefCell<UiSettings>>) -> Self {
        let controls = Self {
            editor: editor.downgrade(),
            settings: Rc::downgrade(settings),
            restoring: Rc::new(Cell::new(None)),
            centering: Rc::new(Cell::new(false)),
            generation: Rc::new(Cell::new(0)),
        };
        editor.buffer().connect_mark_set({
            let controls = controls.clone();
            move |_, _, mark| {
                if mark.name().as_deref() == Some("insert") {
                    controls.center();
                }
            }
        });
        editor.buffer().connect_changed({
            let controls = controls.clone();
            move |_| controls.center()
        });
        editor.connect_unmap({
            let controls = controls.clone();
            move |_| controls.cancel_centering()
        });
        if let Some(adjustment) = editor.vadjustment() {
            adjustment.connect_page_size_notify({
                let controls = controls.clone();
                move |_| controls.update_padding()
            });
        }
        controls
    }

    fn cancel_centering(&self) {
        if self.centering.replace(false) {
            self.generation.set(self.generation.get().wrapping_add(1));
        }
    }

    pub fn update_padding(&self) {
        let (Some(editor), Some(settings)) = (self.editor.upgrade(), self.settings.upgrade())
        else {
            return;
        };
        if !settings.borrow().centered_scrolling {
            self.cancel_centering();
        }
        let line = editor.iter_location(
            &editor
                .buffer()
                .iter_at_offset(editor.buffer().cursor_position()),
        );
        let padding = ((editor.visible_rect().height() - line.height()) / 2).max(0);
        let (top, bottom) = if settings.borrow().centered_scrolling {
            (padding, padding)
        } else {
            (14, 24)
        };
        let previous = editor.top_margin();
        if previous == top && editor.bottom_margin() == bottom {
            return;
        }
        let adjustment = editor.vadjustment();
        let schedule = adjustment.as_ref().is_some_and(|a| {
            if self.restoring.get().is_none() {
                self.restoring.set(Some((previous, a.value())));
                true
            } else {
                false
            }
        });
        editor.set_top_margin(top);
        editor.set_bottom_margin(bottom);
        if schedule {
            let adjustment = adjustment.unwrap().downgrade();
            let restoring = self.restoring.clone();
            let allocated = Cell::new(false);
            editor.add_tick_callback(move |editor, _| {
                if !allocated.replace(true) {
                    return glib::ControlFlow::Continue;
                }
                if let (Some(adjustment), Some((previous, value))) =
                    (adjustment.upgrade(), restoring.take())
                {
                    let value = value + f64::from(editor.top_margin() - previous);
                    adjustment.set_value(value.clamp(
                        adjustment.lower(),
                        (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                    ));
                }
                glib::ControlFlow::Break
            });
        }
    }

    pub fn center(&self) {
        let (Some(editor), Some(settings)) = (self.editor.upgrade(), self.settings.upgrade())
        else {
            return;
        };
        if !settings.borrow().centered_scrolling || self.centering.replace(true) {
            return;
        }
        let controls = self.clone();
        let generation = self.generation.get();
        let frames = Cell::new(0);
        editor.add_tick_callback(move |editor, _| {
            if controls.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let enabled = controls
                .settings
                .upgrade()
                .is_some_and(|s| s.borrow().centered_scrolling);
            if !enabled || !editor.is_mapped() {
                controls.centering.set(false);
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get() + 1);
            if frames.get() == 1 {
                controls.update_padding();
            }
            if frames.get() < 3 {
                return glib::ControlFlow::Continue;
            }
            let line = editor.iter_location(
                &editor
                    .buffer()
                    .iter_at_offset(editor.buffer().cursor_position()),
            );
            if let Some(adjustment) = editor.vadjustment() {
                let target = f64::from(line.y()) + f64::from(line.height()) / 2.0
                    - f64::from(editor.visible_rect().height()) / 2.0
                    + f64::from(editor.top_margin());
                adjustment.set_value(target.clamp(
                    adjustment.lower(),
                    (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                ));
            }
            controls.centering.set(false);
            glib::ControlFlow::Break
        });
    }
}

pub(super) const LENS_SIZE: f32 = 192.0;

#[derive(Clone)]
pub(super) struct PreviewMagnifier {
    pub toggle: gtk::ToggleButton,
    pub lens: gtk::Frame,
    overlay: gtk::Overlay,
}

pub(super) fn clear_lens(lens: &gtk::Frame) {
    lens.set_visible(false);
    if let Some(image) = lens.child().and_downcast::<gtk::Picture>() {
        image.set_paintable(None::<&gdk::Paintable>);
    }
}

impl PreviewMagnifier {
    pub fn new(overlay: &gtk::Overlay, scroll: &gtk::ScrolledWindow) -> Self {
        let toggle = gtk::ToggleButton::builder()
            .icon_name("edit-find-symbolic")
            .tooltip_text("Preview magnifier (2×)")
            .build();
        let image = gtk::Picture::new();
        image.set_can_target(false);
        image.set_size_request(LENS_SIZE as i32, LENS_SIZE as i32);
        image.set_content_fit(gtk::ContentFit::Fill);
        let lens = gtk::Frame::new(None);
        lens.set_child(Some(&image));
        lens.set_halign(gtk::Align::Start);
        lens.set_valign(gtk::Align::Start);
        lens.set_can_target(false);
        lens.add_css_class("preview-magnifier");
        lens.set_visible(false);
        overlay.add_overlay(&lens);
        overlay.set_measure_overlay(&lens, false);
        toggle.connect_toggled({
            let lens = lens.downgrade();
            move |_| {
                if let Some(lens) = lens.upgrade() {
                    clear_lens(&lens);
                }
            }
        });
        for adjustment in [scroll.hadjustment(), scroll.vadjustment()] {
            adjustment.connect_value_changed({
                let lens = lens.downgrade();
                move |_| {
                    if let Some(lens) = lens.upgrade() {
                        clear_lens(&lens);
                    }
                }
            });
            adjustment.connect_page_size_notify({
                let lens = lens.downgrade();
                move |_| {
                    if let Some(lens) = lens.upgrade() {
                        clear_lens(&lens);
                    }
                }
            });
        }
        Self {
            toggle,
            lens,
            overlay: overlay.clone(),
        }
    }

    pub fn hide(&self) {
        clear_lens(&self.lens);
    }

    pub fn attach(&self, picture: &gtk::Picture, aspect: f64) {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let picture = picture.downgrade();
            let overlay = self.overlay.downgrade();
            let lens = self.lens.downgrade();
            let toggle = self.toggle.downgrade();
            move |_, x, y| {
                let (Some(picture), Some(overlay), Some(lens), Some(toggle)) = (
                    picture.upgrade(),
                    overlay.upgrade(),
                    lens.upgrade(),
                    toggle.upgrade(),
                ) else {
                    return;
                };
                clear_lens(&lens);
                if !toggle.is_active()
                    || !picture.is_mapped()
                    || !page_paintable::is_current(&picture)
                {
                    return;
                }
                let Some(image) = lens.child().and_downcast::<gtk::Picture>() else {
                    return;
                };
                let Some(point) =
                    picture.compute_point(&overlay, &graphene::Point::new(x as f32, y as f32))
                else {
                    return;
                };
                let Some(root) = overlay.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let Some(bounds) = overlay.compute_bounds(&root) else {
                    return;
                };
                let left_edge = (-bounds.x()).max(0.0);
                let top_edge = (-bounds.y()).max(0.0);
                let right_edge = (root.width() as f32 - bounds.x()).min(overlay.width() as f32);
                let bottom_edge = (root.height() as f32 - bounds.y()).min(overlay.height() as f32);
                if point.x() < left_edge
                    || point.x() > right_edge
                    || point.y() < top_edge
                    || point.y() > bottom_edge
                {
                    return;
                }
                let size = LENS_SIZE
                    .min(right_edge - left_edge - 2.0)
                    .min(bottom_edge - top_edge - 2.0);
                if size < 64.0 {
                    return;
                }
                let Some(paintable) = magnified_image(&picture, aspect, x, y, size) else {
                    return;
                };
                let left = if point.x() + size + 24.0 <= right_edge {
                    point.x() + 24.0
                } else {
                    point.x() - size - 24.0
                };
                lens.set_margin_start(
                    (left as i32).clamp(left_edge as i32, (right_edge - size - 2.0) as i32),
                );
                lens.set_margin_top(
                    (point.y() as i32 - size as i32 / 2)
                        .clamp(top_edge as i32, (bottom_edge - size - 2.0) as i32),
                );
                image.set_size_request(size as i32, size as i32);
                image.set_paintable(Some(&paintable));
                lens.set_visible(true);
            }
        });
        motion.connect_leave({
            let lens = self.lens.downgrade();
            move |_| {
                if let Some(lens) = lens.upgrade() {
                    clear_lens(&lens);
                }
            }
        });
        for property in [
            "width-request",
            "height-request",
            "content-fit",
            "paintable",
            "scale-factor",
        ] {
            picture.connect_notify_local(Some(property), {
                let lens = self.lens.downgrade();
                move |_, _| {
                    if let Some(lens) = lens.upgrade() {
                        clear_lens(&lens);
                    }
                }
            });
        }
        picture.add_controller(motion);
    }
}

pub(super) fn drawn_page_size(width: f64, height: f64, aspect: f64, cover: bool) -> (f64, f64) {
    let height = if cover {
        height.max(width / aspect)
    } else {
        height.min(width / aspect)
    };
    (height * aspect, height)
}

fn magnified_image(
    picture: &gtk::Picture,
    aspect: f64,
    x: f64,
    y: f64,
    size: f32,
) -> Option<gdk::Paintable> {
    let (width, height) = (f64::from(picture.width()), f64::from(picture.height()));
    if width <= 0.0
        || height <= 0.0
        || !aspect.is_finite()
        || aspect <= 0.0
        || x < 0.0
        || y < 0.0
        || x > width
        || y > height
    {
        return None;
    }
    let (drawn_width, drawn_height) = drawn_page_size(
        width,
        height,
        aspect,
        picture.content_fit() == gtk::ContentFit::Cover,
    );
    let (x, y) = (
        x - (width - drawn_width) / 2.0,
        y - (height - drawn_height) / 2.0,
    );
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x > drawn_width || y > drawn_height
    {
        return None;
    }
    let snapshot = gtk::Snapshot::new();
    snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, size, size));
    snapshot.translate(&graphene::Point::new(
        size / 2.0 - (x * 2.0) as f32,
        size / 2.0 - (y * 2.0) as f32,
    ));
    // Freeze the cached texture, never ask the live page for a different raster size.
    picture
        .paintable()?
        .current_image()
        .snapshot(&snapshot, drawn_width * 2.0, drawn_height * 2.0);
    snapshot.pop();
    snapshot.to_paintable(Some(&graphene::Size::new(size, size)))
}
