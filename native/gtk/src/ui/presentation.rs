use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

use super::model::{
    format_elapsed, presentation_command_for_key, AnnotationStroke, Blackout, NormalizedPoint,
    PresentationCommand, PresentationState, PresentationTool, SlideNumberBuffer,
};

type RefreshCallback = Rc<dyn Fn()>;
type RefreshList = Rc<RefCell<Vec<RefreshCallback>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentationRole {
    Single,
    Audience,
    Presenter,
}

#[derive(Clone)]
pub struct PresentationController {
    application: gtk::Application,
    owner: gtk::ApplicationWindow,
    pages: Rc<RefCell<Vec<PathBuf>>>,
    inline_notes: Rc<RefCell<Vec<String>>>,
    sidecar_notes: Rc<RefCell<Vec<String>>>,
    rendered_notes: Rc<RefCell<Vec<Option<PathBuf>>>>,
    state: Rc<RefCell<PresentationState>>,
    windows: Rc<RefCell<Vec<gtk::ApplicationWindow>>>,
    audience_window: Rc<RefCell<Option<gtk::ApplicationWindow>>>,
    presenter_window: Rc<RefCell<Option<gtk::ApplicationWindow>>>,
    audience_monitor: Rc<Cell<Option<usize>>>,
    refreshers: RefreshList,
    number_buffer: Rc<RefCell<SlideNumberBuffer>>,
    timers: Rc<RefCell<Vec<glib::SourceId>>>,
    number_timeout: Rc<RefCell<Option<glib::SourceId>>>,
    scroll_accumulator: Rc<Cell<f64>>,
    last_scroll_navigation: Rc<Cell<Option<std::time::Instant>>>,
    annotation_color: Rc<Cell<(f64, f64, f64)>>,
    note_font_size: Rc<Cell<u32>>,
    pending_note_save: Rc<RefCell<Option<glib::SourceId>>>,
    pending_note_writes: Rc<RefCell<Vec<(usize, String)>>>,
    hud_timeout: Rc<RefCell<Option<glib::SourceId>>>,
    audience_cursor_timeout: Rc<RefCell<Option<glib::SourceId>>>,
    save_note_font_size: Rc<dyn Fn(u32)>,
    save_note: Rc<dyn Fn(usize, String) -> bool>,
}

impl PresentationController {
    pub fn new(
        application: &gtk::Application,
        owner: &gtk::ApplicationWindow,
        note_font_size: u32,
        save_note: Rc<dyn Fn(usize, String) -> bool>,
        save_note_font_size: Rc<dyn Fn(u32)>,
    ) -> Self {
        Self {
            application: application.clone(),
            owner: owner.clone(),
            pages: Rc::new(RefCell::new(Vec::new())),
            inline_notes: Rc::new(RefCell::new(Vec::new())),
            sidecar_notes: Rc::new(RefCell::new(Vec::new())),
            rendered_notes: Rc::new(RefCell::new(Vec::new())),
            state: Rc::new(RefCell::new(PresentationState::new(0))),
            windows: Rc::new(RefCell::new(Vec::new())),
            audience_window: Rc::new(RefCell::new(None)),
            presenter_window: Rc::new(RefCell::new(None)),
            audience_monitor: Rc::new(Cell::new(None)),
            refreshers: Rc::new(RefCell::new(Vec::new())),
            number_buffer: Rc::new(RefCell::new(SlideNumberBuffer::default())),
            timers: Rc::new(RefCell::new(Vec::new())),
            number_timeout: Rc::new(RefCell::new(None)),
            scroll_accumulator: Rc::new(Cell::new(0.0)),
            last_scroll_navigation: Rc::new(Cell::new(None)),
            annotation_color: Rc::new(Cell::new((1.0, 0.302, 0.0))),
            note_font_size: Rc::new(Cell::new(note_font_size.clamp(12, 34))),
            pending_note_save: Rc::new(RefCell::new(None)),
            pending_note_writes: Rc::new(RefCell::new(Vec::new())),
            hud_timeout: Rc::new(RefCell::new(None)),
            audience_cursor_timeout: Rc::new(RefCell::new(None)),
            save_note_font_size,
            save_note,
        }
    }

    pub fn set_deck(
        &self,
        pages: Vec<PathBuf>,
        inline_notes: Vec<String>,
        sidecar_notes: Vec<String>,
        rendered_notes: Vec<Option<PathBuf>>,
    ) {
        let slide_count = pages.len();
        self.pages.replace(pages);
        self.rendered_notes.replace(rendered_notes);
        self.inline_notes.replace(inline_notes);
        if self.windows.borrow().is_empty() {
            self.sidecar_notes.replace(sidecar_notes);
        } else {
            let mut live_notes = self.sidecar_notes.borrow_mut();
            live_notes.resize(slide_count, String::new());
        }
        let mut state = self.state.borrow_mut();
        state.slide_count = slide_count;
        state.slide = state.slide.min(slide_count.saturating_sub(1));
        state.strokes.resize_with(slide_count, Vec::new);
        drop(state);
        self.refresh();
    }

    pub fn page_count(&self) -> usize {
        self.pages.borrow().len()
    }

    pub fn window_count(&self) -> usize {
        self.windows.borrow().len()
    }

    pub fn start_single(&self) {
        if !self.end() {
            return;
        }
        self.reset_session();
        let available_monitors = monitors();
        self.audience_monitor
            .set(owner_monitor_index(&self.owner, &available_monitors));
        let window = self.build_single_window();
        window.fullscreen();
        window.present();
        self.windows.borrow_mut().push(window);
    }

    pub fn start_presenter(&self) {
        if !self.end() {
            return;
        }
        self.reset_session();
        self.state.borrow_mut().notes_visible = true;
        let available_monitors = monitors();
        let owner_monitor = owner_monitor_index(&self.owner, &available_monitors);
        let audience_monitor = default_audience_monitor(available_monitors.len(), owner_monitor);
        self.audience_monitor.set(audience_monitor);
        let audience = self.build_stage_window("Audience — typsmthng", PresentationRole::Audience);
        if let Some(monitor) = audience_monitor.and_then(|index| available_monitors.get(index)) {
            audience.fullscreen_on_monitor(monitor);
        } else {
            audience.fullscreen();
        }
        audience.present();
        self.audience_window.replace(Some(audience.clone()));

        let presenter = self.build_presenter_window();
        presenter.present();
        self.presenter_window.replace(Some(presenter.clone()));
        self.windows.borrow_mut().extend([audience, presenter]);
    }

    pub fn end(&self) -> bool {
        if !self.flush_pending_note() {
            return false;
        }
        for timer in self.timers.borrow_mut().drain(..) {
            timer.remove();
        }
        if let Some(timeout) = self.number_timeout.borrow_mut().take() {
            timeout.remove();
        }
        if let Some(timeout) = self.hud_timeout.borrow_mut().take() {
            timeout.remove();
        }
        if let Some(timeout) = self.audience_cursor_timeout.borrow_mut().take() {
            timeout.remove();
        }
        self.audience_window.replace(None);
        self.presenter_window.replace(None);
        self.refreshers.borrow_mut().clear();
        let windows = self.windows.borrow_mut().drain(..).collect::<Vec<_>>();
        for window in windows {
            window.close();
        }
        self.owner.present();
        true
    }

