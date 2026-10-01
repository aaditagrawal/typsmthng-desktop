//! Native completion and plain-text semantic hover. Typst work stays on its worker.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gtk::prelude::*;
use typsmthng_gtk::backend::editor::{
    byte_to_char, char_to_byte, flatten_snippet, EditorQuery, EditorRequest, EditorResponse,
};
use typsmthng_gtk::backend::Result;

type HoverCache = Option<(usize, Option<String>)>;

pub type EditorCallback = Rc<dyn Fn(EditorRequest) -> Option<Receiver<Result<EditorResponse>>>>;

#[derive(Clone)]
pub struct EditorTools {
    path: Rc<RefCell<String>>,
    epoch: Rc<Cell<u64>>,
    popup: gtk::Popover,
    hover: Rc<RefCell<HoverCache>>,
    hover_pending: Rc<Cell<bool>>,
}

struct CompletionSession {
    text: String,
    cursor: usize,
    from: usize,
    items: Vec<typst_ide::Completion>,
}

impl EditorTools {
    pub fn new(
        editor: &sourceview5::View,
        anchor: &gtk::Overlay,
        callback: EditorCallback,
    ) -> Self {
        let popup = gtk::Popover::new();
        popup.set_parent(anchor);
        popup.set_autohide(false);
        popup.set_has_arrow(false);
        popup.set_focusable(false);
        let list = gtk::ListBox::new();
        list.set_focusable(false);
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_width(280);
        scroll.set_max_content_height(280);
        scroll.set_propagate_natural_height(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_child(Some(&list));
        popup.set_child(Some(&scroll));
        let hover = Rc::new(RefCell::new(None::<(usize, Option<String>)>));
        let hover_pending = Rc::new(Cell::new(false));
        let tools = Self {
            path: Rc::new(RefCell::new(String::new())),
            epoch: Rc::new(Cell::new(0)),
            popup,
            hover: hover.clone(),
            hover_pending: hover_pending.clone(),
        };
        let buffer = editor.buffer();
        let session = Rc::new(RefCell::new(None::<CompletionSession>));
        let invalidate: Rc<dyn Fn()> = {
            let tools = tools.clone();
            let session = session.clone();
            Rc::new(move || {
                tools.invalidate();
                session.borrow_mut().take();
            })
        };
        buffer.connect_changed({
            let invalidate = invalidate.clone();
            move |_| invalidate()
        });
        buffer.connect_mark_set(move |_, _, mark| {
            if mark.name().as_deref() == Some("insert") {
                invalidate();
            }
        });
        let accept: Rc<dyn Fn()> = {
            let session = session.clone();
            let list = list.downgrade();
            let popup = tools.popup.downgrade();
            let buffer = buffer.downgrade();
            Rc::new(move || {
                let Some(buffer) = buffer.upgrade() else {
                    return;
                };
                let Some(list) = list.upgrade() else {
                    return;
                };
                let Some(index) = list.selected_row().map(|row| row.index() as usize) else {
                    return;
                };
                let Some(session) = session.borrow_mut().take() else {
                    return;
                };
                if let Some(popup) = popup.upgrade() {
                    popup.popdown();
                }
                if buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .as_str()
                    != session.text
                    || char_to_byte(&session.text, buffer.cursor_position() as usize)
                        != Some(session.cursor)
                {
                    return;
                }
                if let Some(item) = session.items.get(index) {
                    apply_completion(&buffer, &session.text, session.from, session.cursor, item);
                }
            })
        };
        list.connect_row_activated({
            let accept = accept.clone();
            move |_, _| accept()
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let editor = editor.downgrade();
            let tools = tools.clone();
            let callback = callback.clone();
            let list = list.clone();
            move |_, key, _, modifiers| {
                use gtk::gdk::Key;
                if tools.popup.is_visible() {
                    match key {
                        Key::Escape => {
                            tools.invalidate();
                            return glib::Propagation::Stop;
                        }
                        Key::Return | Key::KP_Enter | Key::Tab => {
                            accept();
                            return glib::Propagation::Stop;
                        }
                        Key::Up | Key::Down => {
                            let selected = list.selected_row().map_or(0, |row| row.index());
                            let next = if key == Key::Up {
                                selected.saturating_sub(1)
                            } else {
                                selected + 1
                            };
                            if let Some(row) = list.row_at_index(next) {
                                list.select_row(Some(&row));
                            }
                            return glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
                if key != Key::space || !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                    return glib::Propagation::Proceed;
                }
                let Some(editor) = editor.upgrade().filter(|editor| editor.is_editable()) else {
                    return glib::Propagation::Proceed;
                };
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .to_string();
                let Some(cursor) = char_to_byte(&text, buffer.cursor_position() as usize) else {
                    return glib::Propagation::Stop;
                };
                tools.invalidate();
                let epoch = tools.epoch.get();
                let Some(receiver) = callback(EditorRequest {
                    path: tools.path.borrow().clone(),
                    text: text.clone(),
                    cursor,
                    query: EditorQuery::Complete,
                }) else {
                    return glib::Propagation::Stop;
                };
                let tools = tools.clone();
                let list = list.clone();
                let session = session.clone();
                let editor = editor.downgrade();
                poll(receiver, tools.epoch.clone(), epoch, move |response| {
                    let Some(response) = response else {
                        return;
                    };
                    let Some(editor) = editor.upgrade() else {
                        return;
                    };
                    let EditorResponse::Completions { from, mut items } = response else {
                        return;
                    };
                    let Some(prefix) = text.get(from..cursor) else {
                        return;
                    };
                    let prefix = prefix.to_lowercase();
                    items.retain(|item| item.label.to_lowercase().starts_with(&prefix));
                    items.truncate(64);
                    while let Some(row) = list.first_child() {
                        list.remove(&row);
                    }
                    if items.is_empty() {
                        return;
                    }
                    for item in &items {
                        let label = gtk::Label::new(Some(&item.label));
                        label.set_xalign(0.0);
                        label.set_margin_start(8);
                        label.set_margin_end(8);
                        label.set_margin_top(4);
                        label.set_margin_bottom(4);
                        label.set_tooltip_text(item.detail.as_deref());
                        list.append(&label);
                        if let Some(row) =
                            list.row_at_index((list.observe_children().n_items() - 1) as i32)
                        {
                            row.set_focusable(false);
                        }
                    }
                    list.select_row(list.row_at_index(0).as_ref());
                    let iter = editor.buffer().iter_at_mark(&editor.buffer().get_insert());
                    let rect = editor.iter_location(&iter);
                    let (x, y) = editor.buffer_to_window_coords(
                        gtk::TextWindowType::Widget,
                        rect.x(),
                        rect.y(),
                    );
                    let Some(parent) = tools.popup.parent() else {
                        return;
                    };
                    let Some(point) = editor
                        .compute_point(&parent, &gtk::graphene::Point::new(x as f32, y as f32))
                    else {
                        return;
                    };
                    tools.popup.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                        point.x() as i32,
                        point.y() as i32,
                        rect.width().max(1),
                        rect.height(),
                    )));
                    session.replace(Some(CompletionSession {
                        text,
                        cursor,
                        from,
                        items,
                    }));
                    tools.popup.popup();
                });
                glib::Propagation::Stop
            }
        });
        editor.add_controller(keys);
        editor.set_has_tooltip(true);
        editor.connect_query_tooltip({
            let tools = tools.clone();
            move |editor, x, y, keyboard, tooltip| {
                if keyboard || !editor.is_editable() || !tools.path.borrow().ends_with(".typ") {
                    return false;
                }
                let (x, y) = editor.window_to_buffer_coords(gtk::TextWindowType::Widget, x, y);
                let Some(iter) = editor.iter_at_location(x, y) else {
                    return false;
                };
                let buffer = editor.buffer();
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .to_string();
                let Some(cursor) = char_to_byte(&text, iter.offset() as usize) else {
                    return false;
                };
                if let Some((cached, value)) = hover
                    .borrow()
                    .as_ref()
                    .filter(|(cached, _)| *cached == cursor)
                {
                    let _ = cached;
                    if let Some(value) = value {
                        tooltip.set_text(Some(value));
                        return true;
                    }
                    return false;
                }
                if hover_pending.replace(true) {
                    return false;
                }
                let epoch = tools.epoch.get();
                let Some(receiver) = callback(EditorRequest {
                    path: tools.path.borrow().clone(),
                    text,
                    cursor,
                    query: EditorQuery::Hover,
                }) else {
                    hover_pending.set(false);
                    return false;
                };
                let hover = hover.clone();
                let pending = hover_pending.clone();
                let editor = editor.downgrade();
                poll(receiver, tools.epoch.clone(), epoch, move |response| {
                    pending.set(false);
                    if let Some(EditorResponse::Hover(value)) = response {
                        hover.replace(Some((cursor, value)));
                        if let Some(editor) = editor.upgrade() {
                            editor.trigger_tooltip_query();
                        }
                    }
                });
                false
            }
        });
        anchor.connect_destroy({
            let popup = tools.popup.clone();
            move |_| popup.unparent()
        });
        tools
    }

