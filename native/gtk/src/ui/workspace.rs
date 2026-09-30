use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use typsmthng_gtk::backend::preview::SourceMap;

use adw::prelude::*;
use regex::Regex;
use sha2::{Digest, Sha256};
use sourceview5::prelude::*;
use url::Url;

use super::home::icon_button;
use super::model::{
    page_size_label, zoom_editor_font, SearchMode, Theme, UiSettings, ViewMode,
    DEFAULT_EDITOR_FONT_SIZE, EDITOR_FONT_SIZES, PAGE_SIZES,
};
use super::zoom::{
    connect_ctrl_scroll, fit_text_crop, format_scale, step_preview_scale, PreviewZoom, ZoomBadge,
    PIXELS_PER_POINT, PREVIEW_SCROLL_FACTOR,
};

type SearchCallback = Rc<dyn Fn(SearchMode, String, Rc<dyn Fn(Vec<SearchResultRow>)>)>;

/// Apply a preview zoom, optionally anchored at a viewport point.
type SetPreviewZoom = Rc<dyn Fn(PreviewZoom, Option<(f64, f64)>)>;

type DiagnosticLocation = (String, Option<usize>, Option<usize>);

const DEFAULT_PREVIEW_WIDTH: f64 = 560.0;
const DEFAULT_PREVIEW_ASPECT: f64 = 16.0 / 9.0;
const PREVIEW_HORIZONTAL_INSET: i32 = 64;
/// Page width assumed for SVGs without a usable viewBox (560 px at 100%).
const FALLBACK_PAGE_WIDTH: f64 = DEFAULT_PREVIEW_WIDTH / PIXELS_PER_POINT;

#[derive(Debug, PartialEq, Eq)]
struct PreviewIdentity {
    pages: Vec<[u8; 32]>,
    source: Option<String>,
}

fn preview_identity(pages: &[Option<String>], source: Option<&str>) -> Option<PreviewIdentity> {
    let pages = pages
        .iter()
        .map(|page| {
            page.as_ref()
                .map(|page| Sha256::digest(page.as_bytes()).into())
        })
        .collect::<Option<Vec<_>>>()?;
    Some(PreviewIdentity {
        pages,
        source: source.map(str::to_string),
    })
}

fn reusable_preview_pages(
    previous: Option<&PreviewIdentity>,
    next: Option<&PreviewIdentity>,
) -> Vec<bool> {
    match (previous, next) {
        (Some(previous), Some(next))
            if previous.source == next.source && previous.pages.len() == next.pages.len() =>
        {
            previous
                .pages
                .iter()
                .zip(&next.pages)
                .map(|(old, new)| old == new)
                .collect()
        }
        _ => Vec::new(),
    }
}

#[derive(Clone)]
struct PreviewPicture {
    sheet: gtk::Box,
    widget: gtk::Picture,
    aspect_ratio: f64,
    /// Page width in SVG user units (points for Typst output).
    page_width: f64,
}

/// A document point, as a fraction of one page, held under a viewport point.
struct PreviewAnchor {
    page: usize,
    fraction: (f64, f64),
    pointer: (f64, f64),
}

fn preview_anchor(
    pictures: &[PreviewPicture],
    scroll: &gtk::ScrolledWindow,
    pointer: (f64, f64),
) -> Option<PreviewAnchor> {
    let bounds = pictures
        .iter()
        .map(|picture| picture.widget.compute_bounds(scroll))
        .collect::<Option<Vec<_>>>()?;
    // Prefer the page under the pointer, else the vertically nearest page.
    let distance = |rect: &gtk::graphene::Rect| {
        let (top, bottom) = (f64::from(rect.y()), f64::from(rect.y() + rect.height()));
        if pointer.1 < top {
            top - pointer.1
        } else if pointer.1 > bottom {
            pointer.1 - bottom
        } else {
            0.0
        }
    };
    let (page, rect) = bounds
        .iter()
        .enumerate()
        .filter(|(_, rect)| rect.width() > 0.0 && rect.height() > 0.0)
        .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))?;
    Some(PreviewAnchor {
        page,
        fraction: (
            (pointer.0 - f64::from(rect.x())) / f64::from(rect.width()),
            (pointer.1 - f64::from(rect.y())) / f64::from(rect.height()),
        ),
        pointer,
    })
}

fn restore_preview_anchor(
    pictures: &[PreviewPicture],
    scroll: &gtk::ScrolledWindow,
    anchor: &PreviewAnchor,
) {
    let Some(rect) = pictures
        .get(anchor.page)
        .and_then(|picture| picture.widget.compute_bounds(scroll))
    else {
        return;
    };
    let x = f64::from(rect.x()) + anchor.fraction.0 * f64::from(rect.width());
    let y = f64::from(rect.y()) + anchor.fraction.1 * f64::from(rect.height());
    for (adjustment, drift) in [
        (scroll.hadjustment(), x - anchor.pointer.0),
        (scroll.vadjustment(), y - anchor.pointer.1),
    ] {
        adjustment.set_value((adjustment.value() + drift).clamp(
            adjustment.lower(),
            adjustment.upper() - adjustment.page_size(),
        ));
    }
}

#[derive(Clone)]
struct SettingsDialog {
    window: gtk::Window,
    font: gtk::SpinButton,
    line_numbers: gtk::Switch,
    wrapping: gtk::Switch,
    vim: gtk::Switch,
    auto_compile: gtk::Switch,
    delay: gtk::SpinButton,
    theme: gtk::DropDown,
    page_size: gtk::DropDown,
    notes_layout: gtk::DropDown,
    system_fonts: gtk::Switch,
    google_fonts: gtk::Switch,
    translucent: gtk::Switch,
}

#[derive(Clone)]
struct DocumentSearchDialog {
    window: gtk::Window,
    find: gtk::Entry,
}

impl DocumentSearchDialog {
    fn present(&self, buffer: &sourceview5::Buffer) {
        if let Some((start, end)) = buffer.selection_bounds() {
            let selected = buffer.text(&start, &end, true);
            if !selected.is_empty() && !selected.contains('\n') {
                self.find.set_text(&selected);
            }
        }
        self.window.present();
        self.find.grab_focus();
        self.find.select_region(0, -1);
    }
}

impl SettingsDialog {
    fn sync(&self, settings: &UiSettings) {
        self.font.set_value(settings.font_size as f64);
        self.line_numbers.set_active(settings.line_numbers);
        self.wrapping.set_active(settings.line_wrapping);
        self.vim.set_active(settings.vim_mode);
        self.auto_compile.set_active(settings.auto_compile);
        self.delay.set_value(settings.compile_delay_ms as f64);
        self.theme.set_selected(match settings.theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        });
        self.page_size
            .set_selected(page_size_index(&settings.page_size));
        self.notes_layout
            .set_selected(match settings.presentation_notes_layout.as_str() {
                "right-half" => 1,
                "whole" => 2,
                _ => 0,
            });
        self.system_fonts.set_active(settings.system_fonts);
        self.google_fonts.set_active(settings.google_fonts);
        self.translucent.set_active(settings.translucent);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub drop_files: Rc<dyn Fn((Vec<PathBuf>, String))>,
    pub move_path: Rc<dyn Fn((String, String))>,
    pub toggle_hidden: Rc<dyn Fn()>,
    pub rename_path: Rc<dyn Fn(String)>,
    pub duplicate_path: Rc<dyn Fn(String)>,
    pub trash_path: Rc<dyn Fn(String)>,
    pub reveal_path: Rc<dyn Fn(String)>,
    pub open_external: Rc<dyn Fn(String)>,
    pub preview_asset: Rc<dyn Fn(String)>,
    pub check_update: Rc<dyn Fn()>,
    pub export_pdf: Rc<dyn Fn()>,
    pub export_project: Rc<dyn Fn()>,
    pub present_single: Rc<dyn Fn()>,
    pub present_dual: Rc<dyn Fn()>,
    pub refresh_compile: Rc<dyn Fn(String)>,
    pub search: SearchCallback,
    pub settings_changed: Rc<dyn Fn(UiSettings)>,
    /// Persist layout preferences (zoom, panes) without recompiling.
    pub preferences_changed: Rc<dyn Fn(UiSettings)>,
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
    editor_style: gtk::CssProvider,
    sidebar: gtk::Box,
    file_list: gtk::ListBox,
    file_paths: Rc<RefCell<Vec<String>>>,
    file_rows: Rc<RefCell<Vec<FileRow>>>,
    expanded_directories: Rc<RefCell<HashSet<String>>>,
    known_directories: Rc<RefCell<HashSet<String>>>,
    preview_pages: gtk::Box,
    preview_identity: Rc<RefCell<Option<PreviewIdentity>>>,
    preview_pictures: Rc<RefCell<Vec<PreviewPicture>>>,
    resize_preview: Rc<dyn Fn()>,
    preview_zoom: Rc<Cell<PreviewZoom>>,
    content_column: Rc<Cell<Option<(f64, f64)>>>,
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
    settings_dialog: SettingsDialog,
    document_search_dialog: DocumentSearchDialog,
    dirty: Rc<Cell<bool>>,
    suppress_changes: Rc<Cell<bool>>,
    pending_compile: Rc<RefCell<Option<glib::SourceId>>>,
    pending_diagnostics: Rc<RefCell<Option<glib::SourceId>>>,
    pending_error: Rc<RefCell<Option<glib::SourceId>>>,
    revision: Rc<Cell<u64>>,
    last_edit: Rc<Cell<Option<Instant>>>,
    source_map: Rc<RefCell<Option<Arc<SourceMap>>>>,
    callbacks: WorkspaceCallbacks,
    vim_context: Rc<RefCell<Option<sourceview5::VimIMContext>>>,
    vim_status: gtk::Label,
    editor_pane: gtk::Widget,
    view_buttons: [gtk::ToggleButton; 3],
    editor_badge: ZoomBadge,
}

