//! Searchable font-family chooser for the UI and editor font settings.
//!
//! Two sources share one search entry: families installed on this machine
//! (rendered in their own face) and the Google Fonts catalog (searched on a
//! worker thread; choosing a family downloads and registers it for this
//! process). Create one picker per setting and reuse it; closing hides it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::pango;
use typsmthng_gtk::backend::{
    list_local_families, register_app_font_files, search_catalog, BackendError, CatalogOrigin,
    FontCatalog, FontCategory, GoogleFontCache, Result,
};

type SelectedHandler = Rc<dyn Fn(&FontChoice)>;

const MAX_GOOGLE_RESULTS: usize = 300;
const LOCAL_PAGE: &str = "installed";
const GOOGLE_PAGE: &str = "google";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontPickerKind {
    Ui,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontSource {
    Local,
    Google,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontChoice {
    pub family: String,
    pub source: FontSource,
}

#[derive(Clone)]
struct GoogleRow {
    category: FontCategory,
    styles: usize,
    variable: bool,
    cached: bool,
}

enum CatalogState {
    Idle,
    Loading,
    Ready(Arc<FontCatalog>),
    Failed(String),
}

pub struct FontPicker {
    inner: Rc<Inner>,
}

struct Inner {
    window: gtk::Window,
    stack: adw::ViewStack,
    search: gtk::SearchEntry,
    monospace: gtk::ToggleButton,
    local_filter: gtk::CustomFilter,
    local_model: gtk::FilterListModel,
    local_list: gtk::ListView,
    local_mono: RefCell<HashSet<String>>,
    google_model: gtk::StringList,
    google_list: gtk::ListView,
    google_rows: RefCell<HashMap<String, GoogleRow>>,
    google_empty: gtk::Label,
    google_content: gtk::Stack,
    catalog: RefCell<CatalogState>,
    search_generation: Cell<u64>,
    busy: Cell<bool>,
    spinner: gtk::Spinner,
    status: gtk::Label,
    current: RefCell<Option<String>>,
    handlers: RefCell<Vec<SelectedHandler>>,
    cache: GoogleFontCache,
}

impl FontPicker {
    pub fn new(kind: FontPickerKind) -> Self {
        let title = match kind {
            FontPickerKind::Ui => "Interface Font",
            FontPickerKind::Editor => "Editor Font",
        };
        // adw::Window draws the ToolbarView header as its own titlebar; a
        // plain gtk::Window without one gets server-side decorations too.
        let window = adw::Window::builder()
            .title(format!("{title} — typsmthng"))
            .modal(true)
            .default_width(480)
            .default_height(600)
            .hide_on_close(true)
            .build();
        let window: gtk::Window = window.upcast();
        super::dismiss_on_escape(&window);
        window.add_css_class("font-picker");

        let stack = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&switcher));

        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search font families")
            .hexpand(true)
            .build();
        let monospace = gtk::ToggleButton::builder()
            .label("Monospace")
            .tooltip_text("Only show fixed-width families")
            .active(kind == FontPickerKind::Editor)
            .build();
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        controls.add_css_class("font-picker-controls");
        controls.append(&search);
        controls.append(&monospace);

        let local_filter = gtk::CustomFilter::new(|_| true);
        let local_model =
            gtk::FilterListModel::new(Some(gtk::StringList::new(&[])), Some(local_filter.clone()));
        let local_list = gtk::ListView::new(
            Some(gtk::NoSelection::new(Some(local_model.clone()))),
            None::<gtk::ListItemFactory>,
        );
        let local_empty = empty_label("No installed families match");
        let local_content = gtk::Stack::new();
        local_content.add_named(&scrolled(&local_list), Some("list"));
        local_content.add_named(&local_empty, Some("empty"));

        let google_model = gtk::StringList::new(&[]);
        let google_list = gtk::ListView::new(
            Some(gtk::NoSelection::new(Some(google_model.clone()))),
            None::<gtk::ListItemFactory>,
        );
        let google_empty = empty_label("Google Fonts has not loaded yet");
        let google_content = gtk::Stack::new();
        google_content.add_named(&scrolled(&google_list), Some("list"));
        google_content.add_named(&google_empty, Some("empty"));
        google_content.set_visible_child_name("empty");

        stack
            .add_titled_with_icon(
                &local_content,
                Some(LOCAL_PAGE),
                "Installed",
                "computer-symbolic",
            )
            .set_use_underline(false);
        stack
            .add_titled_with_icon(
                &google_content,
                Some(GOOGLE_PAGE),
                "Google Fonts",
                "folder-download-symbolic",
            )
            .set_use_underline(false);
        stack.set_vexpand(true);

        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        let status = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .build();
        status.add_css_class("caption");
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("font-picker-footer");
        footer.append(&spinner);
        footer.append(&status);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&controls);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&stack);
        content.append(&footer);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));
        window
            .downcast_ref::<adw::Window>()
            .expect("font picker is an adw::Window")
            .set_content(Some(&toolbar));

        let inner = Rc::new(Inner {
            window,
            stack,
            search,
            monospace,
            local_filter,
            local_model,
            local_list,
            local_mono: RefCell::new(HashSet::new()),
            google_model,
            google_list,
            google_rows: RefCell::new(HashMap::new()),
            google_empty,
            google_content,
            catalog: RefCell::new(CatalogState::Idle),
            search_generation: Cell::new(0),
            busy: Cell::new(false),
            spinner,
            status,
            current: RefCell::new(None),
            handlers: RefCell::new(Vec::new()),
            cache: GoogleFontCache::default(),
        });
        inner.install_factories();
        inner.connect_signals(&local_content);
        inner.reload_local();
        Self { inner }
    }

    /// Present the picker, optionally modal to `parent`. Installed families
    /// are re-read so fonts registered since the last opening appear.
    pub fn present(&self, parent: Option<&impl IsA<gtk::Window>>) {
        let inner = &self.inner;
        inner.window.set_transient_for(parent);
        inner.reload_local();
        inner.window.present();
        inner.search.grab_focus();
    }

    /// Mark the family currently in use so its row shows a check mark.
    pub fn set_current(&self, family: Option<&str>) {
        self.inner.current.replace(family.map(str::to_owned));
        self.inner.reload_local();
    }

    /// Start on the Google Fonts page with `query` pre-filled.
    pub fn show_google(&self, query: &str) {
        self.inner.search.set_text(query);
        self.inner.stack.set_visible_child_name(GOOGLE_PAGE);
    }

    /// Called after the user picks a family. Google families are already
    /// downloaded and registered with Pango when this fires.
    pub fn connect_selected(&self, handler: impl Fn(&FontChoice) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(handler));
    }
}