    fn reset_session(&self) {
        self.state
            .replace(PresentationState::new(self.pages.borrow().len()));
        self.number_buffer.replace(SlideNumberBuffer::default());
        self.scroll_accumulator.set(0.0);
        self.last_scroll_navigation.set(None);
    }

    fn audience_closed(&self, window: &gtk::ApplicationWindow, stage_refresh: &RefreshCallback) {
        if self
            .audience_window
            .borrow()
            .as_ref()
            .is_some_and(|audience| audience == window)
        {
            self.audience_window.replace(None);
        }
        self.windows
            .borrow_mut()
            .retain(|candidate| candidate != window);
        self.refreshers
            .borrow_mut()
            .retain(|refresh| !Rc::ptr_eq(refresh, stage_refresh));
        if let Some(timeout) = self.audience_cursor_timeout.borrow_mut().take() {
            timeout.remove();
        }
        self.refresh();
    }

    pub fn next(&self) {
        self.state.borrow_mut().next();
        self.refresh();
    }

    pub fn previous(&self) {
        self.state.borrow_mut().previous();
        self.refresh();
    }

    fn refresh(&self) {
        for refresh in self.refreshers.borrow().iter() {
            refresh();
        }
    }

    fn build_stage_window(&self, title: &str, role: PresentationRole) -> gtk::ApplicationWindow {
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title(title)
            .default_width(1280)
            .default_height(720)
            .build();
        window.add_css_class("presentation-shell");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let stage = self.build_slide_stage();
        let stage_refresh = stage.refresh.clone();
        root.append(&stage.root);
        window.set_child(Some(&root));
        self.attach_input(&window, role, None);
        window.connect_close_request({
            let controller = self.clone();
            let stage_refresh = stage_refresh.clone();
            move |window| {
                if role == PresentationRole::Audience {
                    controller.audience_closed(window, &stage_refresh);
                    glib::Propagation::Proceed
                } else if controller.end() {
                    glib::Propagation::Proceed
                } else {
                    glib::Propagation::Stop
                }
            }
        });
        if role == PresentationRole::Audience {
            self.install_audience_cursor_hide(&window);
        }
        window
    }

