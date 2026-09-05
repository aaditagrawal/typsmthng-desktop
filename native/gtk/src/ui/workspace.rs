use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::prelude::*;
use regex::Regex;
use sourceview5::prelude::*;
use url::Url;

use super::home::icon_button;
use super::model::{SearchMode, Theme, UiSettings};

type DiagnosticLocation = (String, Option<usize>, Option<usize>);

const DEFAULT_PREVIEW_WIDTH: f64 = 560.0;
const DEFAULT_PREVIEW_ASPECT: f64 = 16.0 / 9.0;
const PREVIEW_HORIZONTAL_INSET: i32 = 64;

#[derive(Clone)]
struct PreviewPicture {
    widget: gtk::Picture,
    aspect_ratio: f64,
}

#[derive(Debug, Clone)]
pub struct FileRow {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub is_directory: bool,
    pub is_binary: bool,
    pub is_main: bool,
}

#[derive(Debug, Clone)]
pub struct DiagnosticRow {
    pub severity: DiagnosticKind,
    pub path: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    Error,
    Warning,
    Hint,
}

#[derive(Clone)]
pub struct WorkspaceCallbacks {
    pub go_home: Rc<dyn Fn()>,
    pub open_project: Rc<dyn Fn()>,
    pub save: Rc<dyn Fn(String) -> bool>,
    pub force_save: Rc<dyn Fn(String) -> bool>,
    pub select_file: Rc<dyn Fn(String)>,
    pub create_file: Rc<dyn Fn()>,
    pub create_folder: Rc<dyn Fn()>,
    pub import_files: Rc<dyn Fn()>,
    pub drop_files: Rc<dyn Fn(Vec<PathBuf>)>,
    pub move_path: Rc<dyn Fn((String, String))>,
    pub toggle_hidden: Rc<dyn Fn()>,
    pub rename_path: Rc<dyn Fn(String)>,
    pub duplicate_path: Rc<dyn Fn(String)>,
    pub trash_path: Rc<dyn Fn(String)>,
    pub reveal_path: Rc<dyn Fn(String)>,
    pub open_external: Rc<dyn Fn(String)>,
    pub check_update: Rc<dyn Fn()>,
    pub export_pdf: Rc<dyn Fn()>,
    pub export_project: Rc<dyn Fn()>,
    pub present_single: Rc<dyn Fn()>,
    pub present_dual: Rc<dyn Fn()>,
    pub refresh_compile: Rc<dyn Fn(String)>,
    pub search: Rc<dyn Fn(SearchMode, String) -> Vec<SearchResultRow>>,
    pub settings_changed: Rc<dyn Fn(UiSettings)>,
}

#[derive(Debug, Clone)]
pub struct SearchResultRow {
    pub primary: String,
    pub secondary: String,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Clone)]
pub struct WorkspaceView {
    pub root: gtk::Box,
    pub editor: sourceview5::View,
    pub buffer: sourceview5::Buffer,
    sidebar: gtk::Box,
    file_list: gtk::ListBox,
    file_paths: Rc<RefCell<Vec<String>>>,
    file_rows: Rc<RefCell<Vec<FileRow>>>,
    expanded_directories: Rc<RefCell<HashSet<String>>>,
    known_directories: Rc<RefCell<HashSet<String>>>,
    preview_pages: gtk::Box,
    preview_pictures: Rc<RefCell<Vec<PreviewPicture>>>,
    resize_preview: Rc<dyn Fn()>,
    preview_placeholder: gtk::Box,
    diagnostics_list: gtk::ListBox,
    diagnostic_locations: Rc<RefCell<Vec<DiagnosticLocation>>>,
    diagnostics_revealer: gtk::Revealer,
    conflict_revealer: gtk::Revealer,
    conflict_path: gtk::Label,
    project_label: gtk::Label,
    file_label: gtk::Label,
    save_label: gtk::Label,
    compile_label: gtk::Label,
    page_label: gtk::Label,
    settings: Rc<RefCell<UiSettings>>,
    dirty: Rc<Cell<bool>>,
    suppress_changes: Rc<Cell<bool>>,
    pending_compile: Rc<RefCell<Option<glib::SourceId>>>,
    callbacks: WorkspaceCallbacks,
    vim_context: Rc<RefCell<Option<sourceview5::VimIMContext>>>,
}

impl WorkspaceView {
    pub fn new(window: &gtk::ApplicationWindow, callbacks: WorkspaceCallbacks) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("toolbar");
        let home = icon_button("go-home-symbolic", "Back to vaults");
        let sidebar_toggle = icon_button("sidebar-show-symbolic", "Toggle file tree (Ctrl+\\)");
        let project_label = gtk::Label::new(Some("Vault"));
        project_label.add_css_class("section-title");
        project_label.set_margin_start(6);
        let file_label = gtk::Label::new(Some("No file"));
        file_label.add_css_class("muted");
        file_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        file_label.set_hexpand(true);
        file_label.set_halign(gtk::Align::Start);
        let search = icon_button(
            "system-search-symbolic",
            "Search files, contents, and commands (Ctrl+K)",
        );
        let compile = icon_button("view-refresh-symbolic", "Compile now (Ctrl+Enter)");
        let export = icon_button("document-save-symbolic", "Export PDF (Ctrl+Shift+E)");
        let export_project = icon_button("package-x-generic-symbolic", "Export project ZIP");
        let present = gtk::MenuButton::new();
        present.set_icon_name("media-playback-start-symbolic");
        present.set_tooltip_text(Some("Present (F5)"));
        let settings_button = icon_button("preferences-system-symbolic", "Settings (Ctrl+,)");
        let update_button = icon_button("software-update-available-symbolic", "Check for updates");
        toolbar.append(&home);
        toolbar.append(&sidebar_toggle);
        toolbar.append(&project_label);
        toolbar.append(&gtk::Label::new(Some("/")));
        toolbar.append(&file_label);
        toolbar.append(&search);
        toolbar.append(&compile);
        toolbar.append(&export);
        toolbar.append(&export_project);
        toolbar.append(&present);
        toolbar.append(&settings_button);
        toolbar.append(&update_button);
        root.append(&toolbar);

        let present_popover = gtk::Popover::new();
        let present_menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        present_menu.set_margin_top(6);
        present_menu.set_margin_bottom(6);
        present_menu.set_margin_start(6);
        present_menu.set_margin_end(6);
        let here = gtk::Button::with_label("Present here");
        here.set_tooltip_text(Some("Fullscreen this window"));
        let presenter = gtk::Button::with_label("Presenter + audience");
        presenter.set_tooltip_text(Some("Open an audience window on another display"));
        present_menu.append(&here);
        present_menu.append(&presenter);
        present_popover.set_child(Some(&present_menu));
        present.set_popover(Some(&present_popover));

        let conflict_revealer = gtk::Revealer::new();
        conflict_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        let conflict = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        conflict.add_css_class("conflict");
        let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
        conflict.append(&warning);
        let conflict_copy = gtk::Label::new(Some("External change detected in"));
        conflict.append(&conflict_copy);
        let conflict_path = gtk::Label::new(None);
        conflict_path.add_css_class("mono");
        conflict_path.set_hexpand(true);
        conflict_path.set_halign(gtk::Align::Start);
        conflict.append(&conflict_path);
        let reload = gtk::Button::with_label("Reload from disk");
        let keep = gtk::Button::with_label("Keep editor buffer");
        conflict.append(&reload);
        conflict.append(&keep);
        conflict_revealer.set_child(Some(&conflict));
        root.append(&conflict_revealer);