impl WorkspaceView {
    pub fn new(window: &gtk::ApplicationWindow, callbacks: WorkspaceCallbacks) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("toolbar");
        toolbar.add_css_class("workspace-toolbar");
        let home = gtk::Button::with_label("t.");
        home.add_css_class("brand-mark");
        home.set_tooltip_text(Some("Back to projects"));
        let open_project = icon_button("folder-open-symbolic", "Open project");
        let sidebar_toggle = icon_button("sidebar-show-symbolic", "Toggle file tree (Ctrl+\\)");
        let project_label = gtk::Label::new(Some("Vault"));
        project_label.add_css_class("eyebrow");
        project_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        project_label.set_hexpand(true);
        project_label.set_halign(gtk::Align::Start);
        project_label.set_margin_start(6);
        let file_label = gtk::Label::new(Some("No file"));
        file_label.add_css_class("file-tab");
        file_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        file_label.set_hexpand(true);
        file_label.set_halign(gtk::Align::Fill);
        file_label.set_max_width_chars(45);
        let search = icon_button(
            "system-search-symbolic",
            "Search files, contents, and commands (Ctrl+K)",
        );
        let document_search = icon_button("edit-find-symbolic", "Find and replace (Ctrl+F)");
        let compile = icon_button("view-refresh-symbolic", "Compile now (Ctrl+Enter)");
        let export = icon_button("document-save-symbolic", "Export PDF (Ctrl+Shift+E)");
        let export_project = icon_button("package-x-generic-symbolic", "Export project ZIP");
        let present = gtk::MenuButton::new();
        present.set_icon_name("media-playback-start-symbolic");
        present.set_tooltip_text(Some("Present (F5)"));
        let settings_button = icon_button("preferences-system-symbolic", "Settings (Ctrl+,)");
        let theme_button = icon_button(
            "weather-clear-night-symbolic",
            "Cycle system, light, and dark theme",
        );
        let update_button = icon_button("software-update-available-symbolic", "Check for updates");
        toolbar.append(&home);
        toolbar.append(&sidebar_toggle);
        toolbar.append(&open_project);
        toolbar.append(&settings_button);
        toolbar.append(&theme_button);
        toolbar.append(&search);
        toolbar.append(&file_label);
        let view_switcher = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        view_switcher.add_css_class("linked");
        view_switcher.add_css_class("view-switcher");
        let view_buttons = ViewMode::ALL.map(|mode| {
            let (icon, tooltip) = match mode {
                ViewMode::Source => ("document-edit-symbolic", "Source only (Ctrl+1)"),
                ViewMode::Split => ("view-dual-symbolic", "Source and preview (Ctrl+2)"),
                ViewMode::Preview => ("document-print-preview-symbolic", "Preview only (Ctrl+3)"),
            };
            let button = gtk::ToggleButton::new();
            button.set_icon_name(icon);
            button.set_tooltip_text(Some(tooltip));
            button.add_css_class("flat");
            view_switcher.append(&button);
            button
        });
        for button in &view_buttons[1..] {
            button.set_group(Some(&view_buttons[0]));
        }
        view_buttons[1].set_active(true);
        toolbar.append(&view_switcher);
        toolbar.append(&export);
        toolbar.append(&present);
        let more = gtk::MenuButton::new();
        more.set_icon_name("view-more-symbolic");
        more.set_tooltip_text(Some("More actions"));
        let more_popover = gtk::Popover::new();
        let more_actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        more_actions.append(&document_search);
        more_actions.append(&compile);
        more_actions.append(&export_project);
        more_actions.append(&update_button);
        more_popover.set_child(Some(&more_actions));
        more.set_popover(Some(&more_popover));
        toolbar.append(&more);
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
        main_paned.set_position(240);
        main_paned.set_resize_start_child(false);
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
        let files_title = project_label.clone();
        files_title.add_css_class("eyebrow");
        files_title.set_halign(gtk::Align::Fill);
        files_title.set_xalign(0.0);
        files_title.set_hexpand(true);
        let new_file = icon_button("document-new-symbolic", "New file");
        let new_folder = icon_button("folder-new-symbolic", "New folder");
        let import_files = icon_button("document-open-symbolic", "Import files into vault");
        let hidden = icon_button("view-reveal-symbolic", "Toggle hidden files");
        sidebar_header.append(&files_title);
        sidebar_header.append(&new_file);
        sidebar_header.append(&new_folder);
        let file_more = gtk::MenuButton::new();
        file_more.set_icon_name("view-more-symbolic");
        file_more.set_tooltip_text(Some("Import and hidden-file options"));
        file_more.add_css_class("flat");
        let file_options = gtk::Popover::new();
        let file_option_actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        file_option_actions.append(&import_files);
        file_option_actions.append(&hidden);
        file_options.set_child(Some(&file_option_actions));
        file_more.set_popover(Some(&file_options));
        sidebar_header.append(&file_more);
        sidebar.append(&sidebar_header);
        let file_list = gtk::ListBox::new();
        file_list.add_css_class("file-tree");
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
        work_paned.set_position(470);
        work_paned.set_shrink_start_child(true);
        work_paned.set_shrink_end_child(true);
        main_paned.set_end_child(Some(&work_paned));
        // Preserve a usable editor and complete preview controls on laptops.
        // Only react when crossing the threshold so manual toggles still work.
        let narrow_layout = Rc::new(Cell::new(None::<bool>));
        let restore_sidebar = Rc::new(Cell::new(false));
        let adapt_layout: Rc<dyn Fn(i32)> = Rc::new({
            let sidebar = sidebar.clone();
            let work_paned = work_paned.clone();
            move |width| {
                if width <= 0 {
                    return;
                }
                let narrow = width < 1000;
                if Some(narrow) == narrow_layout.replace(Some(narrow)) {
                    return;
                }
                if narrow {
                    restore_sidebar.set(sidebar.is_visible());
                    sidebar.set_visible(false);
                } else if restore_sidebar.replace(false) {
                    sidebar.set_visible(true);
                }
                work_paned.set_position((width - if sidebar.is_visible() { 240 } else { 0 }) / 2);
            }
        });