    pub fn set_file(&self, path: &str) {
        self.invalidate();
        self.path.replace(path.into());
    }
    fn invalidate(&self) {
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.hover.borrow_mut().take();
        self.hover_pending.set(false);
        self.popup.popdown();
    }
}

fn poll(
    receiver: Receiver<Result<EditorResponse>>,
    epoch: Rc<Cell<u64>>,
    expected: u64,
    done: impl FnOnce(Option<EditorResponse>) + 'static,
) {
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(16), move || {
        if epoch.get() != expected {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(response) => {
                done.take().unwrap()(response.ok());
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            _ => {
                done.take().unwrap()(None);
                glib::ControlFlow::Break
            }
        }
    });
}

fn apply_completion(
    buffer: &gtk::TextBuffer,
    text: &str,
    from: usize,
    cursor: usize,
    item: &typst_ide::Completion,
) {
    let Some(start) = byte_to_char(text, from).and_then(|value| i32::try_from(value).ok()) else {
        return;
    };
    let Some(end) = byte_to_char(text, cursor).and_then(|value| i32::try_from(value).ok()) else {
        return;
    };
    let (insert, placeholder) = flatten_snippet(item.apply.as_deref().unwrap_or(&item.label));
    buffer.begin_user_action();
    buffer.delete(
        &mut buffer.iter_at_offset(start),
        &mut buffer.iter_at_offset(end),
    );
    buffer.insert(&mut buffer.iter_at_offset(start), &insert);
    if let Some(range) = placeholder {
        buffer.select_range(
            &buffer.iter_at_offset(start + range.end as i32),
            &buffer.iter_at_offset(start + range.start as i32),
        );
    } else {
        buffer.place_cursor(&buffer.iter_at_offset(start + insert.chars().count() as i32));
    }
    buffer.end_user_action();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires its own GTK process under xvfb-run"]
    fn native_completion_inserts_snippet_once_and_discards_stale_reply() {
        adw::init().unwrap();
        let buffer = sourceview5::Buffer::new(None::<&gtk::TextTagTable>);
        let editor = sourceview5::View::with_buffer(&buffer);
        let reply = Rc::new(RefCell::new(None));
        let anchor = gtk::Overlay::new();
        anchor.set_child(Some(&editor));
        let tools = EditorTools::new(
            &editor,
            &anchor,
            Rc::new({
                let reply = reply.clone();
                move |_| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    reply.replace(Some(tx));
                    Some(rx)
                }
            }),
        );
        tools.set_file("main.typ");
        let window = gtk::Window::new();
        window.set_default_size(640, 480);
        window.set_child(Some(&anchor));
        window.present();
        let item = typst_ide::Completion {
            kind: typst_ide::CompletionKind::Func,
            label: "image".into(),
            apply: Some("image(\"${}\")".into()),
            detail: None,
        };
        buffer.set_text("é🙂 #im");
        buffer.place_cursor(&buffer.end_iter());
        apply_completion(buffer.upcast_ref(), "é🙂 #im", 8, 10, &item);
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
            "é🙂 #image(\"\")"
        );
        assert_eq!(buffer.cursor_position(), 11);
        assert!(buffer.can_undo());
        buffer.undo();
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
            "é🙂 #im"
        );
        let controllers = editor.observe_controllers();
        let key = (0..controllers.n_items())
            .find_map(|index| {
                controllers
                    .item(index)?
                    .downcast::<gtk::EventControllerKey>()
                    .ok()
            })
            .unwrap();
        let trigger = || {
            key.emit_by_name::<bool>(
                "key-pressed",
                &[
                    &gtk::gdk::Key::space,
                    &0_u32,
                    &gtk::gdk::ModifierType::CONTROL_MASK,
                ],
            )
        };
        assert!(trigger());
        let tx = reply.borrow_mut().take().unwrap();
        buffer.insert(&mut buffer.end_iter(), "a");
        let _ = tx.send(Ok(EditorResponse::Completions {
            from: 8,
            items: vec![item.clone()],
        }));
        let context = glib::MainContext::default();
        let drive = || {
            let deadline = std::time::Instant::now() + Duration::from_millis(50);
            while std::time::Instant::now() < deadline {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        drive();
        assert!(!tools.popup.is_visible());
        buffer.set_text("é🙂 #im");
        buffer.place_cursor(&buffer.end_iter());
        editor.grab_focus();
        assert!(trigger());
        reply
            .borrow_mut()
            .take()
            .unwrap()
            .send(Ok(EditorResponse::Completions {
                from: 8,
                items: vec![
                    item.clone(),
                    typst_ide::Completion {
                        label: "image2".into(),
                        ..item.clone()
                    },
                ],
            }))
            .unwrap();
        drive();
        assert!(tools.popup.is_visible());
        assert!(editor.has_focus());
        if let Some(directory) = std::env::var_os("TYPSMTHNG_SNAPSHOT_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let paintable = gtk::WidgetPaintable::new(Some(&tools.popup));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                tools.popup.width() as f64,
                tools.popup.height() as f64,
            );
            let texture = tools
                .popup
                .renderer()
                .unwrap()
                .render_texture(snapshot.to_node().unwrap(), None);
            texture
                .save_to_png(directory.join("completion-popup.png"))
                .unwrap();
            super::super::smoke::capture_windows(&gtk::Application::default());
        }
        let press = |value| {
            key.emit_by_name::<bool>(
                "key-pressed",
                &[&value, &0_u32, &gtk::gdk::ModifierType::empty()],
            )
        };
        assert!(press(gtk::gdk::Key::Down));
        assert!(press(gtk::gdk::Key::Up));
        assert!(press(gtk::gdk::Key::Return));
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
            "é🙂 #image(\"\")"
        );
        buffer.undo();
        assert!(trigger());
        reply
            .borrow_mut()
            .take()
            .unwrap()
            .send(Ok(EditorResponse::Completions {
                from: 8,
                items: vec![item.clone()],
            }))
            .unwrap();
        drive();
        assert!(tools.popup.is_visible());
        assert!(press(gtk::gdk::Key::Escape));
        assert!(!tools.popup.is_visible());
        // Cursor movement invalidates a reply even when the text is unchanged.
        assert!(trigger());
        let tx = reply.borrow_mut().take().unwrap();
        buffer.place_cursor(&buffer.start_iter());
        let _ = tx.send(Ok(EditorResponse::Completions {
            from: 8,
            items: vec![item.clone()],
        }));
        drive();
        assert!(!tools.popup.is_visible());
        // A file switch also discards late completions, even at the same cursor.
        assert!(trigger());
        let tx = reply.borrow_mut().take().unwrap();
        tools.set_file("other.typ");
        let _ = tx.send(Ok(EditorResponse::Completions {
            from: 8,
            items: vec![item],
        }));
        drive();
        assert!(!tools.popup.is_visible());
        window.destroy();
    }
}