        let main_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        main_paned.set_vexpand(true);
        main_paned.set_hexpand(true);
        main_paned.set_position(238);
        main_paned.set_shrink_start_child(false);
        root.append(&main_paned);

        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.add_css_class("sidebar");
        sidebar.set_size_request(170, -1);
        let sidebar_header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        sidebar_header.set_margin_top(7);
        sidebar_header.set_margin_bottom(7);
        sidebar_header.set_margin_start(10);
        sidebar_header.set_margin_end(8);
        let files_title = gtk::Label::new(Some("FILES"));
        files_title.add_css_class("eyebrow");
        files_title.set_halign(gtk::Align::Start);
        files_title.set_hexpand(true);
        let new_file = icon_button("document-new-symbolic", "New file");
        let new_folder = icon_button("folder-new-symbolic", "New folder");
        let import_files = icon_button("document-open-symbolic", "Import files into vault");
        let hidden = icon_button("view-reveal-symbolic", "Toggle hidden files");
        sidebar_header.append(&files_title);
        sidebar_header.append(&new_file);
        sidebar_header.append(&new_folder);
        sidebar_header.append(&import_files);
        sidebar_header.append(&hidden);
        sidebar.append(&sidebar_header);
        let file_list = gtk::ListBox::new();
        file_list.set_selection_mode(gtk::SelectionMode::Single);
        file_list.set_activate_on_single_click(true);
        let file_scroll = gtk::ScrolledWindow::new();
        file_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        file_scroll.set_child(Some(&file_list));
        file_scroll.set_vexpand(true);
        sidebar.append(&file_scroll);
        main_paned.set_start_child(Some(&sidebar));

        let work_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        work_paned.set_wide_handle(false);
        work_paned.set_position(590);
        work_paned.set_shrink_start_child(false);
        work_paned.set_shrink_end_child(false);
        main_paned.set_end_child(Some(&work_paned));