        let buffer = sourceview5::Buffer::new(None::<&gtk::TextTagTable>);
        let language_manager = sourceview5::LanguageManager::default();
        for path in data_search_paths("language-specs") {
            language_manager.append_search_path(path.to_string_lossy().as_ref());
        }
        let scheme_manager = sourceview5::StyleSchemeManager::default();
        for path in data_search_paths("styles") {
            scheme_manager.append_search_path(path.to_string_lossy().as_ref());
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
        editor.add_css_class("typst-editor");
        let editor_style = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &editor.display(),
            &editor_style,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
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
        install_pair_completion(&editor, &buffer, vim_context.clone());
        let editor_scroll = gtk::ScrolledWindow::new();
        editor_scroll.set_child(Some(&editor));
        editor_scroll.set_hexpand(true);
        editor_scroll.set_vexpand(true);
        let editor_overlay = gtk::Overlay::new();
        editor_overlay.set_child(Some(&editor_scroll));
        let editor_badge = ZoomBadge::new();
        editor_overlay.add_overlay(editor_badge.widget());
        work_paned.set_start_child(Some(&editor_overlay));

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
        preview_controls.add_css_class("preview-controls");
        let preview_heading = gtk::Label::new(Some("PREVIEW"));
        preview_heading.add_css_class("eyebrow");
        preview_heading.set_hexpand(true);
        preview_heading.set_halign(gtk::Align::Start);
        preview_controls.append(&preview_heading);
        let zoom_out = icon_button("zoom-out-symbolic", "Zoom out");
        let zoom_menu = gtk::MenuButton::new();
        zoom_menu.set_label("Fit");
        zoom_menu.add_css_class("flat");
        zoom_menu.add_css_class("zoom-level");
        let zoom_in = icon_button("zoom-in-symbolic", "Zoom in");
        let prev_page = icon_button("go-up-symbolic", "Previous page");
        let next_page = icon_button("go-down-symbolic", "Next page");
        preview_controls.append(&zoom_out);
        preview_controls.append(&zoom_menu);
        preview_controls.append(&zoom_in);
        preview_controls.append(&prev_page);
        preview_controls.append(&next_page);
        let preview_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview_panel.append(&preview_controls);
        preview_panel.append(&preview_overlay);
        let preview_badge = ZoomBadge::new();
        preview_overlay.add_overlay(preview_badge.widget());
        let zoom = Rc::new(Cell::new(PreviewZoom::FitWidth));
        // Kept across edits: the source map is dropped on every keystroke, but
        // the text column should not jump while the next compile is pending.
        let content_column = Rc::new(Cell::new(None::<(f64, f64)>));
        let displayed_scale: Rc<dyn Fn() -> f64> = {
            let pictures = preview_pictures.clone();
            let zoom = zoom.clone();
            let preview_scroll = preview_scroll.clone();
            let content_column = content_column.clone();
            Rc::new(move || {
                let page_width = pictures
                    .borrow()
                    .first()
                    .map_or(FALLBACK_PAGE_WIDTH, |picture| picture.page_width);
                zoom.get().scale(
                    f64::from(preview_scroll.width() - PREVIEW_HORIZONTAL_INSET),
                    page_width,
                    content_column.get(),
                )
            })
        };
        let resize_preview: Rc<dyn Fn()> = {
            let pictures = preview_pictures.clone();
            let zoom = zoom.clone();
            let preview_scroll = preview_scroll.clone();
            let zoom_menu = zoom_menu.clone();
            let displayed_scale = displayed_scale.clone();
            let content_column = content_column.clone();
            Rc::new(move || {
                let viewport_width = preview_scroll.width();
                for picture in pictures.borrow().iter() {
                    let size = preview_dimensions(
                        viewport_width,
                        zoom.get(),
                        picture.page_width,
                        picture.aspect_ratio,
                        content_column.get(),
                    );
                    picture.widget.set_size_request(size.width, size.height);
                    picture.widget.set_content_fit(if size.cropped {
                        gtk::ContentFit::Cover
                    } else {
                        gtk::ContentFit::Contain
                    });
                }
                let scale = format_scale(displayed_scale());
                let fitted = viewport_width > PREVIEW_HORIZONTAL_INSET;
                zoom_menu.set_label(&match zoom.get() {
                    PreviewZoom::FitWidth if fitted => format!("Fit · {scale}"),
                    PreviewZoom::FitText if fitted => format!("Text · {scale}"),
                    PreviewZoom::FitWidth | PreviewZoom::FitText => "Fit".into(),
                    PreviewZoom::Scale(_) => scale,
                });
            })
        };
        // Zooming keeps the document point under the pointer (or the viewport
        // centre) fixed. Page sizes change only after the next allocation, so
        // remember where the anchor sat on its page and restore it afterwards.
        let pending_anchor = Rc::new(RefCell::new(None::<PreviewAnchor>));
        let set_zoom: SetPreviewZoom = {
            let zoom = zoom.clone();
            let resize = resize_preview.clone();
            let pictures = preview_pictures.clone();
            let preview_scroll = preview_scroll.clone();
            let preview_pages = preview_pages.clone();
            let badge = preview_badge.clone();
            let displayed_scale = displayed_scale.clone();
            Rc::new(move |next, pointer| {
                let pointer = pointer.unwrap_or_else(|| {
                    (
                        f64::from(preview_scroll.width()) / 2.0,
                        f64::from(preview_scroll.height()) / 2.0,
                    )
                });
                let schedule = {
                    let mut pending = pending_anchor.borrow_mut();
                    match pending.as_mut() {
                        // Rapid wheel events: the layout is still stale, keep
                        // the original page fraction and follow the pointer.
                        Some(anchor) => {
                            anchor.pointer = pointer;
                            false
                        }
                        None => {
                            *pending = preview_anchor(&pictures.borrow(), &preview_scroll, pointer);
                            pending.is_some()
                        }
                    }
                };
                zoom.set(next);
                resize();
                badge.show(&format_scale(displayed_scale()));
                if schedule {
                    let pending_anchor = pending_anchor.clone();
                    let pictures = pictures.clone();
                    let preview_scroll = preview_scroll.clone();
                    let allocated_once = Cell::new(false);
                    preview_pages.add_tick_callback(move |_, _| {
                        if !allocated_once.replace(true) {
                            return glib::ControlFlow::Continue;
                        }
                        if let Some(anchor) = pending_anchor.borrow_mut().take() {
                            restore_preview_anchor(&pictures.borrow(), &preview_scroll, &anchor);
                        }
                        glib::ControlFlow::Break
                    });
                }
            })
        };
        let zoom_popover = gtk::Popover::new();
        let zoom_choices = gtk::Box::new(gtk::Orientation::Vertical, 2);
        zoom_choices.set_margin_top(6);
        zoom_choices.set_margin_bottom(6);
        zoom_choices.set_margin_start(6);
        zoom_choices.set_margin_end(6);
        let fit_text = gtk::Button::with_label("Fit text width");
        fit_text.add_css_class("flat");
        fit_text.set_tooltip_text(Some("Crop the side margins"));
        fit_text.connect_clicked({
            let set_zoom = set_zoom.clone();
            let popover = zoom_popover.clone();
            move |_| {
                popover.popdown();
                set_zoom(PreviewZoom::FitText, None);
            }
        });
        let fit_width = gtk::Button::with_label("Fit page width");
        fit_width.add_css_class("flat");
        fit_width.connect_clicked({
            let set_zoom = set_zoom.clone();
            let popover = zoom_popover.clone();
            move |_| {
                popover.popdown();
                set_zoom(PreviewZoom::FitWidth, None);
            }
        });
        zoom_choices.append(&fit_width);
        zoom_choices.append(&fit_text);
        zoom_choices.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        for scale in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            let label = if scale == 1.0 {
                "100% · actual size".to_string()
            } else {
                format_scale(scale)
            };
            let choice = gtk::Button::with_label(&label);
            choice.add_css_class("flat");
            if let Some(label) = choice.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            choice.connect_clicked({
                let set_zoom = set_zoom.clone();
                let popover = zoom_popover.clone();
                move |_| {
                    popover.popdown();
                    set_zoom(PreviewZoom::Scale(scale), None);
                }
            });
            zoom_choices.append(&choice);
        }
        for button in [&fit_width, &fit_text] {
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
        }
        zoom_popover.set_child(Some(&zoom_choices));
        zoom_menu.set_popover(Some(&zoom_popover));
        for (button, direction) in [(&zoom_out, -1), (&zoom_in, 1)] {
            let set_zoom = set_zoom.clone();
            let displayed_scale = displayed_scale.clone();
            button.connect_clicked(move |_| {
                set_zoom(
                    PreviewZoom::clamped(step_preview_scale(displayed_scale(), direction)),
                    None,
                );
            });
        }
        connect_ctrl_scroll(&preview_scroll, {
            let set_zoom = set_zoom.clone();
            let displayed_scale = displayed_scale.clone();
            move |steps, x, y| {
                let scale = displayed_scale() * PREVIEW_SCROLL_FACTOR.powi(steps);
                set_zoom(PreviewZoom::clamped(scale), Some((x, y)));
            }
        });
        let pinch = gtk::GestureZoom::new();
        let pinch_start = Rc::new(Cell::new(1.0));
        pinch.connect_begin({
            let pinch_start = pinch_start.clone();
            let displayed_scale = displayed_scale.clone();
            move |_, _| pinch_start.set(displayed_scale())
        });
        pinch.connect_scale_changed({
            let set_zoom = set_zoom.clone();
            move |gesture, delta| {
                let centre = gesture.bounding_box_center();
                set_zoom(PreviewZoom::clamped(pinch_start.get() * delta), centre);
            }
        });
        preview_scroll.add_controller(pinch);
        // GdkSurface::layout reports actual native resize events. GTK's
        // default-width is only a requested size, and Widget has no width
        // property notification. Wait for allocation without an idle frame loop.
        let layout_pending = Rc::new(Cell::new(false));
        let refresh_layout: Rc<dyn Fn()> = {
            let root = root.downgrade();
            let resize = resize_preview.clone();
            let zoom = zoom.clone();
            Rc::new(move || {
                if let Some(root) = root.upgrade() {
                    adapt_layout(root.width());
                    if matches!(zoom.get(), PreviewZoom::FitWidth | PreviewZoom::FitText) {
                        resize();
                    }
                }
            })
        };
        let schedule_layout: Rc<dyn Fn()> = {
            let root = root.downgrade();
            Rc::new(move || {
                if let Some(root) = root.upgrade() {
                    schedule_after_allocation(
                        &root,
                        layout_pending.clone(),
                        refresh_layout.clone(),
                    );
                }
            })
        };
        root.connect_map({
            let schedule = schedule_layout.clone();
            move |_| schedule()
        });
        window.connect_realize({
            let schedule = schedule_layout.clone();
            move |window| {
                if let Some(surface) = window.surface() {
                    let schedule = schedule.clone();
                    surface.connect_layout(move |_, _, _| schedule());
                }
            }
        });
        if let Some(surface) = window.surface() {
            let schedule = schedule_layout.clone();
            surface.connect_layout(move |_, _, _| schedule());
        }
        for paned in [&main_paned, &work_paned] {
            let schedule = schedule_layout.clone();
            paned.connect_position_notify(move |_| schedule());
        }
        sidebar.connect_visible_notify(move |_| schedule_layout());
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
        work_paned.set_end_child(Some(&preview_panel));

        let diagnostics_revealer = gtk::Revealer::new();
        diagnostics_revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        let diagnostics_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        diagnostics_box.add_css_class("diagnostics");
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
        compile_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let page_label = gtk::Label::new(Some("0 pages"));
        let cursor_label = gtk::Label::new(Some("Ln 1, Col 1"));
        let vim_status = gtk::Label::new(None);
        vim_status.set_visible(false);
        status.append(&vim_status);
        status.append(&save_label);
        status.append(&compile_label);
        status.append(&page_label);
        status.append(&cursor_label);
        let language_label = gtk::Label::new(Some("TYPST"));
        language_label.add_css_class("status-language");
        status.append(&language_label);
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
        let document_search_dialog = build_document_search_dialog(window, &buffer, &editor);

