//! A source minimap whose high-level Typst headings are labelled in place,
//! like VS Code's `MARK:` section headers.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use sourceview5::prelude::*;

/// Headings deeper than this are too fine-grained for an overview.
const MAX_LEVEL: usize = 2;
const MAP_WIDTH: i32 = 112;
const LABEL_HEIGHT: f64 = 18.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineHeading {
    /// Zero-based buffer line.
    pub line: usize,
    pub level: usize,
    pub title: String,
}

/// Top-level `=` / `==` headings outside raw blocks and block comments.
pub fn outline_headings(source: &str) -> Vec<OutlineHeading> {
    let mut headings = Vec::new();
    let mut fence: Option<usize> = None;
    let mut comment_depth = 0_usize;
    for (line, text) in source.lines().enumerate() {
        let trimmed = text.trim_start();
        if let Some(width) = fence {
            let ticks = trimmed.chars().take_while(|c| *c == '`').count();
            if ticks >= width {
                fence = None;
            }
            continue;
        }
        let ticks = trimmed.chars().take_while(|c| *c == '`').count();
        if ticks >= 3 {
            // A fence closed on the same line is inline raw, not a block.
            if !trimmed[ticks..].contains(&"`".repeat(ticks)) {
                fence = Some(ticks);
            }
            continue;
        }
        let starts_in_comment = comment_depth > 0;
        comment_depth =
            (comment_depth + text.matches("/*").count()).saturating_sub(text.matches("*/").count());
        if starts_in_comment {
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '=').count();
        let rest = &trimmed[level..];
        if level == 0 || level > MAX_LEVEL || !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let title = rest
            .split(" <")
            .next()
            .unwrap_or_default()
            .split("//")
            .next()
            .unwrap_or_default()
            .trim();
        if !title.is_empty() {
            headings.push(OutlineHeading {
                line,
                level,
                title: title.to_string(),
            });
        }
    }
    headings
}

/// Choose which labels fit without overlapping: level-1 headings claim space
/// first, then subheadings fill the remaining gaps. Returns indices in order.
pub fn place_labels(positions: &[(f64, usize)], label_height: f64, height: f64) -> Vec<usize> {
    let mut placed: Vec<(usize, f64)> = Vec::new();
    for level in 1..=MAX_LEVEL {
        for (index, &(y, candidate_level)) in positions.iter().enumerate() {
            if candidate_level != level || y < -label_height || y > height {
                continue;
            }
            if placed
                .iter()
                .all(|(_, other)| (other - y).abs() >= label_height)
            {
                placed.push((index, y));
            }
        }
    }
    let mut indices = placed
        .into_iter()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices
}

#[derive(Clone)]
pub struct HeadingMinimap {
    root: gtk::Overlay,
}

struct HeadingLabel {
    button: gtk::Button,
    line: Rc<Cell<i32>>,
}

impl HeadingMinimap {
    pub fn new(editor: &sourceview5::View) -> Self {
        let map = sourceview5::Map::new();
        map.set_view(editor);
        map.add_css_class("heading-minimap");
        map.set_size_request(MAP_WIDTH, -1);
        let root = gtk::Overlay::new();
        root.set_child(Some(&map));
        // Each label is its own overlay child: an overlay only receives input
        // inside its allocation, so the rest of the map stays draggable.
        let labels = Rc::new(RefCell::new(Vec::<HeadingLabel>::new()));

        let headings = Rc::new(RefCell::new(Vec::<OutlineHeading>::new()));
        let buffer = editor.buffer();
        let layout = {
            let map = map.clone();
            let root = root.clone();
            let labels = labels.clone();
            let headings = headings.clone();
            let editor = editor.clone();
            Rc::new(move || {
                relabel(
                    &map,
                    &root,
                    &mut labels.borrow_mut(),
                    &headings.borrow(),
                    &editor,
                )
            })
        };
        let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
        let reparse = {
            let buffer = buffer.clone();
            let headings = headings.clone();
            let layout = layout.clone();
            let pending = pending.clone();
            Rc::new(move || {
                if let Some(timer) = pending.borrow_mut().take() {
                    timer.remove();
                }
                let buffer = buffer.clone();
                let headings = headings.clone();
                let layout = layout.clone();
                let done = pending.clone();
                // Parsing is cheap but typing is fast; batch bursts of edits.
                let timer = glib::timeout_add_local_once(Duration::from_millis(150), move || {
                    done.borrow_mut().take();
                    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                    headings.replace(outline_headings(&text));
                    layout();
                });
                pending.replace(Some(timer));
            })
        };
        buffer.connect_changed({
            let reparse = reparse.clone();
            move |_| reparse()
        });
        // A file switch replaces the whole text; relabel even without edits.
        editor.connect_buffer_notify(move |_| reparse());
        let frame_pending = Rc::new(Cell::new(false));
        let schedule = {
            let layout = layout.clone();
            let map = map.clone();
            Rc::new(move || {
                if frame_pending.replace(true) {
                    return;
                }
                let layout = layout.clone();
                let frame_pending = frame_pending.clone();
                map.add_tick_callback(move |_, _| {
                    frame_pending.set(false);
                    layout();
                    glib::ControlFlow::Break
                });
            })
        };
        // The map follows the editor's scroll position and re-lays out when
        // the editor's content height changes; relabel after either.
        let follow = {
            let schedule = schedule.clone();
            move |editor: &sourceview5::View| {
                if let Some(adjustment) = editor.vadjustment() {
                    let on_value = schedule.clone();
                    adjustment.connect_value_changed(move |_| on_value());
                    let on_upper = schedule.clone();
                    adjustment.connect_upper_notify(move |_| on_upper());
                }
            }
        };
        follow(editor);
        editor.connect_vadjustment_notify(follow);
        map.connect_map({
            let schedule = schedule.clone();
            move |_| schedule()
        });
        // GTK 4 has no size-changed signal on ordinary widgets; an inert
        // drawing area stretched over the map reports resizes.
        let sizer = gtk::DrawingArea::new();
        sizer.set_can_target(false);
        sizer.connect_resize(move |_, _, _| schedule());
        root.add_overlay(&sizer);
        Self { root }
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.root
    }
}