    fn build_slide_stage(&self) -> SlideStage {
        let root = gtk::Overlay::new();
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.add_css_class("stage");
        let picture = gtk::Picture::new();
        picture.set_keep_aspect_ratio(true);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        root.set_child(Some(&picture));

        let drawing = gtk::DrawingArea::new();
        drawing.set_hexpand(true);
        drawing.set_vexpand(true);
        root.add_overlay(&drawing);
        let blackout = gtk::Box::new(gtk::Orientation::Vertical, 0);
        blackout.set_hexpand(true);
        blackout.set_vexpand(true);
        blackout.set_visible(false);
        blackout.set_can_target(false);
        root.add_overlay(&blackout);
        let laser = gtk::DrawingArea::new();
        laser.set_hexpand(true);
        laser.set_vexpand(true);
        laser.set_can_target(false);
        root.add_overlay(&laser);
        laser.set_draw_func({
            let state = self.state.clone();
            let picture = picture.clone();
            move |_, cr, width, height| {
                let state = state.borrow();
                if state.tool != PresentationTool::Laser {
                    return;
                }
                let Some(point) = state.laser else { return };
                let slide = fitted_slide_rect(&picture, width as f64, height as f64);
                cr.set_source_rgba(1.0, 0.08, 0.02, 0.92);
                cr.arc(
                    slide.x + point.x * slide.width,
                    slide.y + point.y * slide.height,
                    9.0,
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
            }
        });
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let state = self.state.clone();
            let controller = self.clone();
            let picture = picture.clone();
            move |motion_controller, x, y| {
                if state.borrow().tool == PresentationTool::Laser {
                    let Some(widget) = motion_controller.widget() else {
                        return;
                    };
                    state.borrow_mut().laser = normalize_to_slide(
                        &picture,
                        widget.width() as f64,
                        widget.height() as f64,
                        x,
                        y,
                    );
                    controller.refresh();
                }
            }
        });
        motion.connect_leave({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                state.borrow_mut().laser = None;
                controller.refresh();
            }
        });
        drawing.add_controller(motion);

        drawing.set_draw_func({
            let state = self.state.clone();
            let picture = picture.clone();
            move |_, cr, width, height| {
                let state = state.borrow();
                let slide = fitted_slide_rect(&picture, width as f64, height as f64);
                let Some(strokes) = state.strokes.get(state.slide) else {
                    return;
                };
                for stroke in strokes {
                    if stroke.points.is_empty() {
                        continue;
                    }
                    let (r, g, b, a) = stroke.color;
                    cr.set_source_rgba(r, g, b, a);
                    cr.set_line_cap(gtk::cairo::LineCap::Round);
                    cr.set_line_join(gtk::cairo::LineJoin::Round);
                    let line_width = match stroke.tool {
                        PresentationTool::Highlighter => (slide.width * 0.018).max(10.0),
                        _ => (slide.width * 0.004).max(3.0),
                    };
                    cr.set_line_width(line_width);
                    let first = stroke.points[0];
                    if stroke.points.len() == 1 {
                        cr.arc(
                            slide.x + first.x * slide.width,
                            slide.y + first.y * slide.height,
                            line_width / 2.0,
                            0.0,
                            std::f64::consts::TAU,
                        );
                        let _ = cr.fill();
                        continue;
                    }
                    cr.move_to(
                        slide.x + first.x * slide.width,
                        slide.y + first.y * slide.height,
                    );
                    for point in &stroke.points[1..] {
                        cr.line_to(
                            slide.x + point.x * slide.width,
                            slide.y + point.y * slide.height,
                        );
                    }
                    let _ = cr.stroke();
                }
            }
        });

        let pending = Rc::new(RefCell::new(None::<AnnotationStroke>));
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.connect_drag_begin({
            let state = self.state.clone();
            let pending = pending.clone();
            let drawing = drawing.clone();
            let picture = picture.clone();
            let annotation_color = self.annotation_color.clone();
            let controller = self.clone();
            move |_, x, y| {
                let tool = state.borrow().tool;
                if matches!(tool, PresentationTool::Pen | PresentationTool::Highlighter) {
                    let width = drawing.width() as f64;
                    let height = drawing.height() as f64;
                    let (red, green, blue) = annotation_color.get();
                    let color = (
                        red,
                        green,
                        blue,
                        if tool == PresentationTool::Highlighter {
                            0.36
                        } else {
                            0.96
                        },
                    );
                    if let Some(point) = normalize_to_slide(&picture, width, height, x, y) {
                        pending.replace(Some(AnnotationStroke {
                            tool,
                            color,
                            points: vec![point],
                        }));
                    }
                } else if tool == PresentationTool::Eraser
                    && erase_stroke_at(&state, &picture, &drawing, x, y)
                {
                    controller.refresh();
                }
            }
        });
        drag.connect_drag_update({
            let pending = pending.clone();
            let drawing = drawing.clone();
            let picture = picture.clone();
            let state = self.state.clone();
            let controller = self.clone();
            move |gesture, dx, dy| {
                let Some((start_x, start_y)) = gesture.start_point() else {
                    return;
                };
                let width = drawing.width() as f64;
                let height = drawing.height() as f64;
                if let Some(stroke) = pending.borrow_mut().as_mut() {
                    if let Some(point) =
                        normalize_to_slide(&picture, width, height, start_x + dx, start_y + dy)
                    {
                        stroke.points.push(point);
                    }
                } else if state.borrow().tool == PresentationTool::Eraser
                    && erase_stroke_at(&state, &picture, &drawing, start_x + dx, start_y + dy)
                {
                    controller.refresh();
                }
                drawing.queue_draw();
            }
        });
        drag.connect_drag_end({
            let state = self.state.clone();
            let pending = pending.clone();
            let controller = self.clone();
            move |_, _, _| {
                if let Some(stroke) = pending.borrow_mut().take() {
                    let slide = state.borrow().slide;
                    if let Some(strokes) = state.borrow_mut().strokes.get_mut(slide) {
                        strokes.push(stroke);
                    }
                    controller.refresh();
                }
            }
        });
        drawing.add_controller(drag);

        let refresh: Rc<dyn Fn()> = {
            let pages = self.pages.clone();
            let state = self.state.clone();
            let picture = picture.clone();
            let drawing = drawing.clone();
            let blackout = blackout.clone();
            let laser = laser.clone();
            Rc::new(move || {
                let state = state.borrow();
                if let Some(path) = pages.borrow().get(state.slide) {
                    picture.set_filename(Some(path));
                } else {
                    picture.set_paintable(gtk::gdk::Paintable::NONE);
                }
                match state.blackout {
                    Blackout::None => blackout.set_visible(false),
                    Blackout::Black => {
                        blackout.set_visible(true);
                        blackout.remove_css_class("whiteout");
                        blackout.add_css_class("blackout");
                    }
                    Blackout::White => {
                        blackout.set_visible(true);
                        blackout.remove_css_class("blackout");
                        blackout.add_css_class("whiteout");
                    }
                }
                drawing.queue_draw();
                laser.queue_draw();
            })
        };
        refresh();
        self.refreshers.borrow_mut().push(refresh.clone());
        SlideStage { root, refresh }
    }

    fn build_single_window(&self) -> gtk::ApplicationWindow {
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title("Presentation — typsmthng")
            .default_width(1280)
            .default_height(720)
            .build();
        window.add_css_class("presentation-shell");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&self.build_slide_stage().root));

        let notes = self.build_single_notes_panel();
        notes.set_halign(gtk::Align::End);
        notes.set_valign(gtk::Align::Fill);
        notes.set_size_request(400, -1);
        overlay.add_overlay(&notes);

        let hud = gtk::Revealer::new();
        hud.set_transition_type(gtk::RevealerTransitionType::Crossfade);
        hud.set_transition_duration(150);
        hud.set_halign(gtk::Align::Center);
        hud.set_valign(gtk::Align::End);
        hud.set_margin_bottom(18);
        hud.set_margin_start(18);
        hud.set_margin_end(18);
        hud.set_child(Some(&self.build_toolbar(&window, PresentationRole::Single)));
        overlay.add_overlay(&hud);
        window.set_child(Some(&overlay));
        self.attach_input(&window, PresentationRole::Single, Some(&hud));
        window.connect_close_request({
            let controller = self.clone();
            move |_| {
                if controller.end() {
                    glib::Propagation::Proceed
                } else {
                    glib::Propagation::Stop
                }
            }
        });

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let controller = self.clone();
            let hud = hud.clone();
            move |_, _, _| controller.poke_hud(&hud)
        });
        overlay.add_controller(motion);
        let hud_hover = gtk::EventControllerMotion::new();
        hud_hover.connect_enter({
            let controller = self.clone();
            let hud = hud.clone();
            move |_, _, _| controller.hold_hud(&hud)
        });
        hud_hover.connect_leave({
            let controller = self.clone();
            let hud = hud.clone();
            move |_| controller.poke_hud(&hud)
        });
        hud.add_controller(hud_hover);
        self.poke_hud(&hud);
        window
    }

    fn build_single_notes_panel(&self) -> gtk::Box {
        let aside = gtk::Box::new(gtk::Orientation::Vertical, 10);
        aside.add_css_class("notes");
        aside.set_margin_top(12);
        aside.set_margin_bottom(12);
        aside.set_margin_start(12);
        aside.set_margin_end(12);
        let next_label = gtk::Label::new(Some("NEXT"));
        next_label.add_css_class("eyebrow");
        next_label.set_halign(gtk::Align::Start);
        aside.append(&next_label);
        let next_picture = gtk::Picture::new();
        next_picture.set_keep_aspect_ratio(true);
        next_picture.set_size_request(340, 190);
        aside.append(&next_picture);
        let notes_header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let notes_label = gtk::Label::new(Some("SPEAKER NOTES"));
        notes_label.add_css_class("eyebrow");
        notes_label.set_halign(gtk::Align::Start);
        notes_label.set_hexpand(true);
        let notes_smaller = gtk::Button::with_label("−");
        notes_smaller.set_tooltip_text(Some("Smaller speaker notes"));
        let notes_size = gtk::Label::new(Some(&self.note_font_size.get().to_string()));
        notes_size.add_css_class("mono");
        let notes_larger = gtk::Button::with_label("+");
        notes_larger.set_tooltip_text(Some("Larger speaker notes"));
        notes_header.append(&notes_label);
        notes_header.append(&notes_smaller);
        notes_header.append(&notes_size);
        notes_header.append(&notes_larger);
        aside.append(&notes_header);
        let inline_note = gtk::Label::new(None);
        inline_note.set_wrap(true);
        inline_note.set_halign(gtk::Align::Start);
        inline_note.add_css_class("muted");
        inline_note.add_css_class("speaker-notes-text");
        aside.append(&inline_note);
        let rendered_note = gtk::Picture::new();
        rendered_note.set_keep_aspect_ratio(true);
        rendered_note.set_can_shrink(true);
        rendered_note.set_size_request(340, 190);
        rendered_note.set_visible(false);
        aside.append(&rendered_note);
        let note_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let note_view = gtk::TextView::with_buffer(&note_buffer);
        note_view.add_css_class("speaker-notes-text");
        note_view.set_wrap_mode(gtk::WrapMode::WordChar);
        note_view.set_top_margin(10);
        note_view.set_bottom_margin(10);
        note_view.set_left_margin(10);
        note_view.set_right_margin(10);
        let note_scroll = gtk::ScrolledWindow::new();
        note_scroll.set_vexpand(true);
        note_scroll.set_child(Some(&note_view));
        aside.append(&note_scroll);

        let notes_css = gtk::CssProvider::new();
        let apply_note_size: Rc<dyn Fn()> = {
            let notes_css = notes_css.clone();
            let note_font_size = self.note_font_size.clone();
            Rc::new(move || {
                notes_css.load_from_data(&format!(
                    ".speaker-notes-text {{ font-size: {}px; }}",
                    note_font_size.get()
                ));
            })
        };
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&aside),
            &notes_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        apply_note_size();
        notes_smaller.connect_clicked({
            let note_font_size = self.note_font_size.clone();
            let save = self.save_note_font_size.clone();
            let apply = apply_note_size.clone();
            let label = notes_size.clone();
            move |_| {
                let size = note_font_size.get().saturating_sub(2).max(12);
                note_font_size.set(size);
                label.set_text(&size.to_string());
                apply();
                save(size);
            }
        });
        notes_larger.connect_clicked({
            let note_font_size = self.note_font_size.clone();
            let save = self.save_note_font_size.clone();
            let apply = apply_note_size.clone();
            let label = notes_size.clone();
            move |_| {
                let size = (note_font_size.get() + 2).min(34);
                note_font_size.set(size);
                label.set_text(&size.to_string());
                apply();
                save(size);
            }
        });

        let suppress_note_change = Rc::new(Cell::new(false));
        note_buffer.connect_changed({
            let suppress_note_change = suppress_note_change.clone();
            let state = self.state.clone();
            let sidecar_notes = self.sidecar_notes.clone();
            let controller = self.clone();
            move |buffer| {
                if suppress_note_change.get() {
                    return;
                }
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .to_string();
                let slide = state.borrow().slide;
                let mut notes = sidecar_notes.borrow_mut();
                if notes.len() <= slide {
                    notes.resize(slide + 1, String::new());
                }
                notes[slide] = text.clone();
                drop(notes);
                controller.schedule_note_save(slide, text);
            }
        });

        let refresh: Rc<dyn Fn()> = {
            let state = self.state.clone();
            let pages = self.pages.clone();
            let inline_notes = self.inline_notes.clone();
            let sidecar_notes = self.sidecar_notes.clone();
            let rendered_notes = self.rendered_notes.clone();
            let suppress_note_change = suppress_note_change.clone();
            let aside = aside.clone();
            Rc::new(move || {
                let slide = state.borrow().slide;
                aside.set_visible(state.borrow().notes_visible);
                if let Some(path) = pages.borrow().get(slide + 1) {
                    next_picture.set_filename(Some(path));
                } else {
                    next_picture.set_paintable(gtk::gdk::Paintable::NONE);
                }
                let note = sidecar_notes
                    .borrow()
                    .get(slide)
                    .cloned()
                    .unwrap_or_default();
                let current = note_buffer
                    .text(&note_buffer.start_iter(), &note_buffer.end_iter(), true)
                    .to_string();
                if current != note {
                    suppress_note_change.set(true);
                    note_buffer.set_text(&note);
                    suppress_note_change.set(false);
                }
                inline_note.set_label(
                    inline_notes
                        .borrow()
                        .get(slide)
                        .map(String::as_str)
                        .unwrap_or_default(),
                );
                inline_note.set_visible(!inline_note.label().is_empty());
                if let Some(Some(path)) = rendered_notes.borrow().get(slide) {
                    rendered_note.set_filename(Some(path));
                    rendered_note.set_visible(true);
                } else {
                    rendered_note.set_paintable(gtk::gdk::Paintable::NONE);
                    rendered_note.set_visible(false);
                }
            })
        };
        refresh();
        self.refreshers.borrow_mut().push(refresh);
        aside
    }

    fn build_presenter_window(&self) -> gtk::ApplicationWindow {
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title("Presenter console — typsmthng")
            .default_width(1240)
            .default_height(780)
            .build();
        window.add_css_class("presentation-shell");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let console = gtk::Paned::new(gtk::Orientation::Horizontal);
        console.set_position(820);
        console.set_vexpand(true);
        console.set_start_child(Some(&self.build_slide_stage().root));

        let aside = gtk::Box::new(gtk::Orientation::Vertical, 10);
        aside.add_css_class("notes");
        aside.set_margin_top(12);
        aside.set_margin_bottom(12);
        aside.set_margin_start(12);
        aside.set_margin_end(12);
        let next_label = gtk::Label::new(Some("NEXT"));
        next_label.add_css_class("eyebrow");
        next_label.set_halign(gtk::Align::Start);
        aside.append(&next_label);
        let next_picture = gtk::Picture::new();
        next_picture.set_keep_aspect_ratio(true);
        next_picture.set_size_request(340, 190);
        aside.append(&next_picture);
        let notes_header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let notes_label = gtk::Label::new(Some("SPEAKER NOTES"));
        notes_label.add_css_class("eyebrow");
        notes_label.set_halign(gtk::Align::Start);
        notes_label.set_hexpand(true);
        let notes_smaller = gtk::Button::with_label("−");
        notes_smaller.set_tooltip_text(Some("Smaller speaker notes"));
        let notes_size = gtk::Label::new(Some(&self.note_font_size.get().to_string()));
        notes_size.add_css_class("mono");
        let notes_larger = gtk::Button::with_label("+");
        notes_larger.set_tooltip_text(Some("Larger speaker notes"));
        notes_header.append(&notes_label);
        notes_header.append(&notes_smaller);
        notes_header.append(&notes_size);
        notes_header.append(&notes_larger);
        aside.append(&notes_header);
        let inline_note = gtk::Label::new(None);
        inline_note.set_wrap(true);
        inline_note.set_halign(gtk::Align::Start);
        inline_note.add_css_class("muted");
        inline_note.add_css_class("speaker-notes-text");
        aside.append(&inline_note);
        let rendered_note = gtk::Picture::new();
        rendered_note.set_keep_aspect_ratio(true);
        rendered_note.set_can_shrink(true);
        rendered_note.set_size_request(340, 190);
        rendered_note.set_visible(false);
        aside.append(&rendered_note);
        let note_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let note_view = gtk::TextView::with_buffer(&note_buffer);
        note_view.add_css_class("speaker-notes-text");
        note_view.set_wrap_mode(gtk::WrapMode::WordChar);
        note_view.set_top_margin(10);
        note_view.set_bottom_margin(10);
        note_view.set_left_margin(10);
        note_view.set_right_margin(10);
        let note_scroll = gtk::ScrolledWindow::new();
        note_scroll.set_vexpand(true);
        note_scroll.set_child(Some(&note_view));
        aside.append(&note_scroll);

        let notes_css = gtk::CssProvider::new();
        let apply_note_size: Rc<dyn Fn()> = {
            let notes_css = notes_css.clone();
            let note_font_size = self.note_font_size.clone();
            Rc::new(move || {
                notes_css.load_from_data(&format!(
                    ".speaker-notes-text {{ font-size: {}px; }}",
                    note_font_size.get()
                ));
            })
        };
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&window),
            &notes_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        apply_note_size();
        notes_smaller.connect_clicked({
            let note_font_size = self.note_font_size.clone();
            let save = self.save_note_font_size.clone();
            let apply = apply_note_size.clone();
            let label = notes_size.clone();
            move |_| {
                let size = note_font_size.get().saturating_sub(2).max(12);
                note_font_size.set(size);
                label.set_text(&size.to_string());
                apply();
                save(size);
            }
        });
        notes_larger.connect_clicked({
            let note_font_size = self.note_font_size.clone();
            let save = self.save_note_font_size.clone();
            let apply = apply_note_size.clone();
            let label = notes_size.clone();
            move |_| {
                let size = (note_font_size.get() + 2).min(34);
                note_font_size.set(size);
                label.set_text(&size.to_string());
                apply();
                save(size);
            }
        });

        let suppress_note_change = Rc::new(Cell::new(false));
        note_buffer.connect_changed({
            let suppress_note_change = suppress_note_change.clone();
            let state = self.state.clone();
            let sidecar_notes = self.sidecar_notes.clone();
            let controller = self.clone();
            move |buffer| {
                if suppress_note_change.get() {
                    return;
                }
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .to_string();
                let slide = state.borrow().slide;
                let mut notes = sidecar_notes.borrow_mut();
                if notes.len() <= slide {
                    notes.resize(slide + 1, String::new());
                }
                notes[slide] = text.clone();
                drop(notes);
                controller.schedule_note_save(slide, text);
            }
        });
        console.set_end_child(Some(&aside));
        root.append(&console);
        root.append(&self.build_toolbar(&window, PresentationRole::Presenter));
        window.set_child(Some(&root));
        self.attach_input(&window, PresentationRole::Presenter, None);
        window.connect_close_request({
            let controller = self.clone();
            move |_| {
                if controller.end() {
                    glib::Propagation::Proceed
                } else {
                    glib::Propagation::Stop
                }
            }
        });

        let refresh: Rc<dyn Fn()> = {
            let state = self.state.clone();
            let pages = self.pages.clone();
            let inline_notes = self.inline_notes.clone();
            let sidecar_notes = self.sidecar_notes.clone();
            let rendered_notes = self.rendered_notes.clone();
            let suppress_note_change = suppress_note_change.clone();
            let aside = aside.clone();
            Rc::new(move || {
                let slide = state.borrow().slide;
                aside.set_visible(state.borrow().notes_visible);
                if let Some(path) = pages.borrow().get(slide + 1) {
                    next_picture.set_filename(Some(path));
                } else {
                    next_picture.set_paintable(gtk::gdk::Paintable::NONE);
                }
                let note = sidecar_notes
                    .borrow()
                    .get(slide)
                    .cloned()
                    .unwrap_or_default();
                let current = note_buffer
                    .text(&note_buffer.start_iter(), &note_buffer.end_iter(), true)
                    .to_string();
                if current != note {
                    suppress_note_change.set(true);
                    note_buffer.set_text(&note);
                    suppress_note_change.set(false);
                }
                inline_note.set_label(
                    inline_notes
                        .borrow()
                        .get(slide)
                        .map(String::as_str)
                        .unwrap_or_default(),
                );
                inline_note.set_visible(!inline_note.label().is_empty());
                if let Some(Some(path)) = rendered_notes.borrow().get(slide) {
                    rendered_note.set_filename(Some(path));
                    rendered_note.set_visible(true);
                } else {
                    rendered_note.set_paintable(gtk::gdk::Paintable::NONE);
                    rendered_note.set_visible(false);
                }
            })
        };
        refresh();
        self.refreshers.borrow_mut().push(refresh);
        window
    }

    fn schedule_note_save(&self, slide: usize, text: String) {
        if let Some(source) = self.pending_note_save.borrow_mut().take() {
            source.remove();
        }
        let mut pending = self.pending_note_writes.borrow_mut();
        pending.retain(|(pending_slide, _)| *pending_slide != slide);
        pending.push((slide, text));
        drop(pending);
        let pending_note_save = self.pending_note_save.clone();
        let pending_note_writes = self.pending_note_writes.clone();
        let save_note = self.save_note.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(600), move || {
            pending_note_save.borrow_mut().take();
            let pending = std::mem::take(&mut *pending_note_writes.borrow_mut());
            let mut failed = Vec::new();
            for (slide, text) in pending {
                if !save_note(slide, text.clone()) {
                    failed.push((slide, text));
                }
            }
            pending_note_writes.borrow_mut().extend(failed);
        });
        self.pending_note_save.replace(Some(source));
    }

    pub fn flush_pending_note(&self) -> bool {
        if let Some(source) = self.pending_note_save.borrow_mut().take() {
            source.remove();
        }
        let pending = std::mem::take(&mut *self.pending_note_writes.borrow_mut());
        let mut failed = Vec::new();
        for (slide, text) in pending {
            if !(self.save_note)(slide, text.clone()) {
                failed.push((slide, text));
            }
        }
        let saved_all = failed.is_empty();
        self.pending_note_writes.borrow_mut().extend(failed);
        saved_all
    }

    fn hold_hud(&self, hud: &gtk::Revealer) {
        if let Some(source) = self.hud_timeout.borrow_mut().take() {
            source.remove();
        }
        hud.set_reveal_child(true);
    }

    fn poke_hud(&self, hud: &gtk::Revealer) {
        self.hold_hud(hud);
        let timeout_slot = self.hud_timeout.clone();
        let timeout_callback = timeout_slot.clone();
        let state = self.state.clone();
        let hud = hud.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(2600), move || {
            timeout_callback.borrow_mut().take();
            if !state.borrow().notes_visible {
                hud.set_reveal_child(false);
            }
        });
        timeout_slot.replace(Some(source));
    }

    fn install_audience_cursor_hide(&self, window: &gtk::ApplicationWindow) {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let controller = self.clone();
            let window = window.clone();
            move |_, _, _| controller.poke_audience_cursor(&window)
        });
        window.add_controller(motion);
        self.poke_audience_cursor(window);
    }

    fn poke_audience_cursor(&self, window: &gtk::ApplicationWindow) {
        window.set_cursor_from_name(None);
        if let Some(source) = self.audience_cursor_timeout.borrow_mut().take() {
            source.remove();
        }
        let timeout_slot = self.audience_cursor_timeout.clone();
        let timeout_callback = timeout_slot.clone();
        let window = window.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(1800), move || {
            timeout_callback.borrow_mut().take();
            window.set_cursor_from_name(Some("none"));
        });
        timeout_slot.replace(Some(source));
    }

    fn open_audience_on_monitor(&self, index: usize) {
        let available = monitors();
        let Some(monitor) = available.get(index) else {
            return;
        };
        if let Some(window) = self.audience_window.borrow().as_ref() {
            window.fullscreen_on_monitor(monitor);
            window.present();
            self.audience_monitor.set(Some(index));
            return;
        }
        let audience = self.build_stage_window("Audience — typsmthng", PresentationRole::Audience);
        audience.fullscreen_on_monitor(monitor);
        audience.present();
        self.audience_monitor.set(Some(index));
        self.audience_window.replace(Some(audience.clone()));
        self.windows.borrow_mut().push(audience);
        self.refresh();
    }

    fn close_audience(&self) {
        let Some(audience) = self.audience_window.borrow_mut().take() else {
            return;
        };
        self.windows
            .borrow_mut()
            .retain(|candidate| candidate != &audience);
        if let Some(timeout) = self.audience_cursor_timeout.borrow_mut().take() {
            timeout.remove();
        }
        audience.close();
        self.refresh();
    }

    fn build_toolbar(&self, parent: &gtk::ApplicationWindow, role: PresentationRole) -> gtk::Box {
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        toolbar.add_css_class("presentation-toolbar");
        let previous = tool_button("go-previous-symbolic", "Previous slide");
        let next = tool_button("go-next-symbolic", "Next slide");
        let slide = gtk::Label::new(Some("1 / 1"));
        slide.add_css_class("mono");
        let timer = gtk::Label::new(Some("00:00"));
        timer.add_css_class("mono");
        let clock = gtk::Label::new(Some("00:00"));
        clock.add_css_class("mono");
        let timer_toggle = tool_button("media-playback-pause-symbolic", "Pause / resume timer (T)");
        let timer_reset = tool_button("view-refresh-symbolic", "Reset timer (R)");
        let pointer = gtk::ToggleButton::with_label("Pointer");
        pointer.set_active(true);
        let laser = gtk::ToggleButton::with_label("Laser");
        laser.set_group(Some(&pointer));
        let pen = gtk::ToggleButton::with_label("Pen");
        pen.set_group(Some(&pointer));
        let highlighter = gtk::ToggleButton::with_label("Highlight");
        highlighter.set_group(Some(&pointer));
        let eraser = gtk::ToggleButton::with_label("Eraser");
        eraser.set_group(Some(&pointer));
        let colors = gtk::DropDown::from_strings(&[
            "Orange", "Yellow", "Green", "Blue", "Pink", "Black", "White",
        ]);
        colors.set_tooltip_text(Some("Annotation colour"));
        let clear = tool_button("edit-clear-all-symbolic", "Clear slide annotations (C)");
        let black = gtk::ToggleButton::with_label("Black");
        let white = gtk::ToggleButton::with_label("White");
        let grid = gtk::Button::with_label("Grid");
        let notes = gtk::ToggleButton::with_label("Notes");
        let audience_toggle = gtk::Button::with_label("Close audience");
        audience_toggle.set_visible(role == PresentationRole::Presenter);
        let display_labels = monitor_labels();
        let display_label_refs = display_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let displays = gtk::DropDown::from_strings(&display_label_refs);
        displays.set_selected(self.audience_monitor.get().unwrap_or(0) as u32);
        displays.set_tooltip_text(Some("Audience display"));
        let exit = gtk::Button::with_label("End show");
        exit.add_css_class("destructive-action");
        for widget in [
            previous.upcast_ref::<gtk::Widget>(),
            next.upcast_ref(),
            slide.upcast_ref(),
            timer.upcast_ref(),
            clock.upcast_ref(),
            timer_toggle.upcast_ref(),
            timer_reset.upcast_ref(),
            pointer.upcast_ref(),
            laser.upcast_ref(),
            pen.upcast_ref(),
            highlighter.upcast_ref(),
            eraser.upcast_ref(),
            colors.upcast_ref(),
            clear.upcast_ref(),
            black.upcast_ref(),
            white.upcast_ref(),
            grid.upcast_ref(),
            notes.upcast_ref(),
            audience_toggle.upcast_ref(),
            displays.upcast_ref(),
            exit.upcast_ref(),
        ] {
            toolbar.append(widget);
        }
        previous.connect_clicked({
            let controller = self.clone();
            move |_| controller.previous()
        });
        next.connect_clicked({
            let controller = self.clone();
            move |_| controller.next()
        });
        pointer.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                state.borrow_mut().tool = PresentationTool::Pointer;
                controller.refresh();
            }
        });
        laser.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                let mut state = state.borrow_mut();
                state.tool = if state.tool == PresentationTool::Laser {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Laser
                };
                drop(state);
                controller.refresh();
            }
        });
        pen.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                let mut state = state.borrow_mut();
                state.tool = if state.tool == PresentationTool::Pen {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Pen
                };
                drop(state);
                controller.refresh();
            }
        });
        highlighter.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                let mut state = state.borrow_mut();
                state.tool = if state.tool == PresentationTool::Highlighter {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Highlighter
                };
                drop(state);
                controller.refresh();
            }
        });
        eraser.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                let mut state = state.borrow_mut();
                state.tool = if state.tool == PresentationTool::Eraser {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Eraser
                };
                drop(state);
                controller.refresh();
            }
        });
        colors.connect_selected_notify({
            let annotation_color = self.annotation_color.clone();
            move |picker| {
                annotation_color.set(match picker.selected() {
                    1 => (1.0, 0.831, 0.0),
                    2 => (0.169, 0.831, 0.42),
                    3 => (0.231, 0.616, 1.0),
                    4 => (1.0, 0.231, 0.502),
                    5 => (0.067, 0.067, 0.067),
                    6 => (1.0, 1.0, 1.0),
                    _ => (1.0, 0.302, 0.0),
                });
            }
        });
        clear.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                let slide = state.borrow().slide;
                if let Some(strokes) = state.borrow_mut().strokes.get_mut(slide) {
                    strokes.clear();
                }
                controller.refresh();
            }
        });
        black.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                let blackout = state.borrow().blackout;
                state.borrow_mut().blackout = if blackout == Blackout::Black {
                    Blackout::None
                } else {
                    Blackout::Black
                };
                controller.refresh();
            }
        });
        white.connect_clicked({
            let state = self.state.clone();
            let controller = self.clone();
            move |_| {
                let blackout = state.borrow().blackout;
                state.borrow_mut().blackout = if blackout == Blackout::White {
                    Blackout::None
                } else {
                    Blackout::White
                };
                controller.refresh();
            }
        });
        timer_toggle.connect_clicked({
            let controller = self.clone();
            move |_| {
                controller.state.borrow_mut().toggle_timer();
                controller.refresh();
            }
        });
        timer_reset.connect_clicked({
            let controller = self.clone();
            move |_| {
                controller.state.borrow_mut().reset_timer();
                controller.refresh();
            }
        });
        grid.connect_clicked({
            let controller = self.clone();
            let parent = parent.clone();
            move |_| controller.show_grid(&parent)
        });
        notes.connect_clicked({
            let controller = self.clone();
            move |_| {
                let visible = controller.state.borrow().notes_visible;
                controller.state.borrow_mut().notes_visible = !visible;
                controller.refresh();
            }
        });
        displays.connect_selected_notify({
            let controller = self.clone();
            let parent = parent.clone();
            move |chooser| {
                let selected = chooser.selected() as usize;
                let available = monitors();
                if let Some(monitor) = available.get(selected) {
                    if role == PresentationRole::Presenter {
                        if let Some(window) = controller.audience_window.borrow().as_ref() {
                            window.fullscreen_on_monitor(monitor);
                            controller.audience_monitor.set(Some(selected));
                        }
                    } else {
                        parent.fullscreen_on_monitor(monitor);
                        controller.audience_monitor.set(Some(selected));
                    }
                }
            }
        });
        audience_toggle.connect_clicked({
            let controller = self.clone();
            let displays = displays.clone();
            move |_| {
                if controller.audience_window.borrow().is_some() {
                    controller.close_audience();
                } else {
                    controller.open_audience_on_monitor(displays.selected() as usize);
                }
            }
        });
        exit.connect_clicked({
            let controller = self.clone();
            move |_| {
                controller.end();
            }
        });

        let state = self.state.clone();
        let slide_copy = slide.clone();
        let timer_copy = timer.clone();
        let clock_copy = clock.clone();
        let timer_toggle_copy = timer_toggle.clone();
        let timer_source = glib::timeout_add_local(Duration::from_millis(250), move || {
            let state = state.borrow();
            slide_copy.set_text(&format!(
                "{} / {}",
                (state.slide + 1).min(state.slide_count),
                state.slide_count
            ));
            timer_copy.set_text(&format_elapsed(state.elapsed()));
            timer_toggle_copy.set_icon_name(if state.timer_running {
                "media-playback-pause-symbolic"
            } else {
                "media-playback-start-symbolic"
            });
            clock_copy.set_text(&glib::DateTime::now_local().map_or_else(
                |_| "--:--".to_string(),
                |now| {
                    now.format("%H:%M")
                        .map_or_else(|_| "--:--".into(), |v| v.into())
                },
            ));
            glib::ControlFlow::Continue
        });
        self.timers.borrow_mut().push(timer_source);
        let refresh: Rc<dyn Fn()> = {
            let state = self.state.clone();
            let pointer = pointer.clone();
            let laser = laser.clone();
            let pen = pen.clone();
            let highlighter = highlighter.clone();
            let eraser = eraser.clone();
            let black = black.clone();
            let white = white.clone();
            let notes = notes.clone();
            let audience_toggle = audience_toggle.clone();
            let audience_window = self.audience_window.clone();
            Rc::new(move || {
                let state = state.borrow();
                pointer.set_active(state.tool == PresentationTool::Pointer);
                laser.set_active(state.tool == PresentationTool::Laser);
                pen.set_active(state.tool == PresentationTool::Pen);
                highlighter.set_active(state.tool == PresentationTool::Highlighter);
                eraser.set_active(state.tool == PresentationTool::Eraser);
                black.set_active(state.blackout == Blackout::Black);
                white.set_active(state.blackout == Blackout::White);
                notes.set_active(state.notes_visible);
                audience_toggle.set_label(if audience_window.borrow().is_some() {
                    "Close audience"
                } else {
                    "Open audience"
                });
            })
        };
        refresh();
        self.refreshers.borrow_mut().push(refresh);
        toolbar
    }

    fn show_grid(&self, parent: &gtk::ApplicationWindow) {
        let window = gtk::Window::builder()
            .title("Slide grid")
            .transient_for(parent)
            .modal(true)
            .default_width(900)
            .default_height(640)
            .hide_on_close(true)
            .build();
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_column_spacing(12);
        flow.set_row_spacing(12);
        flow.set_margin_top(16);
        flow.set_margin_bottom(16);
        flow.set_margin_start(16);
        flow.set_margin_end(16);
        let (current_slide, annotated_slides) = {
            let state = self.state.borrow();
            (
                state.slide,
                state
                    .strokes
                    .iter()
                    .map(|strokes| !strokes.is_empty())
                    .collect::<Vec<_>>(),
            )
        };
        for (index, page) in self.pages.borrow().iter().enumerate() {
            let button = gtk::Button::new();
            if index == current_slide {
                button.add_css_class("annotation-active");
                button.set_tooltip_text(Some("Current slide"));
            }
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 5);
            let picture = gtk::Picture::for_filename(page);
            picture.set_keep_aspect_ratio(true);
            picture.set_size_request(210, 118);
            cell.append(&picture);
            cell.append(&gtk::Label::new(Some(&(index + 1).to_string())));
            if annotated_slides.get(index).copied().unwrap_or(false) {
                let annotated = gtk::Label::new(Some("ANNOTATED"));
                annotated.add_css_class("eyebrow");
                cell.append(&annotated);
            }
            button.set_child(Some(&cell));
            button.connect_clicked({
                let controller = self.clone();
                let window = window.clone();
                move |_| {
                    controller.state.borrow_mut().goto(index);
                    controller.refresh();
                    window.close();
                }
            });
            flow.insert(&button, -1);
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&flow));
        window.set_child(Some(&scroll));
        window.present();
    }

    fn attach_input(
        &self,
        window: &gtk::ApplicationWindow,
        role: PresentationRole,
        hud: Option<&gtk::Revealer>,
    ) {
        let base_title = window.title().unwrap_or_default().to_string();
        let hud = hud.cloned();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let controller = self.clone();
            let window = window.clone();
            let base_title = base_title.clone();
            let hud = hud.clone();
            move |_, key, _, modifiers| {
                if let Some(hud) = &hud {
                    controller.poke_hud(hud);
                }
                if modifiers.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK
                        | gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::META_MASK
                        | gtk::gdk::ModifierType::SUPER_MASK,
                ) || gtk::prelude::GtkWindowExt::focus(&window)
                    .is_some_and(|widget| widget.is::<gtk::TextView>())
                {
                    return glib::Propagation::Proceed;
                }
                let name = key.name().map(|name| name.to_string()).unwrap_or_default();
                if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
                    && (name == "space" || name == "Return")
                {
                    controller.previous();
                    return glib::Propagation::Stop;
                }
                if name.len() == 1 && name.as_bytes()[0].is_ascii_digit() {
                    controller
                        .number_buffer
                        .borrow_mut()
                        .push_digit(name.chars().next().unwrap_or('0'));
                    if let Some(timeout) = controller.number_timeout.borrow_mut().take() {
                        timeout.remove();
                    }
                    window.set_title(Some(&format!(
                        "Go to slide {} — typsmthng",
                        controller.number_buffer.borrow().display()
                    )));
                    let number_buffer = controller.number_buffer.clone();
                    let number_timeout = controller.number_timeout.clone();
                    let number_timeout_callback = number_timeout.clone();
                    let window = window.clone();
                    let base_title = base_title.clone();
                    let timeout =
                        glib::timeout_add_local_once(Duration::from_millis(1800), move || {
                            number_buffer.borrow_mut().clear();
                            window.set_title(Some(&base_title));
                            number_timeout_callback.replace(None);
                        });
                    number_timeout.replace(Some(timeout));
                    return glib::Propagation::Stop;
                }
                if name == "Return" && !controller.number_buffer.borrow().display().is_empty() {
                    if let Some(timeout) = controller.number_timeout.borrow_mut().take() {
                        timeout.remove();
                    }
                    if let Some(slide) = controller
                        .number_buffer
                        .borrow_mut()
                        .commit(controller.state.borrow().slide_count)
                    {
                        controller.state.borrow_mut().goto(slide);
                        controller.refresh();
                    }
                    window.set_title(Some(&base_title));
                    return glib::Propagation::Stop;
                }
                let Some(command) = presentation_command_for_key(&name) else {
                    return glib::Propagation::Proceed;
                };
                controller.apply_command(command, &window, role);
                glib::Propagation::Stop
            }
        });
        window.add_controller(keys);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll({
            let controller = self.clone();
            let window = window.clone();
            move |_, _, dy| {
                if gtk::prelude::GtkWindowExt::focus(&window)
                    .is_some_and(|widget| widget.is::<gtk::TextView>())
                {
                    return glib::Propagation::Proceed;
                }
                let now = std::time::Instant::now();
                if controller
                    .last_scroll_navigation
                    .get()
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(350))
                {
                    return glib::Propagation::Stop;
                }
                let accumulated = controller.scroll_accumulator.get() + dy;
                controller.scroll_accumulator.set(accumulated);
                if accumulated.abs() >= 1.0 {
                    if accumulated > 0.0 {
                        controller.next();
                    } else {
                        controller.previous();
                    }
                    controller.scroll_accumulator.set(0.0);
                    controller.last_scroll_navigation.set(Some(now));
                }
                glib::Propagation::Stop
            }
        });
        window.add_controller(scroll);

        let click = gtk::GestureClick::new();
        click.connect_released({
            let controller = self.clone();
            move |gesture, _, _, _| {
                if controller.state.borrow().tool != PresentationTool::Pointer {
                    return;
                }
                match gesture.current_button() {
                    1 | 5 => controller.next(),
                    3 | 4 => controller.previous(),
                    _ => {}
                }
            }
        });
        window.add_controller(click);
    }

    fn apply_command(
        &self,
        command: PresentationCommand,
        window: &gtk::ApplicationWindow,
        role: PresentationRole,
    ) {
        let mut state = self.state.borrow_mut();
        match command {
            PresentationCommand::Next => state.next(),
            PresentationCommand::Previous => state.previous(),
            PresentationCommand::First => state.goto(0),
            PresentationCommand::Last => {
                let last = state.slide_count.saturating_sub(1);
                state.goto(last);
            }
            PresentationCommand::ToggleBlack => {
                state.blackout = if state.blackout == Blackout::Black {
                    Blackout::None
                } else {
                    Blackout::Black
                }
            }
            PresentationCommand::ToggleWhite => {
                state.blackout = if state.blackout == Blackout::White {
                    Blackout::None
                } else {
                    Blackout::White
                }
            }
            PresentationCommand::ToggleLaser => {
                state.tool = if state.tool == PresentationTool::Laser {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Laser
                }
            }
            PresentationCommand::TogglePen => {
                state.tool = if state.tool == PresentationTool::Pen {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Pen
                }
            }
            PresentationCommand::ToggleHighlighter => {
                state.tool = if state.tool == PresentationTool::Highlighter {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Highlighter
                }
            }
            PresentationCommand::ToggleEraser => {
                state.tool = if state.tool == PresentationTool::Eraser {
                    PresentationTool::Pointer
                } else {
                    PresentationTool::Eraser
                }
            }
            PresentationCommand::ClearAnnotations => {
                let slide = state.slide;
                if let Some(strokes) = state.strokes.get_mut(slide) {
                    strokes.clear();
                }
            }
            PresentationCommand::ToggleTimer => state.toggle_timer(),
            PresentationCommand::ResetTimer => state.reset_timer(),
            PresentationCommand::ToggleFullscreen => {
                drop(state);
                if role == PresentationRole::Audience {
                    return;
                }
                if window.is_fullscreen() {
                    window.unfullscreen();
                } else {
                    window.fullscreen();
                }
                self.refresh();
                return;
            }
            PresentationCommand::ToggleGrid => {
                drop(state);
                let target = if role == PresentationRole::Audience {
                    self.presenter_window
                        .borrow()
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| window.clone())
                } else {
                    window.clone()
                };
                self.show_grid(&target);
                return;
            }
            PresentationCommand::ToggleNotes => {
                state.notes_visible = !state.notes_visible;
            }
            PresentationCommand::Exit => {
                drop(state);
                self.end();
                return;
            }
        }
        drop(state);
        self.refresh();
    }
}