        let pending_compile = Rc::new(RefCell::new(None::<glib::SourceId>));
        let pending_diagnostics = Rc::new(RefCell::new(None::<glib::SourceId>));
        let pending_error = Rc::new(RefCell::new(None::<glib::SourceId>));
        let revision = Rc::new(Cell::new(0_u64));
        let last_edit = Rc::new(Cell::new(None::<Instant>));
        let source_map = Rc::new(RefCell::new(None));
        {
            let callback = callbacks.go_home.clone();
            home.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.open_project.clone();
            open_project.connect_clicked(move |_| callback());
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
            let settings = settings.clone();
            settings_button.connect_clicked(move |_| {
                dialog.sync(&settings.borrow());
                dialog.window.present();
            });
        }
        {
            let settings = settings.clone();
            let changed = callbacks.settings_changed.clone();
            theme_button.connect_clicked(move |_| {
                let mut value = settings.borrow().clone();
                value.theme = match value.theme {
                    Theme::System => Theme::Light,
                    Theme::Light => Theme::Dark,
                    Theme::Dark => Theme::System,
                };
                changed(value);
            });
        }
        {
            let dialog = search_dialog.clone();
            search.connect_clicked(move |_| dialog.present());
        }
        {
            let dialog = document_search_dialog.clone();
            let buffer = buffer.clone();
            document_search.connect_clicked(move |_| dialog.present(&buffer));
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
            let reveal_source = view_buttons[1].clone();
            let editor_pane = editor_overlay.clone();
            move |_, row| {
                if !editor_pane.is_visible() {
                    reveal_source.set_active(true);
                }
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
                    callback((paths, String::new()));
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
            let revision = revision.clone();
            let last_edit = last_edit.clone();
            let source_map = source_map.clone();
            let pending_diagnostics = pending_diagnostics.clone();
            let pending_error = pending_error.clone();
            let diagnostics_revealer = diagnostics_revealer.clone();
            let compile_label = compile_label.clone();
            buffer.connect_changed(move |_| {
                revision.set(revision.get().wrapping_add(1));
                for pending in [&pending_diagnostics, &pending_error] {
                    if let Some(timer) = pending.borrow_mut().take() {
                        timer.remove();
                    }
                }
                diagnostics_revealer.set_reveal_child(false);
                source_map.replace(None);
                if !suppress_changes.get() {
                    last_edit.set(Some(Instant::now()));
                    if compile_label.text().starts_with("Compile error:") {
                        compile_label.set_text("Editing…");
                    }
                    dirty.set(true);
                    save_label.set_text("Unsaved changes");
                    if let Some(source) = pending.borrow_mut().take() {
                        source.remove();
                    }
                    let live_compile = settings.borrow().auto_compile;
                    let buffer = callback_buffer.clone();
                    let callbacks = callbacks.clone();
                    let delay = settings.borrow().compile_delay_ms.max(50) as u64;
                    let pending_done = pending.clone();
                    let id = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(delay),
                        move || {
                            pending_done.borrow_mut().take();
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

        for (mode, button) in ViewMode::ALL.into_iter().zip(&view_buttons) {
            let settings = settings.clone();
            let persist = callbacks.preferences_changed.clone();
            let editor_pane = editor_overlay.clone();
            let preview_panel = preview_panel.clone();
            let editor = editor.clone();
            button.connect_toggled(move |button| {
                if !button.is_active() {
                    return;
                }
                editor_pane.set_visible(mode.shows_source());
                preview_panel.set_visible(mode.shows_preview());
                if mode.shows_source() {
                    editor.grab_focus();
                }
                let changed = {
                    let mut settings = settings.borrow_mut();
                    let changed = settings.view_mode != mode;
                    settings.view_mode = mode;
                    changed.then(|| settings.clone())
                };
                if let Some(settings) = changed {
                    persist(settings);
                }
            });
        }

        let view = Self {
            root,
            editor,
            buffer,
            editor_style,
            sidebar,
            file_list,
            file_paths,
            file_rows,
            expanded_directories,
            known_directories,
            preview_pages,
            preview_identity: Rc::new(RefCell::new(None)),
            preview_pictures,
            resize_preview,
            preview_zoom: zoom,
            content_column,
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
            settings_dialog,
            document_search_dialog,
            dirty,
            suppress_changes,
            pending_compile,
            pending_diagnostics,
            pending_error,
            revision,
            last_edit,
            source_map,
            callbacks,
            vim_context,
            vim_status,
            editor_pane: editor_overlay.upcast(),
            editor_badge,
            view_buttons,
        };
        view.install_editor_zoom(&editor_scroll);
        view
    }

    fn install_editor_zoom(&self, editor_scroll: &gtk::ScrolledWindow) {
        let this = self.downgrade_zoom();
        connect_ctrl_scroll(editor_scroll, {
            let this = this.clone();
            move |steps, _, _| {
                if let Some(this) = this.upgrade() {
                    let size = this.settings.borrow().font_size;
                    this.set_editor_font_size(zoom_editor_font(size, steps));
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            use gtk::gdk::Key;
            let modifiers = modifiers & gtk::accelerator_get_default_mod_mask();
            // Shift is tolerated so Ctrl++ works on layouts where + is shifted.
            if !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                || modifiers.intersects(
                    gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SUPER_MASK,
                )
            {
                return glib::Propagation::Proceed;
            }
            let Some(this) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let size = this.settings.borrow().font_size;
            let target = match key {
                Key::plus | Key::equal | Key::KP_Add => zoom_editor_font(size, 1),
                Key::minus | Key::underscore | Key::KP_Subtract => zoom_editor_font(size, -1),
                Key::_0 | Key::KP_0 => DEFAULT_EDITOR_FONT_SIZE,
                _ => return glib::Propagation::Proceed,
            };
            this.set_editor_font_size(target);
            glib::Propagation::Stop
        });
        self.editor.add_controller(keys);
    }

    /// Controllers owned by child widgets must not keep the whole view alive.
    fn downgrade_zoom(&self) -> WeakEditorZoom {
        WeakEditorZoom {
            editor: self.editor.downgrade(),
            editor_style: self.editor_style.clone(),
            settings: Rc::downgrade(&self.settings),
            persist: self.callbacks.preferences_changed.clone(),
            badge: self.editor_badge.clone(),
        }
    }

    pub fn set_project(&self, name: &str, files: &[FileRow]) {
        self.project_label.set_text(name);
        if self.file_rows.borrow().as_slice() == files {
            return;
        }
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
            row.add_css_class("file-row");
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
            more.add_css_class("file-actions");
            more.set_create_popup_func({
                let callbacks = self.callbacks.clone();
                let file = file.clone();
                move |button| {
                    let popover = gtk::Popover::new();
                    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
                    menu.set_margin_top(4);
                    menu.set_margin_bottom(4);
                    menu.set_margin_start(4);
                    menu.set_margin_end(4);
                    let mut actions = vec![
                        ("Rename…", callbacks.rename_path.clone()),
                        ("Reveal in file manager", callbacks.reveal_path.clone()),
                        ("Open with default app", callbacks.open_external.clone()),
                        ("Move to trash", callbacks.trash_path.clone()),
                    ];
                    if !file.is_directory {
                        actions.insert(1, ("Duplicate", callbacks.duplicate_path.clone()));
                        if file.name.to_ascii_lowercase().ends_with(".svg") {
                            actions.insert(2, ("Preview image", callbacks.preview_asset.clone()));
                        }
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
                    button.set_popover(Some(&popover));
                }
            });
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
                let external_drop = gtk::DropTarget::new(
                    gtk::gdk::FileList::static_type(),
                    gtk::gdk::DragAction::COPY,
                );
                external_drop.connect_drop({
                    let target = file.path.clone();
                    let callback = self.callbacks.drop_files.clone();
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
                            callback((paths, target.clone()));
                            true
                        }
                    }
                });
                line.add_controller(external_drop);
            }
            row.set_child(Some(&line));
            self.file_list.append(&row);
        }
        refresh_file_visibility(&self.file_list, files, &self.expanded_directories.borrow());
    }

    pub fn show_text_file(&self, path: &str, contents: &str) {
        self.cancel_pending_compile();
        self.suppress_changes.set(true);
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(contents);
        self.buffer.end_irreversible_action();
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

    pub fn set_current_path(&self, path: &str) {
        self.file_label.set_text(path);
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

    pub fn cancel_pending_compile(&self) {
        for pending in [
            &self.pending_compile,
            &self.pending_diagnostics,
            &self.pending_error,
        ] {
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
            }
        }
    }

    pub fn present_settings(&self) {
        self.settings_dialog.sync(&self.settings.borrow());
        self.settings_dialog.window.present();
    }

    pub fn present_document_search(&self) {
        self.document_search_dialog.present(&self.buffer);
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

    pub fn set_compiled_preview(&self, pages: &[PathBuf], source_path: &str) {
        self.set_preview_content(pages, Some(source_path));
    }

    fn set_preview_content(&self, pages: &[PathBuf], source: Option<&str>) {
        let contents = pages
            .iter()
            .map(|page| std::fs::read_to_string(page).ok())
            .collect::<Vec<_>>();
        let identity = preview_identity(&contents, source);
        // Compiler temp paths change on every run. Compare content and source
        // mapping instead so an unchanged render preserves widgets and scrolling.
        if identity.is_some() && *self.preview_identity.borrow() == identity {
            return;
        }
        let reusable =
            reusable_preview_pages(self.preview_identity.borrow().as_ref(), identity.as_ref());
        self.preview_identity.replace(identity);
        while let Some(child) = self.preview_pages.first_child() {
            self.preview_pages.remove(&child);
        }
        let previous_pictures = std::mem::take(&mut *self.preview_pictures.borrow_mut());
        self.preview_placeholder.set_visible(pages.is_empty());
        self.page_label.set_text(&format!(
            "{} {}",
            pages.len(),
            if pages.len() == 1 { "page" } else { "pages" }
        ));
        for (index, page) in pages.iter().enumerate() {
            if reusable.get(index) == Some(&true) {
                if let Some(picture) = previous_pictures.get(index) {
                    self.preview_pages.append(&picture.sheet);
                    self.preview_pictures.borrow_mut().push(picture.clone());
                    continue;
                }
            }
            let svg = &contents[index];
            let (page_width, aspect_ratio) = svg.as_deref().and_then(svg_page_size).map_or(
                (FALLBACK_PAGE_WIDTH, DEFAULT_PREVIEW_ASPECT),
                |(width, height)| (width, width / height),
            );
            let links = svg
                .as_deref()
                .map(extract_external_links)
                .unwrap_or_default();
            let sheet = gtk::Box::new(gtk::Orientation::Vertical, 8);
            sheet.add_css_class("page-sheet");
            if source.is_some() {
                let page_number = gtk::Label::new(Some(&format!("Page {}", index + 1)));
                page_number.add_css_class("muted");
                page_number.set_halign(gtk::Align::Start);
                page_number.set_margin_start(8);
                sheet.append(&page_number);
            }
            let picture = gtk::Picture::new();
            if source.is_some() && !reusable.is_empty() {
                if let Some(previous) = previous_pictures
                    .get(index)
                    .and_then(|previous| previous.widget.paintable())
                {
                    picture.set_paintable(Some(&previous.current_image()));
                }
            }
            super::page_paintable::load(&picture, page);
            if picture.paintable().is_none() {
                self.preview_identity.replace(None);
            }
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            if source.is_some() {
                let edit = gtk::GestureClick::new();
                edit.set_button(1);
                edit.connect_released({
                    let select = self.callbacks.select_file.clone();
                    let source_map = self.source_map.clone();
                    let buffer = self.buffer.clone();
                    let editor = self.editor.clone();
                    let picture = picture.downgrade();
                    let status = self.compile_label.clone();
                    let file_label = self.file_label.clone();
                    let reveal_source = self.view_buttons[1].clone();
                    let editor_pane = self.editor_pane.clone();
                    move |_, press_count, x, y| {
                        if press_count != 1 {
                            return;
                        }
                        let Some(picture) = picture.upgrade() else {
                            return;
                        };
                        if !super::page_paintable::is_current(&picture) {
                            return;
                        }
                        // Release the RefCell borrow before selecting a file: that
                        // can synchronously compile or replace the editor buffer.
                        let map = source_map.borrow().clone();
                        let Some(map) = map else {
                            status.set_text("Waiting for the updated preview…");
                            return;
                        };
                        let Some((_, page_height)) = map.dimensions(index) else {
                            return;
                        };
                        let Some((x, y)) = preview_point(
                            f64::from(picture.width()),
                            f64::from(picture.height()),
                            aspect_ratio,
                            picture.content_fit() == gtk::ContentFit::Cover,
                            page_height,
                            x,
                            y,
                        ) else {
                            return;
                        };
                        if let Some(location) = map.jump(index, x, y) {
                            // Jumping to source from a preview-only layout needs the editor.
                            if !editor_pane.is_visible() {
                                reveal_source.set_active(true);
                            }
                            if file_label.text().as_str() != location.path {
                                select(location.path.clone());
                            }
                            // Selection may be refused while resolving an external conflict.
                            if file_label.text().as_str() != location.path {
                                return;
                            }
                            if let Some(mut target) =
                                buffer.iter_at_line(location.line.saturating_sub(1) as i32)
                            {
                                let available = target.chars_in_line().saturating_sub(1).max(0);
                                target.forward_chars(
                                    (location.column.saturating_sub(1) as i32).min(available),
                                );
                                buffer.place_cursor(&target);
                                editor.scroll_to_iter(&mut target, 0.15, false, 0.0, 0.25);
                                editor.grab_focus();
                            }
                        }
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
                sheet: sheet.clone(),
                widget: picture,
                aspect_ratio,
                page_width,
            });
            self.preview_pages.append(&sheet);
        }
        (self.resize_preview)();
        let resize = self.resize_preview.clone();
        glib::idle_add_local_once(move || resize());
    }

    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    fn diagnostic_delay(&self) -> Duration {
        self.last_edit.get().map_or(Duration::ZERO, |edit| {
            Duration::from_millis(900).saturating_sub(edit.elapsed())
        })
    }

    pub fn set_source_map(&self, source_map: Option<Arc<SourceMap>>) {
        if let Some(map) = &source_map {
            let column = map.content_column();
            if self.content_column.replace(column) != column
                && self.preview_zoom.get() == PreviewZoom::FitText
            {
                (self.resize_preview)();
            }
        }
        self.source_map.replace(source_map);
    }

    pub fn set_compile_error(&self, text: &str) {
        if let Some(timer) = self.pending_error.borrow_mut().take() {
            timer.remove();
        }
        let this = self.clone();
        let text = text.to_string();
        let timer = glib::timeout_add_local_once(self.diagnostic_delay(), move || {
            this.pending_error.borrow_mut().take();
            this.set_compile_status(&text);
        });
        self.pending_error.replace(Some(timer));
    }

    pub fn set_diagnostics(&self, diagnostics: &[DiagnosticRow]) {
        if let Some(timer) = self.pending_diagnostics.borrow_mut().take() {
            timer.remove();
        }
        if let Some(timer) = self.pending_error.borrow_mut().take() {
            timer.remove();
        }
        if diagnostics.is_empty() {
            self.render_diagnostics(diagnostics);
            return;
        }
        let this = self.clone();
        let diagnostics = diagnostics.to_vec();
        let timer = glib::timeout_add_local_once(self.diagnostic_delay(), move || {
            this.pending_diagnostics.borrow_mut().take();
            this.render_diagnostics(&diagnostics);
        });
        self.pending_diagnostics.replace(Some(timer));
    }

    fn render_diagnostics(&self, diagnostics: &[DiagnosticRow]) {
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
        let dark = match settings.theme {
            Theme::Dark => true,
            Theme::Light => false,
            Theme::System => adw::StyleManager::default().is_dark(),
        };
        // GtkSourceView's buffer scheme controls text-node background, syntax,
        // selection and gutter separately from the surrounding GTK theme.
        let schemes = sourceview5::StyleSchemeManager::default();
        let preferred: &[&str] = if dark {
            &["typsmthng-dark", "Adwaita-dark", "classic-dark"]
        } else {
            &["typsmthng-light", "Adwaita", "classic"]
        };
        let scheme = preferred.iter().find_map(|id| schemes.scheme(id));
        self.buffer.set_style_scheme(scheme.as_ref());
        self.editor_style
            .load_from_string(&editor_css(settings.font_size));
        if settings.vim_mode && self.vim_context.borrow().is_none() {
            let vim = sourceview5::VimIMContext::new();
            vim.set_client_widget(Some(&self.editor));
            vim.connect_command_bar_text_notify({
                let status = self.vim_status.clone();
                move |vim| status.set_text(&vim.command_bar_text())
            });
            vim.connect_execute_command({
                let buffer = self.buffer.clone();
                let save = self.callbacks.save.clone();
                let editor = self.editor.downgrade();
                move |_, command| {
                    let command = command.trim().trim_start_matches(':');
                    match command {
                        "w" | "write" => {
                            save(buffer_text(&buffer));
                            true
                        }
                        "q" | "quit" | "wq" | "x" => {
                            // A failed write must never be followed by closing the app.
                            if save(buffer_text(&buffer)) {
                                if let Some(editor) = editor.upgrade() {
                                    let _ = editor.activate_action("app.quit", None);
                                }
                            }
                            true
                        }
                        _ => false,
                    }
                }
            });
            self.vim_status.set_text(&vim.command_bar_text());
            self.vim_context.replace(Some(vim));
        } else if !settings.vim_mode {
            self.vim_context.replace(None);
        }
        self.vim_status.set_visible(settings.vim_mode);
        let view_mode = settings.view_mode;
        self.settings.replace(settings);
        self.set_view_mode(view_mode);
    }

    pub fn set_view_mode(&self, mode: ViewMode) {
        if let Some(index) = ViewMode::ALL
            .iter()
            .position(|candidate| *candidate == mode)
        {
            self.view_buttons[index].set_active(true);
        }
    }
}

#[derive(Clone)]
struct WeakEditorZoom {
    editor: glib::WeakRef<sourceview5::View>,
    editor_style: gtk::CssProvider,
    settings: std::rc::Weak<RefCell<UiSettings>>,
    persist: Rc<dyn Fn(UiSettings)>,
    badge: ZoomBadge,
}

struct EditorZoom {
    editor: sourceview5::View,
    editor_style: gtk::CssProvider,
    settings: Rc<RefCell<UiSettings>>,
    persist: Rc<dyn Fn(UiSettings)>,
    badge: ZoomBadge,
}

impl WeakEditorZoom {
    fn upgrade(&self) -> Option<EditorZoom> {
        Some(EditorZoom {
            editor: self.editor.upgrade()?,
            editor_style: self.editor_style.clone(),
            settings: self.settings.upgrade()?,
            persist: self.persist.clone(),
            badge: self.badge.clone(),
        })
    }
}

impl EditorZoom {
    fn set_editor_font_size(&self, size: u32) {
        self.badge.show(&format!("{size} pt"));
        let settings = {
            let mut settings = self.settings.borrow_mut();
            if settings.font_size == size {
                return;
            }
            settings.font_size = size;
            settings.clone()
        };
        // Keep the first visible line in place while the text reflows.
        let visible = self.editor.visible_rect();
        let anchor = self
            .editor
            .iter_at_location(visible.x(), visible.y())
            .map(|iter| self.editor.buffer().create_mark(None, &iter, true));
        self.editor_style.load_from_string(&editor_css(size));
        if let Some(anchor) = anchor {
            let editor = self.editor.clone();
            glib::idle_add_local_once(move || {
                editor.scroll_to_mark(&anchor, 0.0, true, 0.0, 0.0);
                editor.buffer().delete_mark(&anchor);
            });
        }
        (self.persist)(settings);
    }
}

fn editor_css(font_size: u32) -> String {
    format!(".typst-editor {{ font-size: {font_size}pt; }}")
}

// Tick callbacks run before allocation. The second frame sees the first
// frame's allocation; always remove the callback after that bounded wait.
fn schedule_after_allocation(widget: &gtk::Box, pending: Rc<Cell<bool>>, callback: Rc<dyn Fn()>) {
    if pending.replace(true) {
        return;
    }
    let allocated_once = Cell::new(false);
    widget.add_tick_callback(move |_, _| {
        if !allocated_once.replace(true) {
            return glib::ControlFlow::Continue;
        }
        pending.set(false);
        callback();
        glib::ControlFlow::Break
    });
}

fn buffer_text(buffer: &sourceview5::Buffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn install_pair_completion(
    editor: &sourceview5::View,
    buffer: &sourceview5::Buffer,
    vim_context: Rc<RefCell<Option<sourceview5::VimIMContext>>>,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let buffer = buffer.clone();
        move |_, key, _, modifiers| {
            if vim_context.borrow().is_some()
                || modifiers.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK
                        | gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::SUPER_MASK,
                )
            {
                return glib::Propagation::Proceed;
            }

            let cursor = buffer.iter_at_mark(&buffer.get_insert());
            if key == gtk::gdk::Key::BackSpace {
                if buffer.has_selection() {
                    return glib::Propagation::Proceed;
                }
                let mut before = cursor;
                let mut after = cursor;
                if before.backward_char() && after.forward_char() {
                    let pair = (before.char(), cursor.char());
                    if matches!(
                        pair,
                        ('(', ')') | ('[', ']') | ('{', '}') | ('\'', '\'') | ('"', '"')
                    ) {
                        buffer.begin_user_action();
                        buffer.delete(&mut before, &mut after);
                        buffer.end_user_action();
                        return glib::Propagation::Stop;
                    }
                }
                return glib::Propagation::Proceed;
            }

            let Some(character) = key.to_unicode() else {
                return glib::Propagation::Proceed;
            };
            if !buffer.has_selection()
                && matches!(character, ')' | ']' | '}')
                && cursor.char() == character
            {
                let mut next = cursor;
                next.forward_char();
                buffer.place_cursor(&next);
                return glib::Propagation::Stop;
            }
            let closing = match character {
                '(' => ')',
                '[' => ']',
                '{' => '}',
                '\'' => '\'',
                '"' => '"',
                _ => return glib::Propagation::Proceed,
            };
            let start_offset = buffer
                .selection_bounds()
                .map(|(start, _)| start.offset())
                .unwrap_or_else(|| cursor.offset());
            let selected = buffer
                .selection_bounds()
                .map(|(start, end)| buffer.text(&start, &end, true).to_string())
                .unwrap_or_default();
            let replacement = format!("{character}{selected}{closing}");
            buffer.begin_user_action();
            if let Some((mut start, mut end)) = buffer.selection_bounds() {
                buffer.delete(&mut start, &mut end);
                buffer.insert(&mut start, &replacement);
            } else {
                buffer.insert_at_cursor(&replacement);
            }
            buffer.end_user_action();
            let inside_start = buffer.iter_at_offset(start_offset + 1);
            let inside_end =
                buffer.iter_at_offset(start_offset + 1 + selected.chars().count() as i32);
            if selected.is_empty() {
                buffer.place_cursor(&inside_start);
            } else {
                buffer.select_range(&inside_start, &inside_end);
            }
            glib::Propagation::Stop
        }
    });
    editor.add_controller(keys);
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

/// Candidate directories for bundled GtkSourceView data (`language-specs`,
/// `styles`): macOS bundle resources, installed Linux/Windows prefixes, then
/// the source tree for development builds.
fn data_search_paths(name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            paths.push(directory.join("../Resources").join(name));
            paths.push(directory.join("../share/typsmthng").join(name));
            paths.push(directory.join("share/typsmthng").join(name));
        }
    }
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join(name),
    );
    paths
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

#[derive(Debug, PartialEq, Eq)]
struct PreviewSize {
    width: i32,
    height: i32,
    /// The widget shows a centred strip of the page (`ContentFit::Cover`).
    cropped: bool,
}

fn preview_dimensions(
    viewport_width: i32,
    zoom: PreviewZoom,
    page_width: f64,
    aspect_ratio: f64,
    content_column: Option<(f64, f64)>,
) -> PreviewSize {
    let aspect_ratio = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
        aspect_ratio
    } else {
        DEFAULT_PREVIEW_ASPECT
    };
    let page_width = if page_width.is_finite() && page_width > 0.0 {
        page_width
    } else {
        FALLBACK_PAGE_WIDTH
    };
    let available = viewport_width - PREVIEW_HORIZONTAL_INSET;
    let (width, visible_width) = match (zoom, content_column) {
        (PreviewZoom::FitWidth | PreviewZoom::FitText, _) if available <= 0 => {
            (DEFAULT_PREVIEW_WIDTH, page_width)
        }
        (PreviewZoom::FitText, Some(column)) => {
            (f64::from(available), fit_text_crop(page_width, column))
        }
        (PreviewZoom::FitWidth | PreviewZoom::FitText, _) => (f64::from(available), page_width),
        (PreviewZoom::Scale(scale), _) => (page_width * PIXELS_PER_POINT * scale, page_width),
    };
    let width = width.max(48.0);
    // The full page height at the scale that makes the visible strip `width`.
    let height = width * page_width / visible_width / aspect_ratio;
    PreviewSize {
        width: width.round() as i32,
        height: height.round() as i32,
        cropped: visible_width < page_width,
    }
}

#[cfg(test)]
fn svg_aspect_ratio(svg: &str) -> Option<f64> {
    svg_page_size(svg).map(|(width, height)| width / height)
}

/// The root viewBox (or width/height) size; Typst emits points.
fn svg_page_size(svg: &str) -> Option<(f64, f64)> {
    let root_start = svg.find("<svg")?;
    let root_end = svg[root_start..].find('>')? + root_start;
    let svg = &svg[root_start..=root_end];
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
            return Some((width, height));
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
        .then_some((width, height))
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

/// Account for GtkPicture's centered `Contain` sizing and cropped slide widths.
fn preview_point(
    width: f64,
    height: f64,
    aspect: f64,
    cropped: bool,
    page_height: f64,
    x: f64,
    y: f64,
) -> Option<(f64, f64)> {
    if width <= 0.0 || height <= 0.0 || aspect <= 0.0 {
        return None;
    }
    // Contain letterboxes the page; Cover overflows and crops it centrally.
    let drawn_height = if cropped {
        height.max(width / aspect)
    } else {
        height.min(width / aspect)
    };
    let drawn_width = drawn_height * aspect;
    let x = x - (width - drawn_width) / 2.0;
    let y = y - (height - drawn_height) / 2.0;
    if x < 0.0 || y < 0.0 || x > drawn_width || y > drawn_height {
        return None;
    }
    let scale = page_height / drawn_height;
    Some((x * scale, y * scale))
}

fn build_document_search_dialog(
    parent: &gtk::ApplicationWindow,
    buffer: &sourceview5::Buffer,
    editor: &sourceview5::View,
) -> DocumentSearchDialog {
    let window = gtk::Window::builder()
        .title("Find and replace — typsmthng")
        .transient_for(parent)
        .modal(false)
        .resizable(false)
        .hide_on_close(true)
        .default_width(540)
        .build();
    super::dismiss_on_escape(&window);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(14);
    root.set_margin_bottom(14);
    root.set_margin_start(14);
    root.set_margin_end(14);

    let find = gtk::Entry::new();
    find.set_placeholder_text(Some("Find in current file"));
    let replace = gtk::Entry::new();
    replace.set_placeholder_text(Some("Replace with"));
    let case_sensitive = gtk::CheckButton::with_label("Match case");
    let status = gtk::Label::new(None);
    status.add_css_class("muted");
    status.set_halign(gtk::Align::Start);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let previous = gtk::Button::with_label("Previous");
    let next = gtk::Button::with_label("Next");
    let replace_one = gtk::Button::with_label("Replace");
    let replace_all = gtk::Button::with_label("Replace all");
    controls.append(&previous);
    controls.append(&next);
    controls.append(&replace_one);
    controls.append(&replace_all);
    controls.append(&case_sensitive);
    root.append(&find);
    root.append(&replace);
    root.append(&controls);
    root.append(&status);
    window.set_child(Some(&root));

    let select_match: Rc<dyn Fn(bool)> = {
        let buffer = buffer.clone();
        let editor = editor.clone();
        let find = find.clone();
        let case_sensitive = case_sensitive.clone();
        let status = status.clone();
        Rc::new(move |backwards| {
            let query = find.text();
            if query.is_empty() {
                status.set_text("Enter text to find");
                return;
            }
            match select_document_match(&buffer, &query, case_sensitive.is_active(), backwards) {
                Ok(true) => {
                    status.set_text("");
                    editor.grab_focus();
                }
                Ok(false) => status.set_text("No matches"),
                Err(error) => status.set_text(&error),
            }
        })
    };
    next.connect_clicked({
        let select_match = select_match.clone();
        move |_| select_match(false)
    });
    previous.connect_clicked({
        let select_match = select_match.clone();
        move |_| select_match(true)
    });
    find.connect_activate({
        let select_match = select_match.clone();
        move |_| select_match(false)
    });
    replace_one.connect_clicked({
        let buffer = buffer.clone();
        let find = find.clone();
        let replace = replace.clone();
        let case_sensitive = case_sensitive.clone();
        let select_match = select_match.clone();
        move |_| {
            if let Some((mut start, mut end)) = buffer.selection_bounds() {
                let selected = buffer.text(&start, &end, true);
                if document_regex(&find.text(), case_sensitive.is_active()).is_ok_and(|regex| {
                    regex
                        .find(&selected)
                        .is_some_and(|item| item.range() == (0..selected.len()))
                }) {
                    buffer.begin_user_action();
                    buffer.delete(&mut start, &mut end);
                    buffer.insert(&mut start, &replace.text());
                    buffer.end_user_action();
                }
            }
            select_match(false);
        }
    });
    replace_all.connect_clicked({
        let buffer = buffer.clone();
        let find = find.clone();
        let replace = replace.clone();
        let case_sensitive = case_sensitive.clone();
        let status = status.clone();
        move |_| {
            let query = find.text();
            let Ok(regex) = document_regex(&query, case_sensitive.is_active()) else {
                status.set_text("Enter text to find");
                return;
            };
            let source = buffer_text(&buffer);
            let count = regex.find_iter(&source).count();
            if count == 0 {
                status.set_text("No matches");
                return;
            }
            let ranges = document_match_offsets(&regex, &source);
            buffer.begin_user_action();
            for (start, end) in ranges.into_iter().rev() {
                let mut start = buffer.iter_at_offset(start);
                let mut end = buffer.iter_at_offset(end);
                buffer.delete(&mut start, &mut end);
                buffer.insert(&mut start, &replace.text());
            }
            buffer.end_user_action();
            status.set_text(&format!("Replaced {count} match(es)"));
        }
    });

    DocumentSearchDialog { window, find }
}

fn document_regex(query: &str, case_sensitive: bool) -> Result<Regex, String> {
    if query.is_empty() {
        return Err("empty search".into());
    }
    regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| error.to_string())
}

// Convert UTF-8 regex ranges to GTK character offsets in one forward pass.
// Counting every source prefix separately is quadratic for repeated matches.
fn document_match_offsets(regex: &Regex, source: &str) -> Vec<(i32, i32)> {
    let mut byte_offset = 0;
    let mut char_offset = 0_i32;
    regex
        .find_iter(source)
        .map(|item| {
            char_offset += source[byte_offset..item.start()].chars().count() as i32;
            let start = char_offset;
            char_offset += source[item.start()..item.end()].chars().count() as i32;
            byte_offset = item.end();
            (start, char_offset)
        })
        .collect()
}

fn select_document_match(
    buffer: &sourceview5::Buffer,
    query: &str,
    case_sensitive: bool,
    backwards: bool,
) -> Result<bool, String> {
    let source = buffer_text(buffer);
    let regex = document_regex(query, case_sensitive)?;
    let matches = document_match_offsets(&regex, &source);
    if matches.is_empty() {
        return Ok(false);
    }
    let cursor = buffer.iter_at_mark(&buffer.get_insert()).offset();
    let selected = if backwards {
        matches
            .iter()
            .rev()
            .find(|(_, end)| *end < cursor)
            .unwrap_or_else(|| matches.last().expect("non-empty matches"))
    } else {
        matches
            .iter()
            .find(|(start, _)| *start >= cursor)
            .unwrap_or_else(|| matches.first().expect("non-empty matches"))
    };
    let (start_offset, end_offset) = *selected;
    let start = buffer.iter_at_offset(start_offset);
    let end = buffer.iter_at_offset(end_offset);
    buffer.select_range(&start, &end);
    Ok(true)
}

fn build_settings_dialog(
    parent: &gtk::ApplicationWindow,
    settings: Rc<RefCell<UiSettings>>,
    on_changed: Rc<dyn Fn(UiSettings)>,
) -> SettingsDialog {
    let dialog = gtk::Window::builder()
        .title("Settings — typsmthng")
        .transient_for(parent)
        .modal(true)
        .default_width(620)
        .default_height(520)
        .hide_on_close(true)
        .build();
    super::dismiss_on_escape(&dialog);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some("Settings"))));
    dialog.set_titlebar(Some(&header));
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 12);
    rows.set_margin_top(20);
    rows.set_margin_bottom(20);
    rows.set_margin_start(22);
    rows.set_margin_end(22);
    let group = adw::PreferencesGroup::builder().title("Editor").build();
    rows.append(&group);
    let font = gtk::SpinButton::with_range(
        f64::from(*EDITOR_FONT_SIZES.start()),
        f64::from(*EDITOR_FONT_SIZES.end()),
        1.0,
    );
    font.set_value(settings.borrow().font_size as f64);
    group.add(&setting_row(
        "Editor font size",
        "Ctrl+scroll or Ctrl+= / Ctrl+- in the editor",
        &font,
    ));
    let line_numbers = gtk::Switch::new();
    line_numbers.set_active(settings.borrow().line_numbers);
    group.add(&setting_row(
        "Line numbers",
        "Show the source gutter",
        &line_numbers,
    ));
    let wrapping = gtk::Switch::new();
    wrapping.set_active(settings.borrow().line_wrapping);
    group.add(&setting_row(
        "Line wrapping",
        "Wrap long source lines",
        &wrapping,
    ));
    let vim = gtk::Switch::new();
    vim.set_active(settings.borrow().vim_mode);
    group.add(&setting_row(
        "Vim input",
        "Use GtkSourceView's native Vim mode",
        &vim,
    ));
    let group = adw::PreferencesGroup::builder()
        .title("Compilation")
        .build();
    rows.append(&group);
    let auto_compile = gtk::Switch::new();
    auto_compile.set_active(settings.borrow().auto_compile);
    group.add(&setting_row(
        "Live compile",
        "Render after source changes",
        &auto_compile,
    ));
    let delay = gtk::SpinButton::with_range(100.0, 2000.0, 50.0);
    delay.set_value(settings.borrow().compile_delay_ms as f64);
    group.add(&setting_row(
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
    group.add(&setting_row(
        "Theme",
        "Native light or dark palette",
        &theme,
    ));
    let auto_label = format!("Auto ({})", page_size_label(super::locale_page_size()));
    let page_size_labels = std::iter::once(auto_label.as_str())
        .chain(PAGE_SIZES.iter().map(|(_, label)| *label))
        .collect::<Vec<_>>();
    let page_size = gtk::DropDown::from_strings(&page_size_labels);
    page_size.set_selected(page_size_index(&settings.borrow().page_size));
    group.add(&setting_row(
        "Page size",
        "Auto follows your region's paper; #set page overrides",
        &page_size,
    ));
    let notes_layout =
        gtk::DropDown::from_strings(&["Auto detect", "Notes on right half", "Whole page"]);
    notes_layout.set_selected(match settings.borrow().presentation_notes_layout.as_str() {
        "right-half" => 1,
        "whole" => 2,
        _ => 0,
    });
    group.add(&setting_row(
        "Presenter notes layout",
        "Override automatic splitting for ultrawide documents",
        &notes_layout,
    ));
    let system_fonts = gtk::Switch::new();
    system_fonts.set_active(settings.borrow().system_fonts);
    group.add(&setting_row(
        "System fonts",
        "Expose installed fonts to Typst",
        &system_fonts,
    ));
    let google_fonts = gtk::Switch::new();
    google_fonts.set_active(settings.borrow().google_fonts);
    group.add(&setting_row(
        "Google Fonts",
        "Allow missing font download",
        &google_fonts,
    ));
    let translucent = gtk::Switch::new();
    translucent.set_active(settings.borrow().translucent);
    group.add(&setting_row(
        "Translucent chrome",
        "Use compositor transparency where supported",
        &translucent,
    ));
    let apply = gtk::Button::with_label("Apply settings");
    apply.add_css_class("suggested-action");
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.set_halign(gtk::Align::End);
    footer.set_margin_top(12);
    footer.set_margin_bottom(12);
    footer.set_margin_start(22);
    footer.set_margin_end(22);
    footer.append(&apply);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&rows));
    scroll.set_vexpand(true);
    root.append(&scroll);
    root.append(&footer);
    dialog.set_child(Some(&root));

