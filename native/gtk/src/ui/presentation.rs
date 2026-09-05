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
    refreshers: RefreshList,
    number_buffer: Rc<RefCell<SlideNumberBuffer>>,
    timers: Rc<RefCell<Vec<glib::SourceId>>>,
    number_timeout: Rc<RefCell<Option<glib::SourceId>>>,
    scroll_accumulator: Rc<Cell<f64>>,
    last_scroll_navigation: Rc<Cell<Option<std::time::Instant>>>,
    annotation_color: Rc<Cell<(f64, f64, f64)>>,
    note_font_size: Rc<Cell<u32>>,
    save_note_font_size: Rc<dyn Fn(u32)>,
    save_note: Rc<dyn Fn(usize, String)>,
}

impl PresentationController {
    pub fn new(
        application: &gtk::Application,
        owner: &gtk::ApplicationWindow,
        note_font_size: u32,
        save_note: Rc<dyn Fn(usize, String)>,
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
            refreshers: Rc::new(RefCell::new(Vec::new())),
            number_buffer: Rc::new(RefCell::new(SlideNumberBuffer::default())),
            timers: Rc::new(RefCell::new(Vec::new())),
            number_timeout: Rc::new(RefCell::new(None)),
            scroll_accumulator: Rc::new(Cell::new(0.0)),
            last_scroll_navigation: Rc::new(Cell::new(None)),
            annotation_color: Rc::new(Cell::new((1.0, 0.302, 0.0))),
            note_font_size: Rc::new(Cell::new(note_font_size.clamp(12, 34))),
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
        self.end();
        self.reset_session();
        let window = self.build_stage_window("Presentation — typsmthng", false);
        window.fullscreen();
        window.present();
        self.windows.borrow_mut().push(window);
    }

    pub fn start_presenter(&self) {
        self.end();
        self.reset_session();
        let audience = self.build_stage_window("Audience — typsmthng", true);
        let monitors = monitors();
        if let Some(monitor) = monitors.get(1).or_else(|| monitors.first()) {
            audience.fullscreen_on_monitor(monitor);
        } else {
            audience.fullscreen();
        }
        audience.present();

        let presenter = self.build_presenter_window();
        presenter.present();
        self.windows.borrow_mut().extend([audience, presenter]);
    }

    pub fn end(&self) {
        for timer in self.timers.borrow_mut().drain(..) {
            timer.remove();
        }
        if let Some(timeout) = self.number_timeout.borrow_mut().take() {
            timeout.remove();
        }
        let windows = self.windows.borrow_mut().drain(..).collect::<Vec<_>>();
        for window in windows {
            window.close();
        }
        self.refreshers.borrow_mut().clear();
        self.owner.present();
    }