struct SlideStage {
    root: gtk::Overlay,
    refresh: RefreshCallback,
}

#[derive(Debug, Clone, Copy)]
struct SlideRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn fitted_slide_rect(picture: &gtk::Picture, width: f64, height: f64) -> SlideRect {
    let (intrinsic_width, intrinsic_height) = picture
        .paintable()
        .map(|paintable| {
            (
                paintable.intrinsic_width().max(1) as f64,
                paintable.intrinsic_height().max(1) as f64,
            )
        })
        .unwrap_or((width.max(1.0), height.max(1.0)));
    let content_aspect = intrinsic_width / intrinsic_height;
    let viewport_aspect = width.max(1.0) / height.max(1.0);
    if content_aspect >= viewport_aspect {
        let fitted_height = width / content_aspect;
        SlideRect {
            x: 0.0,
            y: (height - fitted_height) / 2.0,
            width,
            height: fitted_height,
        }
    } else {
        let fitted_width = height * content_aspect;
        SlideRect {
            x: (width - fitted_width) / 2.0,
            y: 0.0,
            width: fitted_width,
            height,
        }
    }
}

fn normalize_to_slide(
    picture: &gtk::Picture,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
) -> Option<NormalizedPoint> {
    let slide = fitted_slide_rect(picture, width.max(1.0), height.max(1.0));
    if x < slide.x || x > slide.x + slide.width || y < slide.y || y > slide.y + slide.height {
        return None;
    }
    Some(NormalizedPoint::new(
        (x - slide.x) / slide.width.max(1.0),
        (y - slide.y) / slide.height.max(1.0),
    ))
}