    let settings_dialog = SettingsDialog {
        window: dialog.clone(),
        font: font.clone(),
        line_numbers: line_numbers.clone(),
        wrapping: wrapping.clone(),
        vim: vim.clone(),
        auto_compile: auto_compile.clone(),
        delay: delay.clone(),
        theme: theme.clone(),
        page_size: page_size.clone(),
        notes_layout: notes_layout.clone(),
        system_fonts: system_fonts.clone(),
        google_fonts: google_fonts.clone(),
        translucent: translucent.clone(),
    };

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
                page_size: (page_size.selected() as usize)
                    .checked_sub(1)
                    .and_then(|index| PAGE_SIZES.get(index))
                    .map_or("auto", |(id, _)| id)
                    .into(),
                presentation_notes_layout: match notes_layout.selected() {
                    1 => "right-half",
                    2 => "whole",
                    _ => "auto",
                }
                .into(),
                system_fonts: system_fonts.is_active(),
                google_fonts: google_fonts.is_active(),
                translucent: translucent.is_active(),
                ..settings.borrow().clone()
            };
            settings.replace(value.clone());
            on_changed(value);
            dialog.set_visible(false);
        }
    });
    settings_dialog
}

fn page_size_index(id: &str) -> u32 {
    PAGE_SIZES
        .iter()
        .position(|(candidate, _)| *candidate == id)
        .map_or(0, |index| index as u32 + 1)
}