        let buffer = sourceview5::Buffer::new(None::<&gtk::TextTagTable>);
        let language_manager = sourceview5::LanguageManager::default();
        for path in language_search_paths() {
            language_manager.append_search_path(path.to_string_lossy().as_ref());
        }
        if let Some(language) = language_manager.language("typst") {
            buffer.set_language(Some(&language));
        }
        buffer.set_highlight_syntax(true);
        buffer.set_highlight_matching_brackets(true);
        let editor = sourceview5::View::with_buffer(&buffer);
        editor.set_monospace(true);
        editor.set_show_line_numbers(true);
        editor.set_show_line_marks(true);
        editor.set_highlight_current_line(true);
        editor.set_auto_indent(true);
        editor.set_smart_backspace(true);
        editor.set_insert_spaces_instead_of_tabs(true);
        editor.set_tab_width(2);
        editor.set_top_margin(14);
        editor.set_bottom_margin(24);
        editor.set_left_margin(10);
        editor.set_right_margin(10);
        editor.add_css_class("editor-pane");
        let vim_context = Rc::new(RefCell::new(None::<sourceview5::VimIMContext>));
        let vim_keys = gtk::EventControllerKey::new();
        vim_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        vim_keys.connect_key_pressed({
            let vim_context = vim_context.clone();
            move |controller, _, _, _| {
                let handled = vim_context.borrow().as_ref().is_some_and(|vim| {
                    controller
                        .current_event()
                        .is_some_and(|event| vim.filter_keypress(&event))
                });
                if handled {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        editor.add_controller(vim_keys);
        let editor_scroll = gtk::ScrolledWindow::new();
        editor_scroll.set_child(Some(&editor));
        editor_scroll.set_hexpand(true);
        editor_scroll.set_vexpand(true);
        work_paned.set_start_child(Some(&editor_scroll));

        let preview_overlay = gtk::Overlay::new();
        preview_overlay.add_css_class("preview-pane");
        let preview_pages = gtk::Box::new(gtk::Orientation::Vertical, 24);
        preview_pages.set_halign(gtk::Align::Center);
        preview_pages.set_margin_top(24);
        preview_pages.set_margin_bottom(48);
        preview_pages.set_margin_start(24);
        preview_pages.set_margin_end(24);
        let preview_scroll = gtk::ScrolledWindow::new();
        preview_scroll.set_child(Some(&preview_pages));
        preview_scroll.set_hexpand(true);
        preview_scroll.set_vexpand(true);
        preview_overlay.set_child(Some(&preview_scroll));

        let preview_placeholder = gtk::Box::new(gtk::Orientation::Vertical, 10);
        preview_placeholder.set_halign(gtk::Align::Center);
        preview_placeholder.set_valign(gtk::Align::Center);
        let preview_glyph = gtk::Image::from_icon_name("document-print-preview-symbolic");
        preview_glyph.set_pixel_size(44);
        preview_placeholder.append(&preview_glyph);
        let preview_title = gtk::Label::new(Some("The rendered pages will gather here"));
        preview_title.add_css_class("section-title");
        preview_placeholder.append(&preview_title);
        let preview_hint = gtk::Label::new(Some("Open a .typ file and compile to begin."));
        preview_hint.add_css_class("muted");
        preview_placeholder.append(&preview_hint);
        preview_overlay.add_overlay(&preview_placeholder);
        let preview_pictures = Rc::new(RefCell::new(Vec::<PreviewPicture>::new()));
        let preview_controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        preview_controls.add_css_class("toolbar");
        preview_controls.set_halign(gtk::Align::End);
        preview_controls.set_valign(gtk::Align::Start);
        preview_controls.set_margin_top(8);
        preview_controls.set_margin_end(10);
        let zoom_out = icon_button("zoom-out-symbolic", "Zoom out");
        let zoom_fit = gtk::Button::with_label("Fit");
        zoom_fit.add_css_class("flat");
        let zoom_in = icon_button("zoom-in-symbolic", "Zoom in");
        let prev_page = icon_button("go-up-symbolic", "Previous page");
        let next_page = icon_button("go-down-symbolic", "Next page");
        preview_controls.append(&zoom_out);
        preview_controls.append(&zoom_fit);
        preview_controls.append(&zoom_in);
        preview_controls.append(&prev_page);
        preview_controls.append(&next_page);
        preview_overlay.add_overlay(&preview_controls);
        let zoom = Rc::new(Cell::new(1.0_f64));
        let fit_to_viewport = Rc::new(Cell::new(true));
        let resize_preview: Rc<dyn Fn()> = {
            let pictures = preview_pictures.clone();
            let zoom = zoom.clone();
            let fit_to_viewport = fit_to_viewport.clone();
            let preview_scroll = preview_scroll.clone();
            Rc::new(move || {
                let viewport_width = preview_scroll.width();
                for picture in pictures.borrow().iter() {
                    let (width, height) = preview_dimensions(
                        viewport_width,
                        zoom.get(),
                        fit_to_viewport.get(),
                        picture.aspect_ratio,
                    );
                    picture.widget.set_size_request(width, height);
                }
            })
        };
        preview_scroll.connect_notify_local(Some("width"), {
            let fit_to_viewport = fit_to_viewport.clone();
            let resize = resize_preview.clone();
            move |_, _| {
                if fit_to_viewport.get() {
                    resize();
                }
            }
        });
        zoom_out.connect_clicked({
            let zoom = zoom.clone();
            let fit_to_viewport = fit_to_viewport.clone();
            let resize = resize_preview.clone();
            move |_| {
                zoom.set((zoom.get() - 0.1).max(0.3));
                fit_to_viewport.set(false);
                resize();
            }
        });
        zoom_in.connect_clicked({
            let zoom = zoom.clone();
            let fit_to_viewport = fit_to_viewport.clone();
            let resize = resize_preview.clone();
            move |_| {
                zoom.set((zoom.get() + 0.1).min(3.0));
                fit_to_viewport.set(false);
                resize();
            }
        });
        zoom_fit.connect_clicked({
            let zoom = zoom.clone();
            let fit_to_viewport = fit_to_viewport.clone();
            let resize = resize_preview.clone();
            move |_| {
                zoom.set(1.0);
                fit_to_viewport.set(true);
                resize();
            }
        });
        prev_page.connect_clicked({
            let scroll = preview_scroll.clone();
            let pictures = preview_pictures.clone();
            move |_| {
                let adjustment = scroll.vadjustment();
                let step = adjustment.upper() / pictures.borrow().len().max(1) as f64;
                adjustment.set_value((adjustment.value() - step).max(adjustment.lower()));
            }
        });
        next_page.connect_clicked({
            let scroll = preview_scroll.clone();
            let pictures = preview_pictures.clone();
            move |_| {
                let adjustment = scroll.vadjustment();
                let step = adjustment.upper() / pictures.borrow().len().max(1) as f64;
                adjustment.set_value(
                    (adjustment.value() + step).min(adjustment.upper() - adjustment.page_size()),
                );
            }
        });
        work_paned.set_end_child(Some(&preview_overlay));

        let diagnostics_revealer = gtk::Revealer::new();
        diagnostics_revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        let diagnostics_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let diagnostics_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        diagnostics_head.set_margin_top(5);
        diagnostics_head.set_margin_bottom(5);
        diagnostics_head.set_margin_start(10);
        diagnostics_head.set_margin_end(8);
        let diagnostics_title = gtk::Label::new(Some("DIAGNOSTICS"));
        diagnostics_title.add_css_class("eyebrow");
        diagnostics_title.set_hexpand(true);
        diagnostics_title.set_halign(gtk::Align::Start);
        let diagnostics_close = icon_button("window-close-symbolic", "Hide diagnostics");
        diagnostics_head.append(&diagnostics_title);
        diagnostics_head.append(&diagnostics_close);
        diagnostics_box.append(&diagnostics_head);
        let diagnostics_list = gtk::ListBox::new();
        diagnostics_list.set_selection_mode(gtk::SelectionMode::Single);
        diagnostics_list.set_activate_on_single_click(true);
        let diagnostic_locations = Rc::new(RefCell::new(Vec::<DiagnosticLocation>::new()));
        let diagnostics_scroll = gtk::ScrolledWindow::new();
        diagnostics_scroll.set_min_content_height(120);
        diagnostics_scroll.set_max_content_height(210);
        diagnostics_scroll.set_propagate_natural_height(true);
        diagnostics_scroll.set_child(Some(&diagnostics_list));
        diagnostics_box.append(&diagnostics_scroll);
        diagnostics_revealer.set_child(Some(&diagnostics_box));
        root.append(&diagnostics_revealer);

        let status = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        status.add_css_class("statusbar");
        let save_label = gtk::Label::new(Some("Ready"));
        save_label.set_halign(gtk::Align::Start);
        let compile_label = gtk::Label::new(Some("No compilation"));
        compile_label.set_halign(gtk::Align::Start);
        compile_label.set_hexpand(true);
        let page_label = gtk::Label::new(Some("0 pages"));
        let cursor_label = gtk::Label::new(Some("Ln 1, Col 1"));
        status.append(&save_label);
        status.append(&compile_label);
        status.append(&page_label);
        status.append(&cursor_label);
        root.append(&status);

        let settings = Rc::new(RefCell::new(UiSettings::default()));
        let dirty = Rc::new(Cell::new(false));
        let suppress_changes = Rc::new(Cell::new(false));
        let file_paths = Rc::new(RefCell::new(Vec::<String>::new()));
        let file_rows = Rc::new(RefCell::new(Vec::<FileRow>::new()));
        let expanded_directories = Rc::new(RefCell::new(HashSet::<String>::new()));
        let known_directories = Rc::new(RefCell::new(HashSet::<String>::new()));

        let settings_dialog =
            build_settings_dialog(window, settings.clone(), callbacks.settings_changed.clone());
        let search_dialog = build_search_dialog(
            window,
            callbacks.search.clone(),
            callbacks.select_file.clone(),
            buffer.clone(),
            editor.clone(),
        );

        let pending_compile = Rc::new(RefCell::new(None::<glib::SourceId>));
        {
            let callback = callbacks.go_home.clone();
            home.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.open_project.clone();
            project_label.add_controller(click_controller(move || callback()));
        }
        {
            let sidebar = sidebar.clone();
            sidebar_toggle.connect_clicked(move |_| sidebar.set_visible(!sidebar.is_visible()));
        }
        {
            let callback = callbacks.create_file.clone();
            new_file.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.create_folder.clone();
            new_folder.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.import_files.clone();
            import_files.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.toggle_hidden.clone();
            hidden.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.check_update.clone();
            update_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.export_pdf.clone();
            export.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.export_project.clone();
            export_project.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.present_single.clone();
            here.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.present_dual.clone();
            presenter.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.refresh_compile.clone();
            let callback_buffer = buffer.clone();
            compile.connect_clicked(move |_| callback(buffer_text(&callback_buffer)));
        }
        {
            let dialog = settings_dialog.clone();
            settings_button.connect_clicked(move |_| dialog.present());
        }
        {
            let dialog = search_dialog.clone();
            search.connect_clicked(move |_| dialog.present());
        }
        diagnostics_close.connect_clicked({
            let diagnostics_revealer = diagnostics_revealer.clone();
            move |_| diagnostics_revealer.set_reveal_child(false)
        });
        keep.connect_clicked({
            let conflict_revealer = conflict_revealer.clone();
            let force_save = callbacks.force_save.clone();
            let buffer = buffer.clone();
            move |_| {
                if force_save(buffer_text(&buffer)) {
                    conflict_revealer.set_reveal_child(false);
                }
            }
        });
        reload.connect_clicked({
            let conflict_revealer = conflict_revealer.clone();
            let callback = callbacks.select_file.clone();
            let conflict_path = conflict_path.clone();
            move |_| {
                callback(conflict_path.text().to_string());
                conflict_revealer.set_reveal_child(false);
            }
        });
        diagnostics_list.connect_row_activated({
            let locations = diagnostic_locations.clone();
            let select = callbacks.select_file.clone();
            let buffer = buffer.clone();
            let editor = editor.clone();
            move |_, row| {
                let Some((path, line, column)) =
                    locations.borrow().get(row.index() as usize).cloned()
                else {
                    return;
                };
                select(path);
                if let Some(mut target) =
                    line.and_then(|line| buffer.iter_at_line(line.saturating_sub(1) as i32))
                {
                    target.forward_chars(column.unwrap_or(1).saturating_sub(1) as i32);
                    buffer.place_cursor(&target);
                    editor.scroll_to_iter(&mut target, 0.15, false, 0.0, 0.25);
                }
            }
        });
        let drop_target = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        drop_target.connect_drop({
            let callback = callbacks.drop_files.clone();
            move |_, value, _, _| {
                let Ok(files) = value.get::<gtk::gdk::FileList>() else {
                    return false;
                };
                let paths = files
                    .files()
                    .into_iter()
                    .filter_map(|file| file.path())
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    false
                } else {
                    callback(paths);
                    true
                }
            }
        });
        root.add_controller(drop_target);
        let internal_drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        internal_drop.connect_drop({
            let callback = callbacks.move_path.clone();
            move |_, value, _, _| {
                value
                    .get::<String>()
                    .map(|source| {
                        callback((source, String::new()));
                        true
                    })
                    .unwrap_or(false)
            }
        });
        file_list.add_controller(internal_drop);
        {
            let callback = callbacks.select_file.clone();
            let paths = file_paths.clone();
            file_list.connect_row_activated(move |_, row| {
                if let Some(path) = paths.borrow().get(row.index() as usize) {
                    callback(path.clone());
                }
            });
        }
        {
            let dirty = dirty.clone();
            let suppress_changes = suppress_changes.clone();
            let save_label = save_label.clone();
            let callback_buffer = buffer.clone();
            let callbacks = callbacks.clone();
            let settings = settings.clone();
            let pending = pending_compile.clone();
            buffer.connect_changed(move |_| {
                if !suppress_changes.get() {
                    dirty.set(true);
                    save_label.set_text("Unsaved changes");
                    if let Some(source) = pending.borrow_mut().take() {
                        source.remove();
                    }
                    let live_compile = settings.borrow().auto_compile;
                    let buffer = callback_buffer.clone();
                    let callbacks = callbacks.clone();
                    let delay = settings.borrow().compile_delay_ms.max(50) as u64;
                    let id = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(delay),
                        move || {
                            let text = buffer_text(&buffer);
                            (callbacks.save)(text.clone());
                            if live_compile {
                                (callbacks.refresh_compile)(text);
                            }
                        },
                    );
                    pending.replace(Some(id));
                }
            });
        }
        {
            let cursor_label = cursor_label.clone();
            buffer.connect_mark_set(move |buffer, iter, mark| {
                if mark.name().as_deref() == Some("insert") {
                    cursor_label.set_text(&format!(
                        "Ln {}, Col {}",
                        iter.line() + 1,
                        iter.line_offset() + 1
                    ));
                }
                let _ = buffer;
            });
        }

        Self {
            root,
            editor,
            buffer,
            sidebar,
            file_list,
            file_paths,
            file_rows,
            expanded_directories,
            known_directories,
            preview_pages,
            preview_pictures,
            resize_preview,
            preview_placeholder,
            diagnostics_list,
            diagnostic_locations,
            diagnostics_revealer,
            conflict_revealer,
            conflict_path,
            project_label,
            file_label,
            save_label,
            compile_label,
            page_label,
            settings,
            dirty,
            suppress_changes,
            pending_compile,
            callbacks,
            vim_context,
        }
    }

    pub fn set_project(&self, name: &str, files: &[FileRow]) {
        self.project_label.set_text(name);
        while let Some(child) = self.file_list.first_child() {
            self.file_list.remove(&child);
        }
        self.file_paths.borrow_mut().clear();
        self.file_rows.replace(files.to_vec());
        let directories = files
            .iter()
            .filter(|file| file.is_directory)
            .map(|file| file.path.clone())
            .collect::<HashSet<_>>();
        self.expanded_directories
            .borrow_mut()
            .retain(|path| directories.contains(path));
        {
            let mut known = self.known_directories.borrow_mut();
            known.retain(|path| directories.contains(path));
            for directory in &directories {
                if known.insert(directory.clone()) {
                    self.expanded_directories
                        .borrow_mut()
                        .insert(directory.clone());
                }
            }
        }
        for file in files {
            self.file_paths.borrow_mut().push(file.path.clone());
            let row = gtk::ListBoxRow::new();
            row.set_activatable(!file.is_directory);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.set_margin_top(4);
            line.set_margin_bottom(4);
            line.set_margin_start(8 + (file.depth as i32 * 12));
            line.set_margin_end(7);
            if file.is_directory {
                let disclosure = gtk::Button::with_label(
                    if self.expanded_directories.borrow().contains(&file.path) {
                        "▾"
                    } else {
                        "▸"
                    },
                );
                disclosure.add_css_class("flat");
                disclosure.set_tooltip_text(Some("Expand or collapse folder"));
                disclosure.connect_clicked({
                    let path = file.path.clone();
                    let expanded = self.expanded_directories.clone();
                    let rows = self.file_rows.clone();
                    let list = self.file_list.clone();
                    move |button| {
                        let mut expanded = expanded.borrow_mut();
                        if !expanded.remove(&path) {
                            expanded.insert(path.clone());
                        }
                        button.set_label(if expanded.contains(&path) {
                            "▾"
                        } else {
                            "▸"
                        });
                        refresh_file_visibility(&list, &rows.borrow(), &expanded);
                    }
                });
                line.append(&disclosure);
            }
            let icon_name = if file.is_directory {
                "folder-symbolic"
            } else if file.is_binary {
                "image-x-generic-symbolic"
            } else {
                "text-x-generic-symbolic"
            };
            line.append(&gtk::Image::from_icon_name(icon_name));
            let name = gtk::Label::new(Some(&file.name));
            name.set_halign(gtk::Align::Start);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            if file.is_main {
                name.add_css_class("document-spine");
                name.set_tooltip_text(Some("Main Typst file"));
            }
            line.append(&name);
            let more = gtk::MenuButton::new();
            more.set_icon_name("view-more-symbolic");
            more.add_css_class("flat");
            let popover = gtk::Popover::new();
            let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
            menu.set_margin_top(4);
            menu.set_margin_bottom(4);
            menu.set_margin_start(4);
            menu.set_margin_end(4);
            let mut actions = vec![
                ("Rename…", self.callbacks.rename_path.clone()),
                ("Reveal in file manager", self.callbacks.reveal_path.clone()),
                (
                    "Open with default app",
                    self.callbacks.open_external.clone(),
                ),
                ("Move to trash", self.callbacks.trash_path.clone()),
            ];
            if !file.is_directory {
                actions.insert(1, ("Duplicate", self.callbacks.duplicate_path.clone()));
            }
            for (label, callback) in actions {
                let button = gtk::Button::with_label(label);
                if label == "Move to trash" {
                    button.add_css_class("destructive-action");
                }
                let path = file.path.clone();
                button.connect_clicked(move |_| callback(path.clone()));
                menu.append(&button);
            }
            popover.set_child(Some(&menu));
            more.set_popover(Some(&popover));
            line.append(&more);
            let drag = gtk::DragSource::new();
            drag.set_actions(gtk::gdk::DragAction::MOVE);
            drag.connect_prepare({
                let path = file.path.clone();
                move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&path.to_value()))
            });
            line.add_controller(drag);
            if file.is_directory {
                let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
                drop.connect_drop({
                    let target = file.path.clone();
                    let callback = self.callbacks.move_path.clone();
                    move |_, value, _, _| {
                        value
                            .get::<String>()
                            .map(|source| {
                                callback((source, target.clone()));
                                true
                            })
                            .unwrap_or(false)
                    }
                });
                line.add_controller(drop);
            }
            row.set_child(Some(&line));
            self.file_list.append(&row);
        }
        refresh_file_visibility(&self.file_list, files, &self.expanded_directories.borrow());
    }

    pub fn show_text_file(&self, path: &str, contents: &str) {
        self.cancel_pending_compile();
        self.suppress_changes.set(true);
        self.buffer.set_text(contents);
        self.buffer.set_modified(false);
        self.suppress_changes.set(false);
        self.dirty.set(false);
        self.file_label.set_text(path);
        self.save_label.set_text("Saved");
        self.editor.set_editable(true);
        self.editor.grab_focus();
    }

    pub fn show_binary_file(&self, path: &Path) {
        self.cancel_pending_compile();
        self.file_label.set_text(path.to_string_lossy().as_ref());
        self.editor.set_editable(false);
        self.suppress_changes.set(true);
        self.buffer.set_text("Binary image preview");
        self.buffer.set_modified(false);
        self.suppress_changes.set(false);
        self.dirty.set(false);
        self.save_label.set_text("Read-only asset");
        self.set_preview_files(&[path.to_path_buf()]);
    }

    pub fn show_missing_file(&self, path: &str) {
        self.cancel_pending_compile();
        self.file_label.set_text(path);
        self.editor.set_editable(false);
        self.suppress_changes.set(true);
        self.buffer
            .set_text("This file was removed outside typsmthng.");
        self.buffer.set_modified(false);
        self.suppress_changes.set(false);
        self.dirty.set(false);
        self.save_label.set_text("File removed");
    }

    pub fn source_text(&self) -> String {
        buffer_text(&self.buffer)
    }

    pub fn mark_saved(&self) {
        self.buffer.set_modified(false);
        self.dirty.set(false);
        self.save_label.set_text("Saved");
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    pub fn request_save(&self) {
        if self.editor.is_editable() {
            (self.callbacks.save)(self.source_text());
        }
    }

    pub fn request_compile(&self) {
        if self.editor.is_editable() {
            (self.callbacks.refresh_compile)(self.source_text());
        }
    }

    pub fn save_before_navigation(&self) -> bool {
        self.cancel_pending_compile();
        if self.dirty.get() && self.editor.is_editable() {
            return (self.callbacks.save)(self.source_text());
        }
        true
    }

    fn cancel_pending_compile(&self) {
        if let Some(source) = self.pending_compile.borrow_mut().take() {
            source.remove();
        }
    }

    pub fn present_settings(&self) {
        for child in gtk::Window::list_toplevels() {
            if let Ok(candidate) = child.downcast::<gtk::Window>() {
                if candidate.title().as_deref() == Some("Settings — typsmthng") {
                    candidate.present();
                    break;
                }
            }
        }
    }

    pub fn present_search(&self) {
        for child in gtk::Window::list_toplevels() {
            if let Ok(candidate) = child.downcast::<gtk::Window>() {
                if candidate.title().as_deref() == Some("Search — typsmthng") {
                    candidate.present();
                    break;
                }
            }
        }
    }

    pub fn toggle_sidebar(&self) {
        self.sidebar.set_visible(!self.sidebar.is_visible());
    }

    pub fn toggle_comment(&self) {
        if !self.editor.is_editable() {
            return;
        }
        let (start, end) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (cursor, cursor)
        });
        let start_line = start.line();
        let end_line = if end.offset() > start.offset() && end.line_offset() == 0 {
            end.line().saturating_sub(1)
        } else {
            end.line()
        };
        let lines = (start_line..=end_line)
            .filter_map(|line| {
                let begin = self.buffer.iter_at_line(line)?;
                let mut finish = begin;
                finish.forward_to_line_end();
                Some(self.buffer.text(&begin, &finish, false).to_string())
            })
            .collect::<Vec<_>>();
        let uncomment = should_uncomment_lines(&lines);
        self.buffer.begin_user_action();
        for line in (start_line..=end_line).rev() {
            let Some(mut begin) = self.buffer.iter_at_line(line) else {
                continue;
            };
            let mut finish = begin;
            finish.forward_to_line_end();
            let text = self.buffer.text(&begin, &finish, false).to_string();
            let trimmed = text.trim_start();
            let indent_bytes = text.len() - trimmed.len();
            let indent_chars = text[..indent_bytes].chars().count();
            if text.trim().is_empty() {
                continue;
            }
            begin.forward_chars(indent_chars as i32);
            if uncomment {
                let remainder = &text[indent_bytes..];
                let remove = if remainder.starts_with("/// ") {
                    4
                } else if remainder.starts_with("///") || remainder.starts_with("// ") {
                    3
                } else if remainder.starts_with("//") {
                    2
                } else {
                    0
                };
                if remove > 0 {
                    let mut comment_end = begin;
                    comment_end.forward_chars(remove);
                    self.buffer.delete(&mut begin, &mut comment_end);
                }
            } else {
                self.buffer.insert(&mut begin, "// ");
            }
        }
        self.buffer.end_user_action();
    }

    pub fn duplicate_lines(&self) {
        if !self.editor.is_editable() {
            return;
        }
        let (start, end) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (cursor, cursor)
        });
        let first_line = start.line();
        let last_line = if end.offset() > start.offset() && end.line_offset() == 0 {
            end.line().saturating_sub(1)
        } else {
            end.line()
        };
        let Some(begin) = self.buffer.iter_at_line(first_line) else {
            return;
        };
        let mut finish = self
            .buffer
            .iter_at_line(last_line + 1)
            .unwrap_or_else(|| self.buffer.end_iter());
        let text = self.buffer.text(&begin, &finish, true).to_string();
        if !text.ends_with('\n') {
            finish = self.buffer.end_iter();
            self.buffer.insert(&mut finish, "\n");
        }
        self.buffer.insert(&mut finish, &text);
    }

    pub fn set_compiling(&self) {
        self.compile_label.set_text("Compiling…");
    }

    pub fn set_compile_status(&self, text: &str) {
        self.compile_label.set_text(text);
    }

    pub fn set_preview_files(&self, pages: &[PathBuf]) {
        self.set_preview_content(pages, None);
    }

    pub fn set_compiled_preview(
        &self,
        pages: &[PathBuf],
        source_path: &str,
        source_line_count: usize,
    ) {
        self.set_preview_content(pages, Some((source_path, source_line_count.max(1))));
    }

    fn set_preview_content(&self, pages: &[PathBuf], source: Option<(&str, usize)>) {
        while let Some(child) = self.preview_pages.first_child() {
            self.preview_pages.remove(&child);
        }
        self.preview_pictures.borrow_mut().clear();
        self.preview_placeholder.set_visible(pages.is_empty());
        self.page_label.set_text(&format!(
            "{} {}",
            pages.len(),
            if pages.len() == 1 { "page" } else { "pages" }
        ));
        for (index, page) in pages.iter().enumerate() {
            let svg = std::fs::read_to_string(page).ok();
            let aspect_ratio = svg
                .as_deref()
                .and_then(svg_aspect_ratio)
                .unwrap_or(DEFAULT_PREVIEW_ASPECT);
            let links = svg
                .as_deref()
                .map(extract_external_links)
                .unwrap_or_default();
            let sheet = gtk::Box::new(gtk::Orientation::Vertical, 8);
            sheet.add_css_class("page-sheet");
            if let Some((source_path, source_line_count)) = source {
                let source_line =
                    source_line_for_page_position(index, pages.len(), 0.5, source_line_count);
                let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                header.set_margin_start(8);
                header.set_margin_end(8);
                let page_number = gtk::Label::new(Some(&format!("Page {}", index + 1)));
                page_number.add_css_class("muted");
                page_number.set_halign(gtk::Align::Start);
                page_number.set_hexpand(true);
                let edit_source =
                    gtk::Button::with_label(&format!("Edit source · line {source_line}"));
                edit_source.add_css_class("flat");
                edit_source.set_tooltip_text(Some(
                    "Open the compiled source near this page. Double-click the page for a more precise position.",
                ));
                edit_source.connect_clicked({
                    let select = self.callbacks.select_file.clone();
                    let source_path = source_path.to_string();
                    let buffer = self.buffer.clone();
                    let editor = self.editor.clone();
                    move |_| {
                        focus_source_line(&select, &source_path, source_line, &buffer, &editor);
                    }
                });
                header.append(&page_number);
                header.append(&edit_source);
                sheet.append(&header);
            }
            let picture = gtk::Picture::for_filename(page);
            picture.set_can_shrink(true);
            picture.set_keep_aspect_ratio(true);
            picture.set_tooltip_text(Some(if source.is_some() {
                "Double-click to edit source near this position"
            } else {
                "Preview"
            }));
            if let Some((source_path, source_line_count)) = source {
                let edit = gtk::GestureClick::new();
                edit.set_button(1);
                edit.connect_released({
                    let select = self.callbacks.select_file.clone();
                    let source_path = source_path.to_string();
                    let buffer = self.buffer.clone();
                    let editor = self.editor.clone();
                    let picture = picture.clone();
                    let page_count = pages.len();
                    move |_, press_count, _, y| {
                        if press_count != 2 {
                            return;
                        }
                        let within_page = if picture.height() > 0 {
                            y / f64::from(picture.height())
                        } else {
                            0.5
                        };
                        let line = source_line_for_page_position(
                            index,
                            page_count,
                            within_page,
                            source_line_count,
                        );
                        focus_source_line(&select, &source_path, line, &buffer, &editor);
                    }
                });
                picture.add_controller(edit);
            }
            sheet.append(&picture);
            if !links.is_empty() {
                let links_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
                links_box.set_margin_start(8);
                links_box.set_margin_end(8);
                let heading = gtk::Label::new(Some(if links.len() == 1 {
                    "LINK ON THIS PAGE"
                } else {
                    "LINKS ON THIS PAGE"
                }));
                heading.add_css_class("eyebrow");
                heading.set_halign(gtk::Align::Start);
                links_box.append(&heading);
                for uri in links {
                    let link = gtk::LinkButton::with_label(&uri, &external_link_label(&uri));
                    link.set_halign(gtk::Align::Start);
                    link.set_tooltip_text(Some(&uri));
                    links_box.append(&link);
                }
                sheet.append(&links_box);
            }
            self.preview_pictures.borrow_mut().push(PreviewPicture {
                widget: picture,
                aspect_ratio,
            });
            self.preview_pages.append(&sheet);
        }
        (self.resize_preview)();
        let resize = self.resize_preview.clone();
        glib::idle_add_local_once(move || resize());
    }

    pub fn set_diagnostics(&self, diagnostics: &[DiagnosticRow]) {
        while let Some(child) = self.diagnostics_list.first_child() {
            self.diagnostics_list.remove(&child);
        }
        self.diagnostics_revealer
            .set_reveal_child(!diagnostics.is_empty());
        self.diagnostic_locations.borrow_mut().clear();
        for diagnostic in diagnostics {
            self.diagnostic_locations.borrow_mut().push((
                diagnostic.path.clone(),
                diagnostic.line,
                diagnostic.column,
            ));
            let row = gtk::ListBoxRow::new();
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.set_margin_top(5);
            line.set_margin_bottom(5);
            line.set_margin_start(10);
            line.set_margin_end(10);
            let mark = gtk::Label::new(Some(match diagnostic.severity {
                DiagnosticKind::Error => "● ERROR",
                DiagnosticKind::Warning => "▲ WARN",
                DiagnosticKind::Hint => "◆ HINT",
            }));
            mark.add_css_class("eyebrow");
            mark.add_css_class(match diagnostic.severity {
                DiagnosticKind::Error => "diagnostic-error",
                DiagnosticKind::Warning => "diagnostic-warning",
                DiagnosticKind::Hint => "muted",
            });
            line.append(&mark);
            let location = match (diagnostic.line, diagnostic.column) {
                (Some(line), Some(column)) => format!("{}:{line}:{column}", diagnostic.path),
                _ => diagnostic.path.clone(),
            };
            let path = gtk::Label::new(Some(&location));
            path.add_css_class("mono");
            line.append(&path);
            let message = gtk::Label::new(Some(&diagnostic.message));
            message.set_halign(gtk::Align::Start);
            message.set_hexpand(true);
            message.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&message);
            row.set_child(Some(&line));
            self.diagnostics_list.append(&row);
        }
    }

    pub fn show_conflict(&self, path: &str) {
        self.conflict_path.set_text(path);
        self.conflict_revealer.set_reveal_child(true);
    }

    pub fn apply_settings(&self, settings: UiSettings) {
        self.editor.set_show_line_numbers(settings.line_numbers);
        self.editor.set_wrap_mode(if settings.line_wrapping {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        });
        let provider = gtk::CssProvider::new();
        provider.load_from_data(&format!(
            ".typst-editor {{ font-size: {}pt; }}",
            settings.font_size
        ));
        self.editor.add_css_class("typst-editor");
        self.editor
            .style_context()
            .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        if settings.vim_mode {
            let vim = sourceview5::VimIMContext::new();
            vim.set_client_widget(Some(&self.editor));
            self.vim_context.replace(Some(vim));
        } else {
            self.vim_context.replace(None);
        }
        self.settings.replace(settings);
    }
}