fn erase_stroke_at(
    state: &Rc<RefCell<PresentationState>>,
    picture: &gtk::Picture,
    drawing: &gtk::DrawingArea,
    x: f64,
    y: f64,
) -> bool {
    let Some(point) = normalize_to_slide(
        picture,
        drawing.width() as f64,
        drawing.height() as f64,
        x,
        y,
    ) else {
        return false;
    };
    let slide = state.borrow().slide;
    let mut state = state.borrow_mut();
    let Some(strokes) = state.strokes.get_mut(slide) else {
        return false;
    };
    let Some((index, _)) = strokes
        .iter()
        .enumerate()
        .filter_map(|(index, stroke)| {
            stroke
                .points
                .iter()
                .map(|candidate| (candidate.x - point.x).hypot(candidate.y - point.y))
                .reduce(f64::min)
                .map(|distance| (index, distance))
        })
        .filter(|(_, distance)| *distance <= 0.05)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
    else {
        return false;
    };
    strokes.remove(index);
    true
}

fn owner_monitor_index(
    owner: &gtk::ApplicationWindow,
    available: &[gtk::gdk::Monitor],
) -> Option<usize> {
    let display = gtk::prelude::WidgetExt::display(owner);
    let surface = owner.surface()?;
    let monitor = display.monitor_at_surface(&surface)?;
    available.iter().position(|candidate| candidate == &monitor)
}