impl Inner {
    fn install_factories(self: &Rc<Self>) {
        let local = gtk::SignalListItemFactory::new();
        local.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            item.set_child(Some(&FamilyRow::new().root));
        });
        let this = Rc::downgrade(self);
        local.connect_bind(move |_, item| {
            let Some(this) = this.upgrade() else { return };
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let family = item_string(item);
            let row = FamilyRow::from_child(item);
            let mono = this.local_mono.borrow().contains(&family);
            row.bind(
                &family,
                true,
                if mono { "Monospace" } else { "Proportional" },
                this.is_current(&family),
            );
        });
        self.local_list.set_factory(Some(&local));

        let google = gtk::SignalListItemFactory::new();
        google.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            item.set_child(Some(&FamilyRow::new().root));
        });
        let this = Rc::downgrade(self);
        google.connect_bind(move |_, item| {
            let Some(this) = this.upgrade() else { return };
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let family = item_string(item);
            let row = FamilyRow::from_child(item);
            let meta = this.google_rows.borrow().get(&family).cloned();
            let mut details = Vec::new();
            let mut own_face = false;
            if let Some(meta) = meta {
                details.push(meta.category.label().to_string());
                details.push(if meta.variable {
                    "Variable".to_string()
                } else {
                    format!(
                        "{} style{}",
                        meta.styles,
                        if meta.styles == 1 { "" } else { "s" }
                    )
                });
                if meta.cached {
                    details.push("Downloaded".into());
                }
                own_face = meta.cached && typsmthng_gtk::backend::family_is_available(&family);
            }
            row.bind(
                &family,
                own_face,
                &details.join(" · "),
                this.is_current(&family),
            );
        });
        self.google_list.set_factory(Some(&google));
    }

    fn connect_signals(self: &Rc<Self>, local_content: &gtk::Stack) {
        let this = Rc::downgrade(self);
        self.local_filter.set_filter_func(move |object| {
            let Some(this) = this.upgrade() else {
                return true;
            };
            let Some(family) = object.downcast_ref::<gtk::StringObject>() else {
                return false;
            };
            let family = family.string();
            let query = this.search.text().trim().to_lowercase();
            (!this.monospace.is_active() || this.local_mono.borrow().contains(family.as_str()))
                && (query.is_empty() || family.to_lowercase().contains(&query))
        });
        let content = local_content.clone();
        self.local_model
            .connect_items_changed(move |model, _, _, _| {
                content.set_visible_child_name(if model.n_items() == 0 {
                    "empty"
                } else {
                    "list"
                });
            });

        let this = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(this) = this.upgrade() {
                this.refilter();
            }
        });
        let this = Rc::downgrade(self);
        self.monospace.connect_toggled(move |_| {
            if let Some(this) = this.upgrade() {
                this.refilter();
            }
        });
        let this = Rc::downgrade(self);
        self.search.connect_activate(move |_| {
            if let Some(this) = this.upgrade() {
                this.activate_first();
            }
        });
        let this = Rc::downgrade(self);
        self.stack.connect_visible_child_name_notify(move |_| {
            let Some(this) = this.upgrade() else { return };
            // Revisiting the page retries a catalog download that failed.
            let failed = matches!(*this.catalog.borrow(), CatalogState::Failed(_));
            if failed && this.on_google_page() {
                this.catalog.replace(CatalogState::Idle);
            }
            this.refilter();
        });

        self.local_list.set_single_click_activate(true);
        let this = Rc::downgrade(self);
        self.local_list.connect_activate(move |_, position| {
            let Some(this) = this.upgrade() else { return };
            if let Some(family) = model_string(&this.local_model, position) {
                this.choose(FontChoice {
                    family,
                    source: FontSource::Local,
                });
            }
        });
        self.google_list.set_single_click_activate(true);
        let this = Rc::downgrade(self);
        self.google_list.connect_activate(move |_, position| {
            let Some(this) = this.upgrade() else { return };
            if let Some(family) = model_string(&this.google_model, position) {
                this.download_and_choose(family);
            }
        });
    }

    fn is_current(&self, family: &str) -> bool {
        self.current
            .borrow()
            .as_deref()
            .is_some_and(|current| current.eq_ignore_ascii_case(family))
    }

    fn on_google_page(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some(GOOGLE_PAGE)
    }

    fn reload_local(&self) {
        let all = list_local_families(false);
        self.local_mono
            .replace(list_local_families(true).into_iter().collect());
        let refs = all.iter().map(String::as_str).collect::<Vec<_>>();
        self.local_model
            .set_model(Some(&gtk::StringList::new(&refs)));
        if !self.busy.get() && !self.on_google_page() {
            self.show_local_count();
        }
    }

    fn refilter(self: &Rc<Self>) {
        self.local_filter.changed(gtk::FilterChange::Different);
        if self.on_google_page() {
            self.search_google();
        } else if !self.busy.get() {
            self.show_local_count();
        }
    }

    fn show_local_count(&self) {
        let shown = self.local_model.n_items();
        let message = if self.monospace.is_active() {
            format!("{shown} monospace {}", families(shown as usize))
        } else {
            format!("{shown} installed {}", families(shown as usize))
        };
        self.set_status(&message, false);
    }

    fn activate_first(self: &Rc<Self>) {
        if self.on_google_page() {
            if let Some(family) = model_string(&self.google_model, 0) {
                self.download_and_choose(family);
            }
        } else if let Some(family) = model_string(&self.local_model, 0) {
            self.choose(FontChoice {
                family,
                source: FontSource::Local,
            });
        }
    }

    fn search_google(self: &Rc<Self>) {
        let state = match &*self.catalog.borrow() {
            CatalogState::Ready(catalog) => Ok(catalog.clone()),
            CatalogState::Loading => return,
            CatalogState::Failed(error) => Err(Some(error.clone())),
            CatalogState::Idle => Err(None),
        };
        let catalog = match state {
            Ok(catalog) => catalog,
            Err(Some(error)) => return self.show_google_empty(&error),
            Err(None) => return self.load_catalog(),
        };
        let generation = self.search_generation.get() + 1;
        self.search_generation.set(generation);
        let query = self.search.text().to_string();
        let category = self
            .monospace
            .is_active()
            .then_some(FontCategory::Monospace);
        let cache = self.cache.clone();
        let search = gio::spawn_blocking(move || {
            let matches = search_catalog(&catalog, &query, category);
            let total = matches.len();
            let rows = matches
                .into_iter()
                .take(MAX_GOOGLE_RESULTS)
                .map(|family| {
                    let row = GoogleRow {
                        category: family.category,
                        styles: family.variants.len(),
                        variable: family.axes.iter().any(|axis| axis == "wght"),
                        cached: cache.cached_family_files(&family.family).is_some(),
                    };
                    (family.family.clone(), row)
                })
                .collect::<Vec<_>>();
            (rows, total)
        });
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok((rows, total)) = search.await else {
                return;
            };
            let Some(this) = this.upgrade() else { return };
            if this.search_generation.get() != generation {
                return;
            }
            this.show_google_results(rows, total);
        });
    }

    fn show_google_results(&self, rows: Vec<(String, GoogleRow)>, total: usize) {
        let names = rows
            .iter()
            .map(|(family, _)| family.as_str())
            .collect::<Vec<_>>();
        self.google_rows.replace(rows.iter().cloned().collect());
        self.google_model
            .splice(0, self.google_model.n_items(), &names);
        if rows.is_empty() {
            self.show_google_empty("No Google Fonts families match");
        } else {
            self.google_content.set_visible_child_name("list");
            if let Some(adjustment) = self
                .google_list
                .parent()
                .and_downcast::<gtk::ScrolledWindow>()
                .map(|scrolled| scrolled.vadjustment())
            {
                adjustment.set_value(0.0);
            }
        }
        if !self.busy.get() {
            let shown = if total > rows.len() {
                format!("Showing {} of {total} families", rows.len())
            } else {
                format!("{total} {}", families(total))
            };
            self.set_status(&shown, false);
        }
    }

    fn show_google_empty(&self, message: &str) {
        self.google_empty.set_label(message);
        self.google_content.set_visible_child_name("empty");
    }

    fn load_catalog(self: &Rc<Self>) {
        self.catalog.replace(CatalogState::Loading);
        self.show_google_empty("Loading Google Fonts…");
        self.set_busy(true, "Loading the Google Fonts catalog…");
        let cache = self.cache.clone();
        let load = gio::spawn_blocking(move || cache.load_catalog());
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = load
                .await
                .unwrap_or_else(|_| Err(BackendError::Network("catalog worker panicked".into())));
            let Some(this) = this.upgrade() else { return };
            this.set_busy(false, "");
            match result {
                Ok((catalog, origin)) => {
                    this.catalog.replace(CatalogState::Ready(Arc::new(catalog)));
                    this.search_google();
                    if origin == CatalogOrigin::StaleCache {
                        this.set_status("Offline: showing the cached Google Fonts catalog", true);
                    }
                }
                Err(error) => {
                    let message = format!("Could not load Google Fonts. {error}");
                    this.catalog.replace(CatalogState::Failed(message.clone()));
                    this.show_google_empty(&message);
                    this.set_status(
                        "Check your connection, then reopen this page to retry",
                        true,
                    );
                }
            }
        });
    }

    fn download_and_choose(self: &Rc<Self>, family: String) {
        if self.busy.get() {
            return;
        }
        self.set_busy(true, &format!("Downloading {family}…"));
        let this = Rc::downgrade(self);
        ensure_registered_async(vec![family], move |mut results| {
            let Some(this) = this.upgrade() else { return };
            this.set_busy(false, "");
            let Some((family, result)) = results.pop() else {
                return;
            };
            match result {
                Ok(_) => {
                    if let Some(row) = this.google_rows.borrow_mut().get_mut(&family) {
                        row.cached = true;
                    }
                    this.choose(FontChoice {
                        family,
                        source: FontSource::Google,
                    });
                }
                Err(error) => {
                    this.set_status(&format!("Could not add {family}: {error}"), true);
                }
            }
        });
    }

    fn choose(&self, choice: FontChoice) {
        self.current.replace(Some(choice.family.clone()));
        let handlers = self.handlers.borrow().clone();
        for handler in handlers {
            handler(&choice);
        }
        self.window.close();
    }

    fn set_busy(&self, busy: bool, message: &str) {
        self.busy.set(busy);
        self.spinner.set_visible(busy);
        self.spinner.set_spinning(busy);
        self.set_status(message, false);
    }

    fn set_status(&self, message: &str, error: bool) {
        self.status.set_label(message);
        if error {
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
        }
    }
}