fn buffer_text(buffer: &sourceview5::Buffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn refresh_file_visibility(list: &gtk::ListBox, files: &[FileRow], expanded: &HashSet<String>) {
    for (index, file) in files.iter().enumerate() {
        let visible = ancestor_directories(&file.path).all(|parent| expanded.contains(parent));
        if let Some(row) = list.row_at_index(index as i32) {
            row.set_visible(visible);
        }
    }
}

fn ancestor_directories(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(|(index, _)| &path[..index])
}

fn language_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            paths.push(directory.join("../Resources/language-specs"));
            paths.push(directory.join("../share/typsmthng/language-specs"));
            paths.push(directory.join("share/typsmthng/language-specs"));
        }
    }
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/language-specs"));
    paths
}

fn click_controller(action: impl Fn() + 'static) -> gtk::GestureClick {
    let gesture = gtk::GestureClick::new();
    gesture.connect_released(move |_, _, _, _| action());
    gesture
}

fn should_uncomment_lines(lines: &[String]) -> bool {
    let mut content = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .peekable();
    content.peek().is_some()
        && content.all(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("//")
        })
}

fn preview_dimensions(
    viewport_width: i32,
    zoom: f64,
    fit_to_viewport: bool,
    aspect_ratio: f64,
) -> (i32, i32) {
    let aspect_ratio = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
        aspect_ratio
    } else {
        DEFAULT_PREVIEW_ASPECT
    };
    let width = if fit_to_viewport && viewport_width > PREVIEW_HORIZONTAL_INSET {
        f64::from(viewport_width - PREVIEW_HORIZONTAL_INSET)
    } else {
        DEFAULT_PREVIEW_WIDTH * zoom.clamp(0.3, 3.0)
    }
    .max(180.0);
    (width.round() as i32, (width / aspect_ratio).round() as i32)
}