fn setting_row(label: &str, detail: &str, control: &impl IsA<gtk::Widget>) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(label)
        .subtitle(detail)
        .build();
    control.set_valign(gtk::Align::Center);
    row.add_suffix(control);
    row.set_activatable_widget(Some(control));
    row
}

fn build_search_dialog(
    parent: &gtk::ApplicationWindow,
    search: SearchCallback,
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
    super::dismiss_on_escape(&dialog);
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

    let generation = Rc::new(Cell::new(0_u64));
    let refresh: Rc<dyn Fn()> = {
        let entry = entry.clone();
        let files = files.clone();
        let contents = contents.clone();
        let results = results.clone();
        let result_paths = result_paths.clone();
        let generation = generation.clone();
        Rc::new(move || {
            let request = generation.get().wrapping_add(1);
            generation.set(request);
            let mode = if files.is_active() {
                SearchMode::Files
            } else if contents.is_active() {
                SearchMode::Contents
            } else {
                SearchMode::Commands
            };
            let results = results.clone();
            let result_paths = result_paths.clone();
            let generation = generation.clone();
            search(
                mode,
                entry.text().to_string(),
                Rc::new(move |rows| {
                    if generation.get() != request {
                        return;
                    }
                    while let Some(child) = results.first_child() {
                        results.remove(&child);
                    }
                    result_paths.borrow_mut().clear();
                    for result in rows.into_iter().take(200) {
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
                        primary.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        row.append(&primary);
                        let secondary = gtk::Label::new(Some(&result.secondary));
                        secondary.set_halign(gtk::Align::Start);
                        secondary.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        secondary.add_css_class("muted");
                        row.append(&secondary);
                        results.append(&row);
                    }
                }),
            );
        })
    };
    let pending_search = Rc::new(RefCell::new(None::<glib::SourceId>));
    entry.connect_changed({
        let generation = generation.clone();
        let refresh = refresh.clone();
        let pending = pending_search.clone();
        move |_| {
            generation.set(generation.get().wrapping_add(1));
            if let Some(source) = pending.borrow_mut().take() {
                source.remove();
            }
            let refresh = refresh.clone();
            let pending_done = pending.clone();
            let source =
                glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                    pending_done.borrow_mut().take();
                    refresh();
                });
            pending.replace(Some(source));
        }
    });
    for toggle in [&files, &contents, &commands] {
        toggle.connect_toggled({
            let refresh = refresh.clone();
            move |button| {
                if button.is_active() {
                    refresh();
                }
            }
        });
    }
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
    dialog.connect_hide({
        let generation = generation.clone();
        move |_| {
            generation.set(generation.get().wrapping_add(1));
            if let Some(source) = pending_search.borrow_mut().take() {
                source.remove();
            }
        }
    });
    dialog.connect_show(move |_| {
        entry.grab_focus();
        refresh();
    });
    dialog
}