/// Download (when needed) and register Google Fonts families off the main
/// thread, then report per-family results on the main thread. Use at startup
/// to restore persisted Google UI/editor fonts.
pub fn ensure_registered_async(
    families: Vec<String>,
    done: impl FnOnce(Vec<(String, Result<Vec<PathBuf>>)>) + 'static,
) {
    let cache = GoogleFontCache::default();
    let requested = families.clone();
    let download = gio::spawn_blocking(move || {
        families
            .into_iter()
            .map(|family| {
                let files = cache.ensure_family_files(&family);
                (family, files)
            })
            .collect::<Vec<_>>()
    });
    glib::spawn_future_local(async move {
        let results = download.await.unwrap_or_else(|_| {
            requested
                .into_iter()
                .map(|family| {
                    let error = BackendError::Network("font download worker panicked".into());
                    (family, Err(error))
                })
                .collect()
        });
        let results = results
            .into_iter()
            .map(|(family, files)| {
                let registered =
                    files.and_then(|files| register_app_font_files(&files).map(|()| files));
                (family, registered)
            })
            .collect();
        done(results);
    });
}

struct FamilyRow {
    root: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    check: gtk::Image,
}

impl FamilyRow {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("font-picker-row");
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        title.add_css_class("font-picker-title");
        let subtitle = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        subtitle.add_css_class("caption");
        subtitle.add_css_class("muted");
        text.append(&title);
        text.append(&subtitle);
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.add_css_class("accent");
        root.append(&text);
        root.append(&check);
        Self {
            root,
            title,
            subtitle,
            check,
        }
    }

    fn from_child(item: &gtk::ListItem) -> Self {
        let root = item.child().and_downcast::<gtk::Box>().unwrap();
        let text = root.first_child().and_downcast::<gtk::Box>().unwrap();
        let title = text.first_child().and_downcast::<gtk::Label>().unwrap();
        let subtitle = title.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let check = text.next_sibling().and_downcast::<gtk::Image>().unwrap();
        Self {
            root,
            title,
            subtitle,
            check,
        }
    }

    fn bind(&self, family: &str, own_face: bool, details: &str, current: bool) {
        self.title.set_label(family);
        let attributes = pango::AttrList::new();
        if own_face {
            let mut description = pango::FontDescription::new();
            description.set_family(family);
            attributes.insert(pango::AttrFontDesc::new(&description));
        }
        self.title.set_attributes(Some(&attributes));
        self.subtitle.set_label(details);
        self.subtitle.set_visible(!details.is_empty());
        self.check.set_opacity(if current { 1.0 } else { 0.0 });
        self.root
            .update_property(&[gtk::accessible::Property::Label(family)]);
    }
}

fn families(count: usize) -> &'static str {
    if count == 1 {
        "family"
    } else {
        "families"
    }
}

fn item_string(item: &gtk::ListItem) -> String {
    item.item()
        .and_downcast::<gtk::StringObject>()
        .map(|object| object.string().to_string())
        .unwrap_or_default()
}

fn model_string(model: &impl IsA<gio::ListModel>, position: u32) -> Option<String> {
    model
        .item(position)
        .and_downcast::<gtk::StringObject>()
        .map(|object| object.string().to_string())
}

fn scrolled(list: &gtk::ListView) -> gtk::ScrolledWindow {
    list.add_css_class("navigation-sidebar");
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(list)
        .build()
}

fn empty_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .valign(gtk::Align::Center)
        .margin_start(24)
        .margin_end(24)
        .build();
    label.add_css_class("muted");
    label
}