fn svg_aspect_ratio(svg: &str) -> Option<f64> {
    static VIEW_BOX: OnceLock<Regex> = OnceLock::new();
    static WIDTH: OnceLock<Regex> = OnceLock::new();
    static HEIGHT: OnceLock<Regex> = OnceLock::new();
    let view_box = VIEW_BOX.get_or_init(|| {
        Regex::new(
            r#"(?i)\bviewBox\s*=\s*["']\s*[-+0-9.eE]+\s+[-+0-9.eE]+\s+([-+0-9.eE]+)\s+([-+0-9.eE]+)\s*["']"#,
        )
        .expect("valid SVG viewBox regex")
    });
    if let Some(captures) = view_box.captures(svg) {
        let width = captures.get(1)?.as_str().parse::<f64>().ok()?;
        let height = captures.get(2)?.as_str().parse::<f64>().ok()?;
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            return Some(width / height);
        }
    }
    let width_regex = WIDTH.get_or_init(|| {
        Regex::new(r#"(?i)\bwidth\s*=\s*["']\s*([-+0-9.eE]+)"#).expect("valid SVG width regex")
    });
    let height_regex = HEIGHT.get_or_init(|| {
        Regex::new(r#"(?i)\bheight\s*=\s*["']\s*([-+0-9.eE]+)"#).expect("valid SVG height regex")
    });
    let width = width_regex
        .captures(svg)?
        .get(1)?
        .as_str()
        .parse::<f64>()
        .ok()?;
    let height = height_regex
        .captures(svg)?
        .get(1)?
        .as_str()
        .parse::<f64>()
        .ok()?;
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some(width / height)
}

fn extract_external_links(svg: &str) -> Vec<String> {
    static HREF: OnceLock<Regex> = OnceLock::new();
    let href = HREF.get_or_init(|| {
        Regex::new(r#"(?i)(?:\bhref|xlink:href)\s*=\s*(?:[\"]([^\"]*)[\"]|[']([^']*)['])"#)
            .expect("valid SVG link regex")
    });
    let mut seen = HashSet::new();
    href.captures_iter(svg)
        .filter_map(|captures| captures.get(1).or_else(|| captures.get(2)))
        .map(|value| decode_xml_attribute(value.as_str()))
        .filter(|value| {
            Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
        })
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn decode_xml_attribute(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn external_link_label(uri: &str) -> String {
    let Ok(url) = Url::parse(uri) else {
        return uri.to_string();
    };
    let label = if url.scheme() == "mailto" {
        format!("Email {}", url.path())
    } else {
        let host = url.host_str().unwrap_or_default();
        let path = url.path().trim_end_matches('/');
        if path.is_empty() {
            format!("Open {host}")
        } else {
            format!("Open {host}{path}")
        }
    };
    let mut characters = label.chars();
    let shortened = characters.by_ref().take(72).collect::<String>();
    if characters.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn source_line_for_page_position(
    page_index: usize,
    page_count: usize,
    within_page: f64,
    source_line_count: usize,
) -> usize {
    if page_count == 0 || source_line_count <= 1 {
        return 1;
    }
    let page = page_index.min(page_count - 1) as f64;
    let position = (page + within_page.clamp(0.0, 1.0)) / page_count as f64;
    1 + (position * source_line_count.saturating_sub(1) as f64).round() as usize
}

fn focus_source_line(
    select: &Rc<dyn Fn(String)>,
    source_path: &str,
    line: usize,
    buffer: &sourceview5::Buffer,
    editor: &sourceview5::View,
) {
    select(source_path.to_string());
    if let Some(mut target) = buffer.iter_at_line(line.saturating_sub(1) as i32) {
        buffer.place_cursor(&target);
        editor.scroll_to_iter(&mut target, 0.15, false, 0.0, 0.25);
        editor.grab_focus();
    }
}

fn build_settings_dialog(
    parent: &gtk::ApplicationWindow,
    settings: Rc<RefCell<UiSettings>>,
    on_changed: Rc<dyn Fn(UiSettings)>,
) -> gtk::Window {
    let dialog = gtk::Window::builder()
        .title("Settings — typsmthng")
        .transient_for(parent)
        .modal(true)
        .default_width(620)
        .default_height(520)
        .hide_on_close(true)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some("Settings"))));
    root.append(&header);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 12);
    rows.set_margin_top(20);
    rows.set_margin_bottom(20);
    rows.set_margin_start(22);
    rows.set_margin_end(22);
    rows.append(&setting_heading("Editor"));
    let font = gtk::SpinButton::with_range(10.0, 28.0, 1.0);
    font.set_value(settings.borrow().font_size as f64);
    rows.append(&setting_row(
        "Editor font size",
        "Points used by GtkSourceView",
        &font,
    ));
    let line_numbers = gtk::Switch::new();
    line_numbers.set_active(settings.borrow().line_numbers);
    rows.append(&setting_row(
        "Line numbers",
        "Show the source gutter",
        &line_numbers,
    ));
    let wrapping = gtk::Switch::new();
    wrapping.set_active(settings.borrow().line_wrapping);
    rows.append(&setting_row(
        "Line wrapping",
        "Wrap long source lines",
        &wrapping,
    ));
    let vim = gtk::Switch::new();
    vim.set_active(settings.borrow().vim_mode);
    rows.append(&setting_row(
        "Vim input",
        "Use GtkSourceView's native Vim mode",
        &vim,
    ));
    rows.append(&setting_heading("Compilation"));
    let auto_compile = gtk::Switch::new();
    auto_compile.set_active(settings.borrow().auto_compile);
    rows.append(&setting_row(
        "Live compile",
        "Render after source changes",
        &auto_compile,
    ));
    let delay = gtk::SpinButton::with_range(100.0, 2000.0, 50.0);
    delay.set_value(settings.borrow().compile_delay_ms as f64);
    rows.append(&setting_row(
        "Compile delay",
        "Debounce in milliseconds",
        &delay,
    ));
    let theme = gtk::DropDown::from_strings(&["System", "Light", "Dark"]);
    theme.set_selected(match settings.borrow().theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    rows.append(&setting_row(
        "Theme",
        "Native light or dark palette",
        &theme,
    ));
    let page_size = gtk::DropDown::from_strings(&[
        "Auto (A4)",
        "A3",
        "A4",
        "A5",
        "A6",
        "US Letter",
        "US Legal",
        "ISO B5",
        "Presentation 16:9",
    ]);
    let selected_page_size = match settings.borrow().page_size.as_str() {
        "a3" => 1,
        "a4" => 2,
        "a5" => 3,
        "a6" => 4,
        "us-letter" => 5,
        "us-legal" => 6,
        "iso-b5" => 7,
        "presentation-16-9" => 8,
        _ => 0,
    };
    page_size.set_selected(selected_page_size);
    rows.append(&setting_row(
        "Page size",
        "Optional page preamble",
        &page_size,
    ));
    let notes_layout =
        gtk::DropDown::from_strings(&["Auto detect", "Notes on right half", "Whole page"]);
    notes_layout.set_selected(match settings.borrow().presentation_notes_layout.as_str() {
        "right-half" => 1,
        "whole" => 2,
        _ => 0,
    });
    rows.append(&setting_row(
        "Presenter notes layout",
        "Override automatic splitting for ultrawide documents",
        &notes_layout,
    ));
    let system_fonts = gtk::Switch::new();
    system_fonts.set_active(settings.borrow().system_fonts);
    rows.append(&setting_row(
        "System fonts",
        "Expose installed fonts to Typst",
        &system_fonts,
    ));
    let google_fonts = gtk::Switch::new();
    google_fonts.set_active(settings.borrow().google_fonts);
    rows.append(&setting_row(
        "Google Fonts",
        "Allow missing font download",
        &google_fonts,
    ));
    let translucent = gtk::Switch::new();
    translucent.set_active(settings.borrow().translucent);
    rows.append(&setting_row(
        "Translucent chrome",
        "Use compositor transparency where supported",
        &translucent,
    ));
    let apply = gtk::Button::with_label("Apply settings");
    apply.add_css_class("suggested-action");
    rows.append(&apply);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&rows));
    scroll.set_vexpand(true);
    root.append(&scroll);
    dialog.set_child(Some(&root));

    apply.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            let value = UiSettings {
                theme: match theme.selected() {
                    1 => Theme::Light,
                    2 => Theme::Dark,
                    _ => Theme::System,
                },
                font_size: font.value() as u32,
                line_numbers: line_numbers.is_active(),
                line_wrapping: wrapping.is_active(),
                vim_mode: vim.is_active(),
                auto_compile: auto_compile.is_active(),
                compile_delay_ms: delay.value() as u32,
                page_size: match page_size.selected() {
                    1 => "a3",
                    2 => "a4",
                    3 => "a5",
                    4 => "a6",
                    5 => "us-letter",
                    6 => "us-legal",
                    7 => "iso-b5",
                    8 => "presentation-16-9",
                    _ => "auto",
                }
                .into(),
                presentation_notes_layout: match notes_layout.selected() {
                    1 => "right-half",
                    2 => "whole",
                    _ => "auto",
                }
                .into(),
                presentation_notes_font_size: settings.borrow().presentation_notes_font_size,
                system_fonts: system_fonts.is_active(),
                google_fonts: google_fonts.is_active(),
                translucent: translucent.is_active(),
            };
            settings.replace(value.clone());
            on_changed(value);
            dialog.set_visible(false);
        }
    });
    dialog
}