    fn reset_session(&self) {
        self.state
            .replace(PresentationState::new(self.pages.borrow().len()));
        self.number_buffer.replace(SlideNumberBuffer::default());
        self.scroll_accumulator.set(0.0);
        self.last_scroll_navigation.set(None);
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

    fn build_stage_window(&self, title: &str, minimal: bool) -> gtk::ApplicationWindow {
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title(title)
            .default_width(1280)
            .default_height(720)
            .build();
        window.add_css_class("presentation-shell");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let stage = self.build_slide_stage();
        root.append(&stage.root);
        if !minimal {
            root.append(&self.build_toolbar(&window));
        }
        window.set_child(Some(&root));
        self.attach_input(&window);
        window.connect_close_request({
            let controller = self.clone();
            move |_| {
                controller.end();
                glib::Propagation::Proceed
            }
        });
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
        let note_overlay = gtk::Label::new(None);
        note_overlay.add_css_class("notes");
        note_overlay.set_wrap(true);
        note_overlay.set_halign(gtk::Align::End);
        note_overlay.set_valign(gtk::Align::End);
        note_overlay.set_max_width_chars(48);
        note_overlay.set_margin_bottom(64);
        note_overlay.set_margin_end(28);
        note_overlay.set_visible(false);
        note_overlay.set_can_target(false);
        root.add_overlay(&note_overlay);

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
                    if stroke.points.len() < 2 {
                        continue;
                    }
                    let (r, g, b, a) = stroke.color;
                    cr.set_source_rgba(r, g, b, a);
                    cr.set_line_cap(gtk::cairo::LineCap::Round);
                    cr.set_line_join(gtk::cairo::LineJoin::Round);
                    cr.set_line_width(match stroke.tool {
                        PresentationTool::Highlighter => (slide.width * 0.018).max(10.0),
                        _ => (slide.width * 0.004).max(3.0),
                    });
                    let first = stroke.points[0];
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
                }
            }
        });
        drag.connect_drag_update({
            let pending = pending.clone();
            let drawing = drawing.clone();
            let picture = picture.clone();
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
        let erase = gtk::GestureClick::new();
        erase.set_button(1);
        erase.connect_released({
            let state = self.state.clone();
            let controller = self.clone();
            let picture = picture.clone();
            let drawing = drawing.clone();
            move |_, _, x, y| {
                if state.borrow().tool == PresentationTool::Eraser {
                    let Some(point) = normalize_to_slide(
                        &picture,
                        drawing.width() as f64,
                        drawing.height() as f64,
                        x,
                        y,
                    ) else {
                        return;
                    };
                    let slide = state.borrow().slide;
                    if let Some(strokes) = state.borrow_mut().strokes.get_mut(slide) {
                        if let Some((index, _)) = strokes
                            .iter()
                            .enumerate()
                            .filter_map(|(index, stroke)| {
                                stroke
                                    .points
                                    .iter()
                                    .map(|candidate| {
                                        (candidate.x - point.x).hypot(candidate.y - point.y)
                                    })
                                    .reduce(f64::min)
                                    .map(|distance| (index, distance))
                            })
                            .filter(|(_, distance)| *distance <= 0.05)
                            .min_by(|(_, a), (_, b)| a.total_cmp(b))
                        {
                            strokes.remove(index);
                        }
                    }
                    controller.refresh();
                }
            }
        });
        drawing.add_controller(erase);

        let refresh: Rc<dyn Fn()> = {
            let pages = self.pages.clone();
            let state = self.state.clone();
            let picture = picture.clone();
            let drawing = drawing.clone();
            let blackout = blackout.clone();
            let inline_notes = self.inline_notes.clone();
            let sidecar_notes = self.sidecar_notes.clone();
            let note_overlay = note_overlay.clone();
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
                note_overlay.set_label(&combined_note(
                    &inline_notes.borrow(),
                    &sidecar_notes.borrow(),
                    state.slide,
                ));
                note_overlay.set_visible(state.notes_visible);
            })
        };
        refresh();
        self.refreshers.borrow_mut().push(refresh);
        SlideStage { root }
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
        let pending_note_save = Rc::new(RefCell::new(None::<glib::SourceId>));
        note_buffer.connect_changed({
            let suppress_note_change = suppress_note_change.clone();
            let pending_note_save = pending_note_save.clone();
            let state = self.state.clone();
            let sidecar_notes = self.sidecar_notes.clone();
            let save_note = self.save_note.clone();
            move |buffer| {
                if suppress_note_change.get() {
                    return;
                }
                if let Some(source) = pending_note_save.borrow_mut().take() {
                    source.remove();
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
                let save_note = save_note.clone();
                pending_note_save.replace(Some(glib::timeout_add_local_once(
                    Duration::from_millis(600),
                    move || save_note(slide, text),
                )));
            }
        });
        console.set_end_child(Some(&aside));
        root.append(&console);
        root.append(&self.build_toolbar(&window));
        window.set_child(Some(&root));
        self.attach_input(&window);

        let refresh: Rc<dyn Fn()> = {
            let state = self.state.clone();
            let pages = self.pages.clone();
            let inline_notes = self.inline_notes.clone();
            let sidecar_notes = self.sidecar_notes.clone();
            let rendered_notes = self.rendered_notes.clone();
            let suppress_note_change = suppress_note_change.clone();
            Rc::new(move || {
                let slide = state.borrow().slide;
                if let Some(path) = pages.borrow().get(slide + 1) {
                    next_picture.set_filename(Some(path));
                } else {
                    next_picture.set_paintable(gtk::gdk::Paintable::NONE);
                }
                suppress_note_change.set(true);
                note_buffer.set_text(
                    sidecar_notes
                        .borrow()
                        .get(slide)
                        .map(String::as_str)
                        .unwrap_or_default(),
                );
                suppress_note_change.set(false);
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

    fn build_toolbar(&self, parent: &gtk::ApplicationWindow) -> gtk::Box {
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
        let displays = gtk::DropDown::from_strings(&monitor_labels());
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
        pointer.connect_toggled({
            let state = self.state.clone();
            move |b| {
                if b.is_active() {
                    state.borrow_mut().tool = PresentationTool::Pointer;
                }
            }
        });
        laser.connect_toggled({
            let state = self.state.clone();
            move |b| {
                if b.is_active() {
                    state.borrow_mut().tool = PresentationTool::Laser;
                }
            }
        });
        pen.connect_toggled({
            let state = self.state.clone();
            move |b| {
                if b.is_active() {
                    state.borrow_mut().tool = PresentationTool::Pen;
                }
            }
        });
        highlighter.connect_toggled({
            let state = self.state.clone();
            move |b| {
                if b.is_active() {
                    state.borrow_mut().tool = PresentationTool::Highlighter;
                }
            }
        });
        eraser.connect_toggled({
            let state = self.state.clone();
            move |b| {
                if b.is_active() {
                    state.borrow_mut().tool = PresentationTool::Eraser;
                }
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
        black.connect_toggled({
            let state = self.state.clone();
            let controller = self.clone();
            move |button| {
                state.borrow_mut().blackout = if button.is_active() {
                    Blackout::Black
                } else {
                    Blackout::None
                };
                controller.refresh();
            }
        });
        white.connect_toggled({
            let state = self.state.clone();
            let controller = self.clone();
            move |button| {
                state.borrow_mut().blackout = if button.is_active() {
                    Blackout::White
                } else {
                    Blackout::None
                };
                controller.refresh();
            }
        });
        timer_toggle.connect_clicked({
            let state = self.state.clone();
            move |_| state.borrow_mut().toggle_timer()
        });
        timer_reset.connect_clicked({
            let state = self.state.clone();
            move |_| state.borrow_mut().reset_timer()
        });
        grid.connect_clicked({
            let controller = self.clone();
            let parent = parent.clone();
            move |_| controller.show_grid(&parent)
        });
        displays.connect_selected_notify({
            let windows = self.windows.clone();
            move |chooser| {
                if let (Some(window), Some(monitor)) = (
                    windows.borrow().first(),
                    monitors().get(chooser.selected() as usize),
                ) {
                    window.fullscreen_on_monitor(monitor);
                }
            }
        });
        exit.connect_clicked({
            let controller = self.clone();
            move |_| controller.end()
        });

        let state = self.state.clone();
        let slide_copy = slide.clone();
        let timer_copy = timer.clone();
        let clock_copy = clock.clone();
        let timer_source = glib::timeout_add_local(Duration::from_millis(250), move || {
            let state = state.borrow();
            slide_copy.set_text(&format!(
                "{} / {}",
                (state.slide + 1).min(state.slide_count),
                state.slide_count
            ));
            timer_copy.set_text(&format_elapsed(state.elapsed()));
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
        for (index, page) in self.pages.borrow().iter().enumerate() {
            let button = gtk::Button::new();
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 5);
            let picture = gtk::Picture::for_filename(page);
            picture.set_keep_aspect_ratio(true);
            picture.set_size_request(210, 118);
            cell.append(&picture);
            cell.append(&gtk::Label::new(Some(&(index + 1).to_string())));
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

    fn attach_input(&self, window: &gtk::ApplicationWindow) {
        let base_title = window.title().unwrap_or_default().to_string();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let controller = self.clone();
            let window = window.clone();
            let base_title = base_title.clone();
            move |_, key, _, modifiers| {
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
                controller.apply_command(command, &window);
                glib::Propagation::Stop
            }
        });
        window.add_controller(keys);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll({
            let controller = self.clone();
            move |_, _, dy| {
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

    fn apply_command(&self, command: PresentationCommand, window: &gtk::ApplicationWindow) {
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
                self.show_grid(window);
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

fn combined_note(inline: &[String], sidecar: &[String], slide: usize) -> String {
    let inline = inline.get(slide).map(String::as_str).unwrap_or_default();
    let sidecar = sidecar.get(slide).map(String::as_str).unwrap_or_default();
    match (inline.trim().is_empty(), sidecar.trim().is_empty()) {
        (true, true) => "No notes for this slide.".into(),
        (false, true) => inline.into(),
        (true, false) => sidecar.into(),
        (false, false) => format!("{inline}\n\n{sidecar}"),
    }
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

fn monitor_labels() -> Vec<&'static str> {
    match monitors().len() {
        0 | 1 => vec!["Primary display"],
        2 => vec!["Display 1", "Display 2"],
        3 => vec!["Display 1", "Display 2", "Display 3"],
        _ => vec!["Display 1", "Display 2", "Display 3", "Display 4"],
    }
}