fn default_audience_monitor(count: usize, owner_monitor: Option<usize>) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let owner_monitor = owner_monitor.unwrap_or(0).min(count - 1);
    (0..count)
        .find(|index| *index != owner_monitor)
        .or(Some(owner_monitor))
}

fn tool_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button
}

fn monitors() -> Vec<gtk::gdk::Monitor> {
    let Some(display) = gtk::gdk::Display::default() else {
        return Vec::new();
    };
    let model = display.monitors();
    (0..model.n_items())
        .filter_map(|index| model.item(index)?.downcast::<gtk::gdk::Monitor>().ok())
        .collect()
}

fn monitor_labels() -> Vec<String> {
    let count = monitors().len();
    if count <= 1 {
        vec!["Primary display".into()]
    } else {
        (1..=count)
            .map(|index| format!("Display {index}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::default_audience_monitor;

    #[test]
    fn audience_monitor_avoids_the_presenter_display() {
        assert_eq!(default_audience_monitor(0, None), None);
        assert_eq!(default_audience_monitor(1, Some(0)), Some(0));
        assert_eq!(default_audience_monitor(2, Some(0)), Some(1));
        assert_eq!(default_audience_monitor(2, Some(1)), Some(0));
        assert_eq!(default_audience_monitor(5, Some(3)), Some(0));
    }
}