fn relabel(
    map: &sourceview5::Map,
    root: &gtk::Overlay,
    labels: &mut Vec<HeadingLabel>,
    headings: &[OutlineHeading],
    editor: &sourceview5::View,
) {
    let buffer = map.buffer();
    let positions = headings
        .iter()
        .map(|heading| {
            let y = buffer
                .iter_at_line(heading.line as i32)
                .map(|iter| {
                    let (top, height) = map.line_yrange(&iter);
                    let (_, y) = map.buffer_to_window_coords(
                        gtk::TextWindowType::Widget,
                        0,
                        top + height / 2,
                    );
                    f64::from(y) - LABEL_HEIGHT / 2.0
                })
                .unwrap_or(f64::NEG_INFINITY);
            (y, heading.level)
        })
        .collect::<Vec<_>>();
    let visible = place_labels(&positions, LABEL_HEIGHT, f64::from(map.height()));
    while labels.len() > visible.len() {
        root.remove_overlay(&labels.pop().unwrap().button);
    }
    for (slot, index) in visible.into_iter().enumerate() {
        let heading = &headings[index];
        if slot == labels.len() {
            let button = gtk::Button::new();
            button.add_css_class("flat");
            button.add_css_class("minimap-heading");
            button.set_halign(gtk::Align::Fill);
            button.set_valign(gtk::Align::Start);
            button.set_margin_end(4);
            button.set_size_request(-1, LABEL_HEIGHT as i32);
            let line = Rc::new(Cell::new(heading.line as i32));
            button.connect_clicked({
                let line = line.clone();
                let editor = editor.downgrade();
                move |_| {
                    let Some(editor) = editor.upgrade() else {
                        return;
                    };
                    let buffer = editor.buffer();
                    if let Some(mut target) = buffer.iter_at_line(line.get()) {
                        buffer.place_cursor(&target);
                        editor.scroll_to_iter(&mut target, 0.0, true, 0.0, 0.1);
                        editor.grab_focus();
                    }
                }
            });
            root.add_overlay(&button);
            labels.push(HeadingLabel { button, line });
        }
        let label = &labels[slot];
        let button = &label.button;
        label.line.set(heading.line as i32);
        // The map's adjustment changes while GTK lays out each edit. Keep
        // existing labels, and avoid triggering another layout for unchanged
        // text/geometry on those notifications.
        if button.label().as_deref() != Some(heading.title.as_str()) {
            button.set_label(&heading.title);
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_xalign(0.0);
            }
        }
        let (class, other) = if heading.level == 1 {
            ("minimap-heading-1", "minimap-heading-2")
        } else {
            ("minimap-heading-2", "minimap-heading-1")
        };
        if !button.has_css_class(class) {
            button.remove_css_class(other);
            button.add_css_class(class);
        }
        let indent = if heading.level == 1 { 4 } else { 12 };
        if button.margin_start() != indent {
            button.set_margin_start(indent);
        }
        let top = positions[index].0.max(0.0).round() as i32;
        if button.margin_top() != top {
            button.set_margin_top(top);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a display; run one exact filter under Xvfb"]
    fn native_minimap_reuses_labels_and_tracks_edited_heading_locations() {
        gtk::init().unwrap();
        sourceview5::init();
        let buffer = sourceview5::Buffer::new(None);
        let source = (0..160)
            .map(|line| {
                if line % 20 == 0 {
                    format!("= Problem {line}\n")
                } else {
                    "Some text and $x^2$ in the document.\n".into()
                }
            })
            .collect::<String>();
        buffer.set_text(&source);
        let editor = sourceview5::View::with_buffer(&buffer);
        let map = sourceview5::Map::new();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&map));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hexpand(true);
        scroll.set_child(Some(&editor));
        // Match the workspace: ScrolledWindow supplies the adjustment before
        // GtkSourceMap connects its scroll handlers to the editor.
        map.set_view(&editor);
        row.append(&scroll);
        row.append(&overlay);
        let window = gtk::Window::builder()
            .default_width(900)
            .default_height(700)
            .child(&row)
            .build();
        window.present();
        let deadline = std::time::Instant::now() + Duration::from_millis(200);
        while std::time::Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut headings = outline_headings(&source);
        let mut labels = Vec::new();
        relabel(&map, &overlay, &mut labels, &headings, &editor);
        assert!(labels.len() >= 2);
        let original = labels
            .iter()
            .map(|label| label.button.clone())
            .collect::<Vec<_>>();
        for _ in 0..20 {
            relabel(&map, &overlay, &mut labels, &headings, &editor);
        }
        assert!(labels
            .iter()
            .zip(&original)
            .all(|(label, button)| label.button == *button));
        headings[0].title = "Updated heading".into();
        headings[0].line = 1;
        headings[0].level = 2;
        relabel(&map, &overlay, &mut labels, &headings, &editor);
        assert_eq!(labels[0].button, original[0]);
        assert_eq!(labels[0].button.label().as_deref(), Some("Updated heading"));
        assert!(labels[0].button.has_css_class("minimap-heading-2"));
        labels[0].button.emit_clicked();
        assert_eq!(buffer.iter_at_mark(&buffer.get_insert()).line(), 1);

        // Compare the old remove/recreate workload with the same label update
        // after reuse. These timings cover only minimap widget work, not Typst
        // compilation, a physical key event, or a rendered frame.
        const REPEATS: u32 = 200;
        let started = std::time::Instant::now();
        for _ in 0..REPEATS {
            for label in labels.drain(..) {
                overlay.remove_overlay(&label.button);
            }
            relabel(&map, &overlay, &mut labels, &headings, &editor);
        }
        let recreate = started.elapsed();
        let started = std::time::Instant::now();
        for _ in 0..REPEATS {
            relabel(&map, &overlay, &mut labels, &headings, &editor);
        }
        let reuse = started.elapsed();
        println!(
            "minimap {REPEATS} relabels, {} visible headings: recreate={recreate:?}, reuse={reuse:?}",
            labels.len()
        );
        headings.truncate(1);
        relabel(&map, &overlay, &mut labels, &headings, &editor);
        assert_eq!(labels.len(), 1);
        headings.clear();
        relabel(&map, &overlay, &mut labels, &headings, &editor);
        assert!(labels.is_empty());
        map.set_property("view", None::<sourceview5::View>);
        window.destroy();
    }

    #[test]
    fn outline_keeps_top_levels_and_skips_raw_and_comments() {
        let source = "= Problem 1 <p1>\n== Part (a) // todo\n=== Detail\n```typ\n= Not a heading\n```\n/* = hidden\n*/\n=Tight\n#let x = 1\n= Problem 2";
        assert_eq!(
            outline_headings(source),
            vec![
                OutlineHeading {
                    line: 0,
                    level: 1,
                    title: "Problem 1".into()
                },
                OutlineHeading {
                    line: 1,
                    level: 2,
                    title: "Part (a)".into()
                },
                OutlineHeading {
                    line: 10,
                    level: 1,
                    title: "Problem 2".into()
                },
            ]
        );
    }

    #[test]
    fn labels_prefer_top_level_headings_when_crowded() {
        // A subheading squeezed between two chapters yields to both.
        let positions = [(0.0, 1), (10.0, 2), (20.0, 1), (60.0, 2), (-40.0, 1)];
        assert_eq!(place_labels(&positions, 18.0, 100.0), vec![0, 2, 3]);
    }
}