fn setting_heading(label: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(label));
    label.add_css_class("section-title");
    label.set_halign(gtk::Align::Start);
    label.set_margin_top(8);
    label
}

fn setting_row(label: &str, detail: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(label));
    title.set_halign(gtk::Align::Start);
    text.append(&title);
    let detail = gtk::Label::new(Some(detail));
    detail.set_halign(gtk::Align::Start);
    detail.add_css_class("muted");
    text.append(&detail);
    row.append(&text);
    row.append(control);
    row
}

fn build_search_dialog(
    parent: &gtk::ApplicationWindow,
    search: Rc<dyn Fn(SearchMode, String) -> Vec<SearchResultRow>>,
    select: Rc<dyn Fn(String)>,
    buffer: sourceview5::Buffer,
    editor: sourceview5::View,
) -> gtk::Window {
    let dialog = gtk::Window::builder()
        .title("Search — typsmthng")
        .transient_for(parent)
        .modal(true)
        .default_width(680)
        .default_height(440)
        .hide_on_close(true)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("search-palette");
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let files = gtk::ToggleButton::with_label("Files");
    files.set_active(true);
    let contents = gtk::ToggleButton::with_label("Contents");
    contents.set_group(Some(&files));
    let commands = gtk::ToggleButton::with_label("Commands");
    commands.set_group(Some(&files));
    modes.append(&files);
    modes.append(&contents);
    modes.append(&commands);
    root.append(&modes);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Search the vault or run a command…"));
    root.append(&entry);
    let results = gtk::ListBox::new();
    let result_paths = Rc::new(RefCell::new(Vec::<
        Option<(String, Option<usize>, Option<usize>)>,
    >::new()));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&results));
    scroll.set_vexpand(true);
    root.append(&scroll);
    dialog.set_child(Some(&root));

    let refresh: Rc<dyn Fn()> = {
        let entry = entry.clone();
        let files = files.clone();
        let contents = contents.clone();
        let results = results.clone();
        let result_paths = result_paths.clone();
        Rc::new(move || {
            while let Some(child) = results.first_child() {
                results.remove(&child);
            }
            result_paths.borrow_mut().clear();
            let mode = if files.is_active() {
                SearchMode::Files
            } else if contents.is_active() {
                SearchMode::Contents
            } else {
                SearchMode::Commands
            };
            for result in search(mode, entry.text().to_string()) {
                result_paths
                    .borrow_mut()
                    .push(result.path.map(|path| (path, result.line, result.column)));
                let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
                row.set_margin_top(7);
                row.set_margin_bottom(7);
                row.set_margin_start(9);
                row.set_margin_end(9);
                let primary = gtk::Label::new(Some(&result.primary));
                primary.set_halign(gtk::Align::Start);
                row.append(&primary);
                let secondary = gtk::Label::new(Some(&result.secondary));
                secondary.set_halign(gtk::Align::Start);
                secondary.set_ellipsize(gtk::pango::EllipsizeMode::End);
                secondary.add_css_class("muted");
                row.append(&secondary);
                results.append(&row);
            }
        })
    };
    entry.connect_search_changed({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    files.connect_toggled({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    contents.connect_toggled({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    commands.connect_toggled({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    results.connect_row_activated({
        let result_paths = result_paths.clone();
        let dialog = dialog.clone();
        let buffer = buffer.clone();
        let editor = editor.clone();
        move |_, row| {
            if let Some(Some((path, line, column))) =
                result_paths.borrow().get(row.index() as usize)
            {
                select(path.clone());
                if let Some(mut target) =
                    line.and_then(|line| buffer.iter_at_line(line.saturating_sub(1) as i32))
                {
                    target.forward_chars(column.unwrap_or(1).saturating_sub(1) as i32);
                    buffer.place_cursor(&target);
                    editor.scroll_to_iter(&mut target, 0.15, false, 0.0, 0.25);
                }
                dialog.set_visible(false);
            }
        }
    });
    dialog.connect_show(move |_| {
        entry.grab_focus();
    });
    dialog
}

#[cfg(test)]
mod tests {
    use super::{
        extract_external_links, preview_dimensions, should_uncomment_lines,
        source_line_for_page_position, svg_aspect_ratio,
    };

    #[test]
    fn comment_toggle_ignores_blank_lines() {
        assert!(should_uncomment_lines(&[
            "  // first".into(),
            String::new(),
            "\t/// second".into(),
        ]));
        assert!(!should_uncomment_lines(&[
            "// first".into(),
            "second".into(),
        ]));
        assert!(!should_uncomment_lines(&[String::new()]));
    }

    #[test]
    fn preview_fit_uses_viewport_and_svg_aspect() {
        assert_eq!(preview_dimensions(900, 1.0, true, 2.0), (836, 418));
        assert_eq!(preview_dimensions(900, 1.5, false, 2.0), (840, 420));
        assert_eq!(preview_dimensions(0, 1.0, true, 2.0), (560, 280));
        assert_eq!(
            svg_aspect_ratio(r#"<svg viewBox="0 0 800 400"></svg>"#),
            Some(2.0)
        );
        assert_eq!(
            svg_aspect_ratio(r#"<svg width="600pt" height="800pt"></svg>"#),
            Some(0.75)
        );
    }

    #[test]
    fn preview_links_are_external_safe_and_deduplicated() {
        let svg = r##"<svg>
          <a href="#local">local</a>
          <a href="https://example.com/docs?a=1&amp;b=2">docs</a>
          <a xlink:href='mailto:hello@example.com'>mail</a>
          <a href="javascript:alert(1)">unsafe</a>
          <a href="https://example.com/docs?a=1&amp;b=2">duplicate</a>
        </svg>"##;
        assert_eq!(
            extract_external_links(svg),
            vec![
                "https://example.com/docs?a=1&b=2".to_string(),
                "mailto:hello@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn page_position_maps_across_the_source() {
        assert_eq!(source_line_for_page_position(0, 3, 0.0, 30), 1);
        assert_eq!(source_line_for_page_position(1, 3, 0.5, 30), 16);
        assert_eq!(source_line_for_page_position(2, 3, 1.0, 30), 30);
        assert_eq!(source_line_for_page_position(4, 0, 0.5, 30), 1);
    }
}