#[cfg(test)]
mod tests {
    use super::{
        document_match_offsets, document_regex, extract_external_links, preview_dimensions,
        preview_identity, preview_point, reusable_preview_pages, should_uncomment_lines,
        svg_aspect_ratio, PreviewZoom,
    };

    #[test]
    fn preview_reuses_only_unchanged_pages_with_stable_source_mapping() {
        let before = preview_identity(&[Some("one".into()), Some("two".into())], Some("main.typ"));
        let after = preview_identity(
            &[Some("one".into()), Some("changed".into())],
            Some("main.typ"),
        );
        assert_eq!(
            reusable_preview_pages(before.as_ref(), after.as_ref()),
            vec![true, false]
        );
        let different_mapping =
            preview_identity(&[Some("one".into()), Some("two".into())], Some("other.typ"));
        assert!(reusable_preview_pages(before.as_ref(), different_mapping.as_ref()).is_empty());
        let additional_page = preview_identity(
            &[Some("one".into()), Some("two".into()), Some("three".into())],
            Some("main.typ"),
        );
        assert!(reusable_preview_pages(before.as_ref(), additional_page.as_ref()).is_empty());
        assert!(reusable_preview_pages(None, after.as_ref()).is_empty());
    }

    #[test]
    fn preview_reuse_requires_identical_content_order_and_source_mapping() {
        let pages = vec![Some("<svg>one</svg>".into()), Some("<svg>two</svg>".into())];
        let first = preview_identity(&pages, Some("main.typ"));
        assert_eq!(first, preview_identity(&pages, Some("main.typ")));
        assert_ne!(first, preview_identity(&pages, Some("other.typ")));
        assert_ne!(first, preview_identity(&pages, None));
        let reversed = pages.into_iter().rev().collect::<Vec<_>>();
        assert_ne!(first, preview_identity(&reversed, Some("main.typ")));
        assert!(preview_identity(&[None], None).is_none());
        assert_eq!(
            svg_aspect_ratio(r#"<svg><rect width="50" height="100"/></svg>"#),
            None
        );
    }

    #[test]
    fn document_matches_use_unicode_character_offsets() {
        let regex = document_regex("é", true).unwrap();
        assert_eq!(
            document_match_offsets(&regex, "é🙂 éé"),
            vec![(0, 1), (3, 4), (4, 5)]
        );
        let source = "é ".repeat(10_000);
        let matches = document_match_offsets(&regex, &source);
        assert_eq!(matches.len(), 10_000);
        assert_eq!(matches.last(), Some(&(19_998, 19_999)));
    }

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
        let size = |viewport, zoom, page_width, aspect| {
            let size = preview_dimensions(viewport, zoom, page_width, aspect, None);
            (size.width, size.height)
        };
        let fit = PreviewZoom::FitWidth;
        assert_eq!(size(900, fit, 595.0, 2.0), (836, 418));
        assert_eq!(size(900, PreviewZoom::Scale(1.5), 420.0, 2.0), (840, 420));
        // 100% is the page's physical size at 96 DPI: A4 is 794 px wide.
        assert_eq!(
            size(900, PreviewZoom::Scale(1.0), 595.28, 595.28 / 841.89).0,
            794
        );
        assert_eq!(size(0, fit, 595.0, 2.0), (560, 280));
        assert_eq!(size(900, PreviewZoom::FitText, 595.0, 2.0), (836, 418));
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
    fn fit_text_crops_margins_and_keeps_full_page_height() {
        let a4 = (595.28, 841.89);
        let column = (70.87, 524.41);
        let size = preview_dimensions(436, PreviewZoom::FitText, a4.0, a4.0 / a4.1, Some(column));
        assert!(size.cropped);
        assert_eq!(size.width, 372);
        let crop = super::fit_text_crop(a4.0, column);
        let expected = 372.0 * a4.0 / crop * a4.1 / a4.0;
        assert_eq!(size.height, expected.round() as i32);
        // A click on the strip's left edge lands on the column's left side.
        let (x, _) = preview_point(
            f64::from(size.width),
            f64::from(size.height),
            a4.0 / a4.1,
            true,
            a4.1,
            0.0,
            10.0,
        )
        .unwrap();
        assert!((x - (a4.0 - crop) / 2.0).abs() < 0.5);
    }

    #[test]
    fn clicks_account_for_letterboxing_zoom_and_cropped_pages() {
        assert_eq!(
            preview_point(800.0, 600.0, 2.0, false, 400.0, 200.0, 200.0),
            Some((200.0, 100.0))
        );
        assert_eq!(
            preview_point(800.0, 600.0, 2.0, false, 400.0, 200.0, 50.0),
            None
        );
        assert_eq!(
            preview_point(400.0, 200.0, 2.0, false, 400.0, 100.0, 50.0),
            Some((200.0, 100.0))
        );
        assert_eq!(preview_point(0.0, 0.0, 2.0, false, 400.0, 0.0, 0.0), None);
    }
    #[test]
    #[ignore = "requires a display; run under xvfb-run with --test-threads=1"]
    fn native_preview_clicks_and_diagnostic_idle_timing() {
        use super::*;
        use typsmthng_gtk::backend::{preview::PreviewCompiler, CompileOptions, Project};
        adw::init().unwrap();
        let application = gtk::Application::builder()
            .application_id("dev.typsmthng.PreviewTest")
            .build();
        application.register(None::<&gio::Cancellable>).unwrap();
        let window = gtk::ApplicationWindow::builder()
            .application(&application)
            .default_width(1200)
            .default_height(800)
            .build();
        let noop: Rc<dyn Fn()> = Rc::new(|| {});
        let path_noop: Rc<dyn Fn(String)> = Rc::new(|_| {});
        let workspace = WorkspaceView::new(
            &window,
            WorkspaceCallbacks {
                go_home: noop.clone(),
                open_project: noop.clone(),
                save: Rc::new(|_| true),
                force_save: Rc::new(|_| true),
                select_file: path_noop.clone(),
                create_file: noop.clone(),
                create_folder: noop.clone(),
                import_files: noop.clone(),
                drop_files: Rc::new(|_| {}),
                move_path: Rc::new(|_| {}),
                toggle_hidden: noop.clone(),
                rename_path: path_noop.clone(),
                duplicate_path: path_noop.clone(),
                trash_path: path_noop.clone(),
                reveal_path: path_noop.clone(),
                open_external: path_noop.clone(),
                preview_asset: path_noop.clone(),
                check_update: noop.clone(),
                export_pdf: noop.clone(),
                export_project: noop.clone(),
                present_single: noop.clone(),
                present_dual: noop,
                refresh_compile: path_noop,
                search: Rc::new(|_, _, reply| reply(Vec::new())),
                settings_changed: Rc::new(|_| {}),
                preferences_changed: Rc::new(|_| {}),
            },
        );
        window.set_child(Some(&workspace.root));
        window.present();
        let drive = |duration| {
            let until = Instant::now() + duration;
            let context = glib::MainContext::default();
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let dir = tempfile::tempdir().unwrap();
        let source = format!(
            "#set page(width: 240pt, height: 180pt, margin: 20pt)\n{}$ integral_0^1 x dif x $",
            "// comment\n".repeat(25)
        );
        std::fs::write(dir.path().join("main.typ"), &source).unwrap();
        let project = Project::open(dir.path()).unwrap();
        let compiler = PreviewCompiler::default();
        let options = CompileOptions {
            ignore_system_fonts: true,
            ..Default::default()
        };
        let preview = compiler
            .compile(&project, "main.typ", &options)
            .unwrap()
            .artifact
            .unwrap();
        let map = preview.source_map.clone();
        workspace.show_text_file("main.typ", &source);
        let page = dir.path().join("page.svg");
        std::fs::write(&page, &preview.pages[0].svg).unwrap();
        workspace.set_source_map(Some(preview.source_map));
        workspace.set_compiled_preview(std::slice::from_ref(&page), "main.typ");
        drive(Duration::from_millis(100));
        let picture = workspace.preview_pictures.borrow()[0].widget.clone();
        let wait_for_page = |picture: &gtk::Picture| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !super::super::page_paintable::is_current(picture) && Instant::now() < deadline {
                drive(Duration::from_millis(10));
            }
            assert!(super::super::page_paintable::is_current(picture));
        };
        wait_for_page(&picture);
        assert!(!picture.has_tooltip());
        assert!(picture.tooltip_text().is_none());
        // Click the integral below its baseline, where Typst's glyph hit test
        // misses. This must exercise the enclosing-equation fallback.
        let (x, y) = (102.0, 42.0);
        let location = map.jump(0, x, y).expect("clickable integral descender");
        assert_eq!(location.line, 27);
        assert_eq!(location.column, 1);
        let height = f64::from(picture.height()).min(f64::from(picture.width()) / (240.0 / 180.0));
        let widget_x =
            (f64::from(picture.width()) - height * 240.0 / 180.0) / 2.0 + x * height / 180.0;
        let widget_y = (f64::from(picture.height()) - height) / 2.0 + y * height / 180.0;
        let controllers = picture.observe_controllers();
        let click = (0..controllers.n_items())
            .find_map(|index| {
                controllers
                    .item(index)?
                    .downcast::<gtk::GestureClick>()
                    .ok()
            })
            .unwrap();
        click.emit_by_name::<()>("released", &[&1_i32, &widget_x, &widget_y]);
        assert_eq!(
            workspace
                .buffer
                .iter_at_mark(&workspace.buffer.get_insert())
                .line(),
            26
        );
        assert_eq!(
            workspace
                .buffer
                .iter_at_mark(&workspace.buffer.get_insert())
                .line_offset(),
            location.column as i32 - 1
        );

        workspace
            .buffer
            .insert(&mut workspace.buffer.start_iter(), "// inserted\n");
        assert!(workspace.source_map.borrow().is_none());
        std::fs::write(dir.path().join("main.typ"), workspace.source_text()).unwrap();
        let preview = compiler
            .compile(&project, "main.typ", &options)
            .unwrap()
            .artifact
            .unwrap();
        workspace.set_source_map(Some(preview.source_map));
        workspace.set_compiled_preview(std::slice::from_ref(&page), "main.typ");
        assert_eq!(workspace.preview_pictures.borrow()[0].widget, picture);
        click.emit_by_name::<()>("released", &[&1_i32, &widget_x, &widget_y]);
        assert_eq!(
            workspace
                .buffer
                .iter_at_mark(&workspace.buffer.get_insert())
                .line(),
            27
        );

        // A changed layout retains the previous image while its new pixels are
        // decoded. A click must not use the new source map during that interval.
        let previous_size = (picture.width(), picture.height());
        let shifted_source = workspace
            .source_text()
            .replace("$ integral", "#v(24pt)\n$ integral");
        workspace.show_text_file("main.typ", &shifted_source);
        std::fs::write(dir.path().join("main.typ"), &shifted_source).unwrap();
        let preview = compiler
            .compile(&project, "main.typ", &options)
            .unwrap()
            .artifact
            .unwrap();
        let map = preview.source_map.clone();
        let formula_line = shifted_source
            .lines()
            .position(|line| line.contains("$ integral"))
            .unwrap()
            + 1;
        let (new_y, location) = (1..180)
            .find_map(|y| {
                map.jump(0, x, f64::from(y))
                    .filter(|location| location.line == formula_line)
                    .filter(|location| {
                        [(-0.1, 0.0), (0.1, 0.0), (0.0, -0.1), (0.0, 0.1)]
                            .into_iter()
                            .all(|(dx, dy)| {
                                map.jump(0, x + dx, f64::from(y) + dy).as_ref() == Some(location)
                            })
                    })
                    .map(|location| (f64::from(y), location))
            })
            .expect("formula moved down in the new layout");
        assert!(new_y > y);
        std::fs::write(&page, &preview.pages[0].svg).unwrap();
        workspace.set_source_map(Some(preview.source_map));
        workspace.set_compiled_preview(std::slice::from_ref(&page), "main.typ");
        let replacement = workspace.preview_pictures.borrow()[0].widget.clone();
        assert_ne!(replacement, picture);
        assert!(!super::super::page_paintable::is_current(&replacement));
        replacement.allocate(previous_size.0, previous_size.1, -1, None);
        assert!(replacement.width() > 0 && replacement.height() > 0);
        let new_point = |picture: &gtk::Picture| {
            let height =
                f64::from(picture.height()).min(f64::from(picture.width()) / (240.0 / 180.0));
            (
                (f64::from(picture.width()) - height * 240.0 / 180.0) / 2.0 + x * height / 180.0,
                (f64::from(picture.height()) - height) / 2.0 + new_y * height / 180.0,
            )
        };
        let (widget_x, widget_y) = new_point(&replacement);
        let controllers = replacement.observe_controllers();
        let click = (0..controllers.n_items())
            .find_map(|index| {
                controllers
                    .item(index)?
                    .downcast::<gtk::GestureClick>()
                    .ok()
            })
            .unwrap();
        let cursor = workspace.buffer.cursor_position();
        click.emit_by_name::<()>("released", &[&1_i32, &widget_x, &widget_y]);
        assert_eq!(workspace.buffer.cursor_position(), cursor);
        wait_for_page(&replacement);
        let (widget_x, widget_y) = new_point(&replacement);
        click.emit_by_name::<()>("released", &[&1_i32, &widget_x, &widget_y]);
        let cursor = workspace
            .buffer
            .iter_at_mark(&workspace.buffer.get_insert());
        assert_eq!(cursor.line() as usize + 1, location.line);
        assert_eq!(cursor.line_offset() as usize + 1, location.column);

        let errors = [DiagnosticRow {
            severity: DiagnosticKind::Error,
            path: "main.typ".into(),
            line: Some(1),
            column: Some(1),
            message: "unfinished expression".into(),
        }];
        workspace
            .buffer
            .insert(&mut workspace.buffer.end_iter(), "\n#");
        workspace.set_diagnostics(&errors);
        workspace.set_compile_error("Compile error: unfinished expression");
        drive(Duration::from_millis(450));
        assert!(!workspace.diagnostics_revealer.reveals_child());
        assert_ne!(
            workspace.compile_label.text(),
            "Compile error: unfinished expression"
        );
        workspace
            .buffer
            .insert(&mut workspace.buffer.end_iter(), "text");
        workspace.set_diagnostics(&[]);
        workspace.set_compile_status("Compiled");
        drive(Duration::from_millis(950));
        assert!(!workspace.diagnostics_revealer.reveals_child());
        assert_eq!(workspace.compile_label.text(), "Compiled");
        workspace
            .buffer
            .insert(&mut workspace.buffer.end_iter(), "(");
        workspace.set_diagnostics(&errors);
        drive(Duration::from_millis(950));
        assert!(workspace.diagnostics_revealer.reveals_child());
        workspace.cancel_pending_compile();
        window.destroy();
    }
}
