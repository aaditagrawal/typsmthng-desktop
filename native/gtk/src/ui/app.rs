use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use super::updates::{UpdateDialog, UpdatePhase};
use adw::prelude::*;
use typsmthng_gtk::backend::app_fonts;
use typsmthng_gtk::backend::{
    export_project, export_projects, import_project, import_projects, ArchiveLimits, BackendError,
    CompileOptions, CompileOutput, DiagnosticSeverity, EntryKind, ExportArtifact, ExportFormat,
    ExternalEventKind, ExternalWatcher, FileContent, GoogleFontCache, InlineNote, PdfStandard,
    Project, StateStore, Theme as BackendTheme, TypstTool, UniverseClient, UniverseTemplate,
    UpdateClient, UpdateStatus, UserSettings, WindowState as BackendWindowState,
};

use typsmthng_gtk::backend::imports::{
    convert_latex_project_if_needed, import_latex_file, import_latex_sources, import_path_tree,
    unique_project_path,
};
use typsmthng_gtk::backend::notes;
use typsmthng_gtk::backend::preview::{PreviewCompiler, SourceMap};
use typsmthng_gtk::backend::rendered_preview::{prepare_preview, PreparedPreview};

use super::home::{HomeCallbacks, HomeView, RecentProjectRow};
use super::model::{
    effective_page_size, resolve_startup_path, ui_font_name, SearchMode, Theme, UiSettings,
    ViewMode,
};
use super::presentation::PresentationController;
use super::workspace::{
    DiagnosticKind, DiagnosticRow, FileRow, SearchResultRow, WorkspaceCallbacks, WorkspaceView,
};

const APP_ID: &str = "dev.typsmthng.Typsmthng";

thread_local! {
    static CONTROLLERS: RefCell<Vec<Rc<AppController>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub startup_path: Option<PathBuf>,
    pub smoke_test: bool,
    pub presentation_smoke_test: bool,
    pub interaction_smoke_test: bool,
}

pub fn launch(options: LaunchOptions) -> glib::ExitCode {
    let application: gtk::Application = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build()
        .upcast();

    application.connect_activate({
        let options = options.clone();
        move |application| {
            if application.active_window().is_some() {
                return;
            }
            super::install_css();
            AppController::build(application, options.clone());
        }
    });
    application.connect_open({
        let options = options.clone();
        move |application, files, _hint| {
            let path = files.first().and_then(gio::File::path);
            let existing = CONTROLLERS.with(|controllers| controllers.borrow().last().cloned());
            if let Some(controller) = existing {
                if let Some(path) = path {
                    controller.open_startup_path(&path);
                }
                controller.window.present();
            } else {
                super::install_css();
                let mut launch = options.clone();
                launch.startup_path = path;
                AppController::build(application, launch);
            }
        }
    });
    // Application-specific switches and positional startup paths are parsed in
    // main; do not ask GApplication to reject them as unknown GTK options.
    let mut arguments = vec!["typsmthng".to_string()];
    if let Some(path) = &options.startup_path {
        arguments.push(path.to_string_lossy().into_owned());
    }
    application.run_with_args(&arguments)
}

struct AppController {
    self_weak: RefCell<Weak<AppController>>,
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    update_button: gtk::Button,
    update_phase: Cell<UpdatePhase>,
    update_progress: Cell<(u64, Option<u64>)>,
    update_error: RefCell<Option<String>>,
    update_dialog: RefCell<Option<UpdateDialog>>,
    changelog_dialog: RefCell<Option<adw::Dialog>>,
    available_update: RefCell<Option<UpdateStatus>>,
    downloaded_update: RefCell<Option<(tempfile::TempDir, PathBuf, String)>>,
    state_store: Option<StateStore>,
    _smoke_state: Option<tempfile::TempDir>,
    project: RefCell<Option<Project>>,
    current_file: RefCell<Option<String>>,
    disk_baseline: RefCell<Option<(String, String)>>,
    compiled_main: RefCell<Option<String>>,
    home: RefCell<Option<HomeView>>,
    workspace: RefCell<Option<WorkspaceView>>,
    presentation: RefCell<Option<PresentationController>>,
    settings: RefCell<UiSettings>,
    hidden_files: RefCell<bool>,
    render_cache: RefCell<Option<tempfile::TempDir>>,
    watcher: RefCell<Option<ExternalWatcher>>,
    preview_compiler: PreviewCompiler,
    compile_generation: Cell<u64>,
    compile_in_flight: Cell<bool>,
    compile_cancellation: RefCell<Option<Arc<AtomicBool>>>,
    font_in_flight: Cell<bool>,
    pending_fonts: RefCell<Option<(Project, String)>>,
    notes_in_flight: Cell<bool>,
    notes_generation: Cell<Option<u64>>,
    pending_compile_source: RefCell<Option<String>>,
    search_in_flight: Cell<bool>,
    pending_search: RefCell<Option<PendingSearch>>,
    /// The desktop's `gtk-font-name`, captured before any override.
    system_font: String,
}

type SearchReply = Rc<dyn Fn(Vec<SearchResultRow>)>;
struct PendingSearch {
    mode: SearchMode,
    query: String,
    reply: SearchReply,
}

struct CompileFinished {
    generation: u64,
    revision: u64,
    source_map: Option<Arc<SourceMap>>,
    main: String,
    result: Result<(CompileOutput<PreparedPreview>, Vec<InlineNote>), String>,
}

impl AppController {
    fn build(application: &gtk::Application, options: LaunchOptions) -> Rc<Self> {
        let started = std::time::Instant::now();
        let mut startup_errors = Vec::new();
        let smoke =
            options.smoke_test || options.presentation_smoke_test || options.interaction_smoke_test;
        let smoke_state = smoke.then(|| tempfile::tempdir().expect("smoke state directory"));
        let discovered_store = smoke_state
            .as_ref()
            .map_or_else(StateStore::discover, |dir| Ok(StateStore::new(dir.path())));
        let state_store = match discovered_store {
            Ok(store) => Some(store),
            Err(error) => {
                startup_errors.push(format!("State storage: {error}"));
                None
            }
        };
        let backend_settings = match state_store.as_ref().map(StateStore::load_settings) {
            Some(Ok(settings)) => settings,
            Some(Err(error)) => {
                startup_errors.push(format!("Settings: {error}"));
                UserSettings::default()
            }
            None => UserSettings::default(),
        };
        let window_state = match state_store.as_ref().map(StateStore::load_metadata) {
            Some(Ok(metadata)) => metadata.window_state,
            Some(Err(error)) => {
                startup_errors.push(format!("Project history: {error}"));
                BackendWindowState::default()
            }
            None => BackendWindowState::default(),
        };
        let mut settings = settings_from_backend(&backend_settings);
        if smoke {
            match std::env::var("TYPSMTHNG_SMOKE_THEME").as_deref() {
                Ok("light") => settings.theme = Theme::Light,
                Ok("dark") => settings.theme = Theme::Dark,
                _ => {}
            }
            settings.google_fonts = false;
            settings.vim_mode = std::env::var_os("TYPSMTHNG_SMOKE_VIM").is_some();
        }
        let native_window = adw::ApplicationWindow::builder()
            .application(application)
            .title("typsmthng")
            .default_width(if smoke {
                std::env::var("TYPSMTHNG_SMOKE_WIDTH")
                    .ok()
                    .and_then(|width| width.parse().ok())
                    .unwrap_or(window_state.width)
            } else {
                window_state.width
            })
            .default_height(window_state.height)
            .build();
        let window: gtk::ApplicationWindow = native_window.clone().upcast();
        window.set_size_request(760, 520);
        window.add_css_class("typsmthng");
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(120);
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new("typsmthng", "");
        window
            .bind_property("title", &title, "title")
            .sync_create()
            .build();
        header.set_title_widget(Some(&title));
        let update_button = gtk::Button::with_label("Check for updates");
        update_button.set_tooltip_text(Some("Check GitHub for a newer stable release"));
        header.pack_end(&update_button);
        let menu = gio::Menu::new();
        menu.append(Some("Check for updates"), Some("app.check-updates"));
        menu.append(Some("What's new"), Some("app.whats-new"));
        let app_menu = gtk::MenuButton::new();
        app_menu.set_icon_name("open-menu-symbolic");
        app_menu.set_tooltip_text(Some("Application menu"));
        app_menu.set_menu_model(Some(&menu));
        header.pack_end(&app_menu);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&stack));
        native_window.set_content(Some(&toolbar));

        let controller = Rc::new(Self {
            self_weak: RefCell::new(Weak::new()),
            application: application.clone(),
            window,
            stack,
            update_button,
            update_phase: Cell::new(UpdatePhase::Idle),
            update_progress: Cell::new((0, None)),
            update_error: RefCell::new(None),
            update_dialog: RefCell::new(None),
            changelog_dialog: RefCell::new(None),
            available_update: RefCell::new(None),
            downloaded_update: RefCell::new(None),
            state_store,
            _smoke_state: smoke_state,
            project: RefCell::new(None),
            current_file: RefCell::new(None),
            disk_baseline: RefCell::new(None),
            compiled_main: RefCell::new(None),
            home: RefCell::new(None),
            workspace: RefCell::new(None),
            presentation: RefCell::new(None),
            settings: RefCell::new(settings.clone()),
            hidden_files: RefCell::new(false),
            render_cache: RefCell::new(None),
            watcher: RefCell::new(None),
            preview_compiler: PreviewCompiler::default(),
            compile_generation: Cell::new(0),
            compile_in_flight: Cell::new(false),
            compile_cancellation: RefCell::new(None),
            font_in_flight: Cell::new(false),
            pending_fonts: RefCell::new(None),
            notes_in_flight: Cell::new(false),
            notes_generation: Cell::new(None),
            pending_compile_source: RefCell::new(None),
            search_in_flight: Cell::new(false),
            pending_search: RefCell::new(None),
            system_font: gtk::Settings::default()
                .and_then(|settings| settings.gtk_font_name())
                .map_or_else(|| "Sans 11".to_string(), |name| name.to_string()),
        });
        controller.self_weak.replace(Rc::downgrade(&controller));

        controller.update_button.connect_clicked({
            let weak = Rc::downgrade(&controller);
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.activate_update();
                }
            }
        });
        controller.install_views();
        controller.install_actions();
        if smoke {
            let count = std::env::var("TYPSMTHNG_SMOKE_PROJECTS")
                .ok()
                .and_then(|count| count.parse::<usize>().ok())
                .unwrap_or(0)
                .min(8);
            if let (Some(directory), Some(store)) =
                (&controller._smoke_state, &controller.state_store)
            {
                for index in 1..=count {
                    let project =
                        Project::create(directory.path(), &format!("Sample project {index}"))
                            .expect("create visual-review project");
                    store
                        .upsert_recent(
                            project.root(),
                            project.name(),
                            1,
                            Some("main.typ".into()),
                            false,
                        )
                        .expect("record visual-review project");
                }
            }
        }
        controller.refresh_recents();
        controller
            .workspace
            .borrow()
            .as_ref()
            .unwrap()
            .apply_settings(settings.clone());
        controller.apply_theme(settings.theme);
        controller.apply_ui_font(&settings);
        if !smoke {
            if startup_errors.is_empty() {
                let weak = Rc::downgrade(&controller);
                glib::idle_add_local_once(move || {
                    if let Some(this) = weak.upgrade() {
                        this.show_unseen_changelog();
                    }
                });
            }
            controller.restore_downloaded_fonts(&settings);
        }
        adw::StyleManager::default().connect_dark_notify({
            let weak = Rc::downgrade(&controller);
            move |_| {
                if let Some(controller) = weak.upgrade() {
                    let settings = controller.settings.borrow().clone();
                    if let Some(home) = controller.home.borrow().as_ref() {
                        home.apply_theme(settings.theme);
                    }
                    if let Some(workspace) = controller.workspace.borrow().as_ref() {
                        workspace.apply_settings(settings);
                    }
                }
            }
        });
        if settings.translucent {
            controller.window.add_css_class("translucent");
        }
        controller.window.present();
        if window_state.maximized {
            controller.window.maximize();
        }
        if !startup_errors.is_empty() {
            controller.show_error(
                "Some application data could not be loaded",
                &startup_errors.join("\n"),
            );
        }
        {
            let weak = Rc::downgrade(&controller);
            glib::timeout_add_local(Duration::from_millis(350), move || {
                if let Some(this) = weak.upgrade() {
                    this.poll_external_changes();
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }

        if let Some(path) = options.startup_path {
            controller.open_startup_path(&path);
        } else if let Some(path) = controller
            .state_store
            .as_ref()
            .and_then(|store| store.load_metadata().ok())
            .and_then(|metadata| metadata.reopen_last_project_path)
            .filter(|path| path.is_dir())
        {
            controller.open_project(&path, None);
        } else {
            controller.show_home();
        }

        if !smoke {
            if let Some(cache) = directories::ProjectDirs::from("dev", "typsmthng", "typsmthng") {
                if let Ok(entries) = std::fs::read_dir(cache.cache_dir().join("updates")) {
                    for entry in entries.flatten() {
                        let error_path = entry.path().join("install-error.txt");
                        if let Ok(error) = std::fs::read_to_string(&error_path) {
                            controller.show_error("Update could not be installed", &error);
                            let _ = std::fs::remove_file(error_path);
                        }
                        // Retain recent installer logs, then reclaim old artifacts.
                        if entry.file_name().to_string_lossy().starts_with("update-")
                            && entry.file_type().is_ok_and(|kind| kind.is_dir())
                            && entry
                                .metadata()
                                .and_then(|metadata| metadata.modified())
                                .ok()
                                .and_then(|modified| modified.elapsed().ok())
                                .is_some_and(|age| age > Duration::from_secs(7 * 24 * 60 * 60))
                        {
                            let _ = std::fs::remove_dir_all(entry.path());
                        }
                    }
                }
            }
            let weak = Rc::downgrade(&controller);
            glib::timeout_add_local_once(Duration::from_secs(2), move || {
                if let Some(this) = weak.upgrade() {
                    this.check_update_with_feedback(false);
                }
            });
            let weak = Rc::downgrade(&controller);
            glib::timeout_add_local(Duration::from_secs(6 * 60 * 60), move || {
                if let Some(this) = weak.upgrade() {
                    this.check_update_with_feedback(false);
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }
        if options.interaction_smoke_test {
            controller.run_interaction_smoke();
        } else if options.presentation_smoke_test {
            let weak = Rc::downgrade(&controller);
            let application = application.clone();
            let attempts = Rc::new(Cell::new(0_u16));
            glib::timeout_add_local(Duration::from_millis(100), move || {
                let Some(this) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let attempt = attempts.get() + 1;
                attempts.set(attempt);
                let Some(presentation) = this.presentation.borrow().as_ref().cloned() else {
                    return glib::ControlFlow::Continue;
                };
                if presentation.page_count() > 0 && presentation.window_count() == 0 {
                    this.present_dual();
                }
                if presentation.window_count() > 0
                    && this.notes_generation.get() == Some(this.compile_generation.get())
                {
                    if let Ok(expected) = std::env::var("TYPSMTHNG_SMOKE_EXPECT_NOTE") {
                        assert!(
                            presentation.has_inline_note(&expected),
                            "expected inline speaker note was not loaded"
                        );
                    }
                    println!(
                        "TYPESMTHNG_PRESENTATION_READY {{\"gtk\":true,\"pages\":{},\"windows\":{},\"notes_loaded\":true}}",
                        presentation.page_count(), presentation.window_count()
                    );
                    let _ = std::io::stdout().flush();
                    let application = application.clone();
                    glib::timeout_add_local_once(
                        Duration::from_millis(
                            std::env::var("TYPSMTHNG_SMOKE_HOLD_MS")
                                .ok()
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(900),
                        ),
                        move || {
                            super::smoke::capture_windows(&application);
                            application.quit()
                        },
                    );
                    glib::ControlFlow::Break
                } else if attempt >= 200 {
                    eprintln!("TYPESMTHNG_PRESENTATION_FAILED {{\"reason\":\"compile-timeout\"}}");
                    std::process::exit(1);
                } else {
                    glib::ControlFlow::Continue
                }
            });
        } else if options.smoke_test {
            match std::env::var("TYPSMTHNG_SMOKE_VIEW").as_deref() {
                Ok("settings") => controller.show_settings(),
                Ok("guide") => controller.show_guide(),
                Ok("updates") => controller.present_update_dialog(),
                Ok("changelog") => controller.show_changelog(),
                Ok("font-picker") => controller.show_font_picker_smoke(),
                Ok("export") => controller.choose_document_export(),
                Ok("templates") => controller.create_from_template(),
                Ok("import") => controller.show_import_options(),
                Ok("name") => controller.prompt_name("New project", "Project name", |_, _| {}),
                Ok("search") => {
                    if let Some(workspace) = controller.workspace.borrow().as_ref() {
                        workspace.present_search();
                    }
                }
                _ => {}
            }
            let application = application.clone();
            glib::idle_add_local_once(move || {
                println!(
                    "TYPESMTHNG_SMOKE_READY {{\"gtk\":true,\"window\":true,\"startup_ms\":{}}}",
                    started.elapsed().as_millis()
                );
                let _ = std::io::stdout().flush();
                glib::timeout_add_local_once(
                    Duration::from_millis(
                        std::env::var("TYPSMTHNG_SMOKE_HOLD_MS")
                            .ok()
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(900),
                    ),
                    move || {
                        super::smoke::capture_windows(&application);
                        application.quit()
                    },
                );
            });
        }
        CONTROLLERS.with(|controllers| controllers.borrow_mut().push(controller.clone()));
        controller
    }

    fn run_interaction_smoke(self: &Rc<Self>) {
        let project = Project::create(self._smoke_state.as_ref().unwrap().path(), "interaction")
            .expect("create isolated interaction fixture");
        project
            .write_text_atomic(
                "main.typ",
                include_str!("../../tests/fixtures/demo/main.typ"),
            )
            .expect("write interaction fixture");
        project
            .create_binary_file("asset.bin", &[0, 255, 1, 128])
            .expect("write binary fixture");
        self.open_project(project.root(), Some("main.typ".into()));
        let weak = self.weak();
        let mut step = 0_u32;
        let mut last_tick = std::time::Instant::now();
        let mut max_tick = Duration::ZERO;
        let began = last_tick;
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            max_tick = max_tick.max(last_tick.elapsed());
            last_tick = std::time::Instant::now();
            let workspace = this.workspace.borrow().as_ref().unwrap().clone();
            step += 1;
            if step <= 12 {
                let buffer = workspace.editor.buffer();
                buffer.insert(
                    &mut buffer.end_iter(),
                    &format!("\n// interaction {step}: café λ"),
                );
                workspace.request_compile();
                if step == 12 {
                    // A queued compile must read the live buffer, not replay the
                    // older snapshot and mark these newer edits as saved.
                    buffer.insert(&mut buffer.end_iter(), "\n// unscheduled final edit");
                }
            } else if step == 13 {
                // Exercise file switching while a previous compile is queued.
                this.select_file("asset.bin".into());
            } else if step == 18 {
                assert_eq!(
                    std::fs::read(project.root().join("asset.bin")).unwrap(),
                    [0, 255, 1, 128]
                );
                this.select_file("main.typ".into());
            } else if step > 18
                && !this.compile_in_flight.get()
                && this.pending_compile_source.borrow().is_none()
            {
                assert!(workspace.source_text().contains("interaction 12: café λ"));
                assert!(workspace.source_text().contains("unscheduled final edit"));
                assert!(std::fs::read_to_string(project.root().join("main.typ"))
                    .unwrap()
                    .contains("interaction 12: café λ"));
                assert_eq!(this.presentation.borrow().as_ref().unwrap().page_count(), 3);
                println!("TYPESMTHNG_INTERACTION_READY {{\"edits\":12,\"asset_preserved\":true,\"pages\":3,\"elapsed_ms\":{},\"max_tick_ms\":{}}}", began.elapsed().as_millis(), max_tick.as_millis());
                this.run_resize_smoke();
                return glib::ControlFlow::Break;
            }
            assert!(
                began.elapsed() < Duration::from_secs(20),
                "interaction smoke timed out"
            );
            glib::ControlFlow::Continue
        });
    }

    fn run_resize_smoke(&self) {
        let started = std::time::Instant::now();
        self.window.set_default_size(760, 800);
        let weak = self.weak();
        let mut phase = 0;
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let workspace = this.workspace.borrow().as_ref().unwrap().clone();
            let sidebar =
                find_css_widget(workspace.root.upcast_ref(), "sidebar").expect("sidebar widget");
            // Adwaita decorations are included in the requested window size,
            // while widget allocations describe the usable content area.
            if phase == 0 && (740..=780).contains(&workspace.root.width()) && !sidebar.is_visible()
            {
                assert!(
                    workspace.editor.width() >= 300,
                    "narrow editor is too small"
                );
                phase = 1;
                this.window.set_default_size(1280, 800);
            } else if phase == 1
                && (1260..=1300).contains(&workspace.root.width())
                && sidebar.is_visible()
            {
                println!("TYPESMTHNG_RESIZE_READY {{\"narrow\":760,\"wide\":1280,\"sidebar_restored\":true}}");
                super::smoke::capture_windows(&this.application);
                this.application.quit();
                return glib::ControlFlow::Break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(8),
                "native resize smoke timed out: width={} sidebar={} editor={}",
                workspace.root.width(),
                sidebar.is_visible(),
                workspace.editor.width()
            );
            glib::ControlFlow::Continue
        });
    }

    fn install_views(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let home = HomeView::new(HomeCallbacks {
            open_folder: callback0(&weak, Self::choose_open_project),
            create_project: callback0(&weak, Self::choose_create_project),
            open_recent: callback1(&weak, |this, path| {
                this.open_project(Path::new(&path), None)
            }),
            show_guide: callback0(&weak, Self::show_guide),
            show_settings: callback0(&weak, Self::show_settings),
            toggle_favorite: callback1(&weak, Self::toggle_favorite),
            remove_recent: callback1(&weak, Self::remove_recent),
            rename_recent: callback1(&weak, Self::rename_recent),
            import_project: callback0(&weak, Self::show_import_options),
            export_all: callback0(&weak, Self::export_all_projects),
            export_selected: callback1(&weak, Self::export_selected_projects),
            create_from_template: callback0(&weak, Self::create_from_template),
            create_workspace: callback0(&weak, Self::create_workspace),
            select_workspace: callback1(&weak, Self::select_workspace),
            manage_workspace: callback0(&weak, Self::manage_workspace),
            assign_workspace: callback1(&weak, Self::assign_project_workspace),
            assign_selected_workspace: callback1(&weak, Self::assign_projects_workspace),
        });
        self.stack.add_named(&home.root, Some("home"));
        self.home.replace(Some(home));

        let workspace = WorkspaceView::new(
            &self.window,
            WorkspaceCallbacks {
                editor_query: {
                    let weak = weak.clone();
                    Rc::new(move |request| {
                        let this = weak.upgrade()?;
                        if this.current_file.borrow().as_deref() != Some(request.path.as_str())
                            || !request.path.ends_with(".typ")
                        {
                            return None;
                        }
                        let project = this.project.borrow().clone()?;
                        let options = this.compile_options().ok()?;
                        this.preview_compiler
                            .editor(&project, &options, request)
                            .ok()
                    })
                },
                go_home: callback0(&weak, Self::close_to_home),
                open_project: callback0(&weak, Self::choose_open_project),
                save: callback1_result(&weak, Self::save_current),
                force_save: callback1_result(&weak, Self::force_save_current),
                select_file: callback1(&weak, Self::select_file),
                create_file: callback0(&weak, Self::prompt_create_file),
                create_folder: callback0(&weak, Self::prompt_create_folder),
                import_files: callback0(&weak, Self::import_files),
                drop_files: callback1(&weak, Self::import_paths_at),
                move_path: callback1(&weak, Self::move_path),
                toggle_hidden: callback0(&weak, Self::toggle_hidden),
                rename_path: callback1(&weak, Self::rename_path),
                duplicate_path: callback1(&weak, Self::duplicate_path),
                trash_path: callback1(&weak, Self::trash_path),
                reveal_path: callback1(&weak, Self::reveal_path),
                open_external: callback1(&weak, Self::open_external),
                preview_asset: callback1(&weak, Self::preview_asset),
                check_update: callback0(&weak, Self::check_update),
                export_pdf: callback0(&weak, Self::export_pdf),
                export_document: callback0(&weak, Self::choose_document_export),
                export_project: callback0(&weak, Self::export_current_project),
                present_single: callback0(&weak, Self::present_single),
                present_dual: callback0(&weak, Self::present_dual),
                refresh_compile: callback1(&weak, Self::compile),
                search: {
                    let weak = weak.clone();
                    Rc::new(move |mode, query, reply| {
                        if let Some(this) = weak.upgrade() {
                            this.search(mode, query, reply);
                        } else {
                            reply(Vec::new());
                        }
                    })
                },
                settings_changed: callback1(&weak, Self::settings_changed),
                preferences_changed: callback1(&weak, Self::preferences_changed),
            },
        );
        self.stack.add_named(&workspace.root, Some("workspace"));
        self.workspace.replace(Some(workspace));
        let save_note: Rc<dyn Fn(usize, String) -> bool> = {
            let weak = weak.clone();
            Rc::new(move |slide, text| {
                weak.upgrade()
                    .is_some_and(|this| this.save_presentation_note(slide, &text))
            })
        };
        let save_note_font_size: Rc<dyn Fn(u32)> = {
            let weak = weak.clone();
            Rc::new(move |size| {
                if let Some(this) = weak.upgrade() {
                    this.persist_presentation_note_font_size(size);
                }
            })
        };
        self.presentation.replace(Some(PresentationController::new(
            &self.application,
            &self.window,
            self.settings.borrow().presentation_notes_font_size,
            save_note,
            save_note_font_size,
        )));
    }

    fn install_actions(self: &Rc<Self>) {
        self.add_action("open", &["<Primary>o"], Self::choose_open_project);
        self.add_action("new", &["<Primary>n"], Self::choose_create_project);
        self.add_action("save", &["<Primary>s"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.request_save();
            }
        });
        self.add_action("jump-to-preview", &["<Primary><Shift>j"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.jump_to_preview();
            }
        });
        self.add_action("compile", &["<Primary>Return"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.request_compile();
            }
        });
        self.add_action(
            "export",
            &["<Primary><Shift>e", "<Primary><Shift>Return"],
            Self::export_pdf,
        );
        self.add_action("export-document", &[], Self::choose_document_export);
        self.add_action(
            "export-project",
            &["<Primary><Shift>s"],
            Self::export_current_project,
        );
        self.add_action("search", &["<Primary>k"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.present_search();
            }
        });
        self.add_action("find", &["<Primary>f", "<Primary>h"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.present_document_search();
            }
        });
        self.add_action("settings", &["<Primary>comma"], Self::show_settings);
        self.add_action("check-updates", &[], Self::check_update);
        self.add_action("whats-new", &[], Self::show_changelog);
        self.add_action("sidebar", &["<Primary>backslash"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.toggle_sidebar();
            }
        });
        self.add_action("comment", &["<Primary>slash"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.toggle_comment();
            }
        });
        self.add_action("format-document", &["<Primary><Shift>i"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.format_document();
            }
        });
        self.add_action("duplicate-line", &["<Primary>d"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.duplicate_lines();
            }
        });
        self.add_action("cycle-theme", &["<Primary>j"], Self::cycle_theme);
        self.add_action("view-source", &["<Primary>1"], |this| {
            this.set_view_mode(ViewMode::Source)
        });
        self.add_action("view-split", &["<Primary>2"], |this| {
            this.set_view_mode(ViewMode::Split)
        });
        self.add_action("view-preview", &["<Primary>3"], |this| {
            this.set_view_mode(ViewMode::Preview)
        });
        self.add_action("minimap", &["<Primary><Shift>m"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.toggle_minimap();
            }
        });
        self.add_action("quit", &["<Primary>q"], |this| {
            this.window.close();
        });
        self.add_action(
            "present",
            &["F5", "<Primary><Shift>p"],
            Self::present_single,
        );
        self.add_action(
            "presenter",
            &["<Shift>F5", "<Primary><Alt>p"],
            Self::present_dual,
        );
        self.add_action("home", &["<Primary><Shift>h"], Self::close_to_home);
        self.window.connect_close_request({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(this) = weak.upgrade() {
                    if this.update_phase.get() == UpdatePhase::Installing {
                        return glib::Propagation::Stop;
                    }
                    if this
                        .workspace
                        .borrow()
                        .as_ref()
                        .is_some_and(|workspace| !workspace.save_before_navigation())
                    {
                        return glib::Propagation::Stop;
                    }
                    if let Some(presentation) = this.presentation.borrow().as_ref() {
                        if !presentation.end() {
                            return glib::Propagation::Stop;
                        }
                    }
                    if let Some(store) = &this.state_store {
                        let _ = store.save_window_state(BackendWindowState {
                            width: this.window.width(),
                            height: this.window.height(),
                            maximized: this.window.is_maximized(),
                        });
                    }
                }
                glib::Propagation::Proceed
            }
        });
    }

    fn add_action(self: &Rc<Self>, name: &str, accelerators: &[&str], activate: fn(&Self)) {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                activate(&this);
            }
        });
        self.application.add_action(&action);
        self.application
            .set_accels_for_action(&format!("app.{name}"), accelerators);
    }

    fn show_home(&self) {
        self.stack.set_visible_child_name("home");
        self.window.set_title(Some("typsmthng"));
    }

    fn close_to_home(&self) {
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            if !workspace.save_before_navigation() {
                return;
            }
        }
        if let Some(presentation) = self.presentation.borrow().as_ref() {
            if !presentation.end() {
                return;
            }
        }
        self.compile_generation
            .set(self.compile_generation.get().wrapping_add(1));
        self.pending_compile_source.replace(None);
        if let Some(cancel) = self.compile_cancellation.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.cancel_pending_compile();
        }
        self.project.replace(None);
        self.current_file.replace(None);
        self.disk_baseline.replace(None);
        if let Some(store) = &self.state_store {
            let _ = store.set_reopen_project(None);
        }
        self.refresh_recents();
        self.show_home();
    }

    fn open_startup_path(&self, path: &Path) {
        let (root, file) = resolve_startup_path(path);
        self.open_project(
            &root,
            file.as_deref()
                .map(|path| path.to_string_lossy().into_owned()),
        );
    }

    fn open_project(&self, root: &Path, selected: Option<String>) {
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            if !workspace.save_before_navigation() {
                return;
            }
        }
        self.compile_generation
            .set(self.compile_generation.get().wrapping_add(1));
        self.pending_compile_source.replace(None);
        if let Some(cancel) = self.compile_cancellation.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
        match Project::open(root) {
            Ok(project) => {
                let recent = self
                    .state_store
                    .as_ref()
                    .and_then(|store| store.load_metadata().ok())
                    .and_then(|metadata| {
                        metadata
                            .recent_projects
                            .into_iter()
                            .find(|item| item.root_path == project.root())
                    });
                self.hidden_files.replace(
                    recent
                        .as_ref()
                        .is_some_and(|item| item.hidden_files_visible),
                );
                let entries = match project.entries(*self.hidden_files.borrow()) {
                    Ok(entries) => entries,
                    Err(error) => {
                        self.show_error("Could not read project", &error.to_string());
                        return;
                    }
                };
                let count = entries
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::File)
                    .count();
                let main = selected
                    .or_else(|| recent.and_then(|item| item.last_file_path))
                    .or_else(|| project.resolve_main_file(None).ok());
                if let Some(store) = &self.state_store {
                    if let Err(error) = store.upsert_recent(
                        project.root(),
                        project.name(),
                        count,
                        main.clone(),
                        true,
                    ) {
                        self.show_error("Could not update project history", &error.to_string());
                    }
                }
                let rows = entries
                    .iter()
                    .map(|entry| FileRow {
                        path: entry.path.clone(),
                        name: entry.name.clone(),
                        depth: entry.path.matches('/').count(),
                        is_directory: entry.kind == EntryKind::Directory,
                        is_binary: entry.is_binary,
                        is_main: main.as_deref() == Some(entry.path.as_str()),
                    })
                    .collect::<Vec<_>>();
                let name = project.name();
                self.workspace
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .set_project_root(project.root());
                self.compile_generation
                    .set(self.compile_generation.get().wrapping_add(1));
                self.compiled_main.replace(None);
                self.project.replace(Some(project));
                self.disk_baseline.replace(None);
                self.watcher.replace(ExternalWatcher::new(root).ok());
                self.workspace
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .set_project(&name, &rows);
                self.stack.set_visible_child_name("workspace");
                self.window.set_title(Some(&format!("{name} — typsmthng")));
                if let Some(path) = main {
                    self.select_file(path);
                }
            }
            Err(error) => self.show_error("Could not open project", &error.to_string()),
        }
    }

    fn choose_open_project(&self) {
        let chooser = gtk::FileDialog::builder()
            .title("Open a Typst project")
            .accept_label("Open")
            .build();
        chooser.select_folder(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(path)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        this.open_project(&path, None);
                    }
                }
            }
        });
    }

    fn choose_create_project(&self) {
        let chooser = gtk::FileDialog::builder()
            .title("Choose a parent folder")
            .accept_label("Choose")
            .build();
        chooser.select_folder(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(parent)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        this.prompt_name("New project", "Project name", move |this, name| {
                            match Project::create(&parent, &name) {
                                Ok(project) => {
                                    this.open_project(project.root(), Some("main.typ".into()))
                                }
                                Err(error) => {
                                    this.show_error("Could not create project", &error.to_string())
                                }
                            }
                        });
                    }
                }
            }
        });
    }

    fn select_file(&self, path: String) {
        if let Some(command) = path.strip_prefix(":command:") {
            match command {
                "format" => {
                    if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.format_document();
                    }
                }
                "compile" => {
                    if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.request_compile();
                    }
                }
                "jump-to-preview" => {
                    if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.jump_to_preview();
                    }
                }
                "export" => self.export_pdf(),
                "export-document" => self.choose_document_export(),
                "present" => self.present_single(),
                "presenter" => self.present_dual(),
                "new-file" => self.prompt_create_file(),
                "settings" => self.show_settings(),
                _ => {}
            }
            return;
        }
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        if self.current_file.borrow().as_deref() != Some(path.as_str()) {
            if let Some(workspace) = self.workspace.borrow().as_ref() {
                if !workspace.save_before_navigation() {
                    return;
                }
            }
        }
        match project.read_file(&path) {
            Ok(file) => {
                // A queued source belongs to the previous buffer. In particular,
                // opening an asset must never save that source over the asset.
                self.pending_compile_source.replace(None);
                if let Some(cancel) = self.compile_cancellation.borrow().as_ref() {
                    cancel.store(true, Ordering::Relaxed);
                }
                self.compile_generation
                    .set(self.compile_generation.get().wrapping_add(1));
                self.current_file.replace(Some(path.clone()));
                if let Some(store) = &self.state_store {
                    if let Err(error) = store.persist_last_file(project.root(), Some(path.clone()))
                    {
                        self.show_error("Could not update project history", &error.to_string());
                    }
                }
                match file.content {
                    FileContent::Text(contents) => {
                        self.disk_baseline
                            .replace(Some((path.clone(), contents.clone())));
                        self.workspace
                            .borrow()
                            .as_ref()
                            .unwrap()
                            .show_text_file(&path, &contents);
                        if path.ends_with(".typ") {
                            self.compile(contents);
                        }
                    }
                    FileContent::Binary(_) => {
                        self.disk_baseline.replace(None);
                        self.workspace
                            .borrow()
                            .as_ref()
                            .unwrap()
                            .show_binary_file(&project.root().join(&path));
                    }
                }
            }
            Err(error) => self.show_error("Could not open file", &error.to_string()),
        }
    }

    fn save_current(&self, source: String) -> bool {
        let (Some(project), Some(path)) = (
            self.project.borrow().clone(),
            self.current_file.borrow().clone(),
        ) else {
            return false;
        };
        self.write_current_checked(&project, &path, &source, "Could not save file")
    }

    fn force_save_current(&self, source: String) -> bool {
        let (Some(project), Some(path)) = (
            self.project.borrow().clone(),
            self.current_file.borrow().clone(),
        ) else {
            return false;
        };
        let disk = project
            .read_file(&path)
            .ok()
            .and_then(|file| match file.content {
                FileContent::Text(text) => Some(text),
                FileContent::Binary(_) => None,
            });
        if let Some(disk) = disk {
            self.disk_baseline.replace(Some((path.clone(), disk)));
        } else {
            self.disk_baseline.replace(None);
        }
        self.write_current_checked(&project, &path, &source, "Could not keep editor buffer")
    }

    fn write_current_checked(
        &self,
        project: &Project,
        path: &str,
        source: &str,
        error_title: &str,
    ) -> bool {
        let disk_source = project
            .read_file(path)
            .ok()
            .and_then(|file| match file.content {
                FileContent::Text(text) => Some(text),
                FileContent::Binary(_) => None,
            });
        let baseline = self.disk_baseline.borrow().clone();
        let baseline_for_path = baseline
            .as_ref()
            .filter(|(baseline_path, _)| baseline_path == path)
            .map(|(_, source)| source.as_str());
        let disk_changed =
            baseline_for_path.is_some_and(|baseline| disk_source.as_deref() != Some(baseline));
        if disk_changed && disk_source.as_deref() != Some(source) {
            if let Some(workspace) = self.workspace.borrow().as_ref() {
                workspace.show_conflict(path);
                workspace.set_compile_status("External change must be resolved before saving");
            }
            return false;
        }
        // Autosave may already have persisted this exact buffer. Avoid another
        // atomic rename, fsync, and watcher event on every preview request.
        if disk_source.as_deref() == Some(source) {
            self.disk_baseline
                .replace(Some((path.to_string(), source.to_string())));
            if let Some(workspace) = self.workspace.borrow().as_ref() {
                workspace.mark_saved();
            }
            return true;
        }
        match project.write_text_atomic(path, source) {
            Ok(_) => {
                self.disk_baseline
                    .replace(Some((path.to_string(), source.to_string())));
                if let Some(watcher) = self.watcher.borrow_mut().as_mut() {
                    let _ = watcher.suppress_own_write(path, Duration::from_secs(2));
                }
                if let Some(workspace) = self.workspace.borrow().as_ref() {
                    workspace.mark_saved();
                }
                true
            }
            Err(error) => {
                self.show_error(error_title, &error.to_string());
                false
            }
        }
    }

    fn compile(&self, source: String) {
        let Some(workspace) = self.workspace.borrow().as_ref().cloned() else {
            return;
        };
        if !self.settings.borrow().auto_compile && source.is_empty() {
            workspace.set_compile_status("Live compile paused");
            return;
        }
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        // Preview reads the editor buffer without writing it to disk. Saving
        // has its own timer and may be disabled or delayed independently.
        let current_source = self
            .current_file
            .borrow()
            .clone()
            .map(|path| (path, source.clone()));
        if let Some(cancel) = self.compile_cancellation.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
        let generation = self.compile_generation.get().wrapping_add(1);
        self.compile_generation.set(generation);
        if self.compile_in_flight.get() {
            if let Some(cancel) = self.compile_cancellation.borrow().as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
            self.pending_compile_source.replace(Some(source));
            workspace.set_compile_status("Compilation queued…");
            return;
        }
        self.compile_in_flight.set(true);
        workspace.set_compiling();
        let cancellation = Arc::new(AtomicBool::new(false));
        self.compile_cancellation
            .replace(Some(cancellation.clone()));
        let settings = self.settings.borrow().clone();
        if settings.google_fonts {
            self.prepare_project_fonts(project.clone(), source.clone());
        }
        let revision = workspace.revision();
        let compiler = self.preview_compiler.clone();
        let locale_paper = super::locale_page_size();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn({
            let current = self.current_file.borrow().clone();
            move || {
                let mut source_map = None;
                let mut main = current.clone().unwrap_or_else(|| "main.typ".into());
                // Main-file discovery walks the project, and page settings
                // read source/metadata. Both belong with compilation rather
                // than blocking GTK on every debounced edit.
                let result = (|| {
                    main = project.resolve_main_file(current.as_deref())?;
                    let mut options = compile_options_for_project(
                        Some(&project),
                        Some(&main),
                        &settings,
                        locale_paper,
                    )?;
                    options.cancellation = Some(cancellation);
                    if settings.google_fonts {
                        if let Some(path) = GoogleFontCache::default().cached_directory() {
                            options.font_paths.push(path);
                        }
                    }
                    compiler.compile_with_source(&project, &main, current_source, &options)
                })()
                .and_then(|output| {
                    let output = CompileOutput {
                        artifact: output.artifact.map(|preview| {
                            source_map = Some(preview.source_map);
                            preview.pages
                        }),
                        diagnostics: output.diagnostics,
                        stdout: output.stdout,
                        stderr: output.stderr,
                        elapsed: output.elapsed,
                    };
                    let output = prepare_preview(output, &settings.presentation_notes_layout)?;
                    Ok((output, Vec::new()))
                })
                .map_err(|error| error.to_string());
                let _ = sender.send(CompileFinished {
                    generation,
                    revision,
                    source_map,
                    main,
                    result,
                });
            }
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(20), move || {
            match receiver.try_recv() {
                Ok(finished) => {
                    if let Some(this) = weak.upgrade() {
                        this.compile_in_flight.set(false);
                        this.apply_compile_result(finished);
                        let pending = this.pending_compile_source.borrow_mut().take();
                        if pending.is_some() {
                            if let Some(workspace) = this.workspace.borrow().as_ref() {
                                workspace.request_compile();
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(this) = weak.upgrade() {
                        this.compile_in_flight.set(false);
                        let pending = this.pending_compile_source.borrow_mut().take();
                        if pending.is_some() {
                            if let Some(workspace) = this.workspace.borrow().as_ref() {
                                workspace.request_compile();
                            }
                        } else if let Some(workspace) = this.workspace.borrow().as_ref() {
                            workspace.set_compile_status(
                                "Compiler worker stopped. Compile again to retry.",
                            );
                        }
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn compile_options(&self) -> Result<CompileOptions, BackendError> {
        let project = self.project.borrow();
        let main = project.as_ref().and_then(|project| {
            project
                .resolve_main_file(self.current_file.borrow().as_deref())
                .ok()
        });
        compile_options_for_project(
            project.as_ref(),
            main.as_deref(),
            &self.settings.borrow(),
            super::locale_page_size(),
        )
    }

    fn apply_compile_result(&self, finished: CompileFinished) {
        if finished.generation != self.compile_generation.get() {
            return;
        }
        let Some(workspace) = self.workspace.borrow().as_ref().cloned() else {
            return;
        };
        if finished.revision != workspace.revision() {
            return;
        }
        let (output, inline_notes) = match finished.result {
            Ok(output) => output,
            Err(error) => {
                workspace.set_compile_error(&format!("Compile error: {error}"));
                return;
            }
        };
        let diagnostics = output
            .diagnostics
            .iter()
            .map(|diagnostic| DiagnosticRow {
                severity: match diagnostic.severity {
                    DiagnosticSeverity::Error => DiagnosticKind::Error,
                    DiagnosticSeverity::Warning => DiagnosticKind::Warning,
                    DiagnosticSeverity::Hint => DiagnosticKind::Hint,
                },
                path: diagnostic
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| finished.main.clone()),
                line: diagnostic.line,
                column: diagnostic.column,
                message: diagnostic.message.clone(),
            })
            .collect::<Vec<_>>();
        workspace.set_diagnostics(&diagnostics);
        let Some(preview) = output.artifact else {
            let detail = if output.stderr.trim().is_empty() {
                "Typst did not produce pages"
            } else {
                output.stderr.trim()
            };
            workspace.set_compile_error(&format!("Compile error: {detail}"));
            return;
        };
        let PreparedPreview {
            cache,
            paths,
            rendered_notes,
        } = preview;
        self.render_cache.replace(Some(cache));
        self.compiled_main.replace(Some(finished.main.clone()));
        workspace.set_source_map(finished.source_map);
        workspace.set_compiled_preview(&paths, &finished.main);
        workspace.set_compile_status(&format!("Compiled in {} ms", output.elapsed.as_millis()));
        if let Some(presentation) = self.presentation.borrow().as_ref() {
            let (inline_notes, sidecar_notes) =
                self.load_presentation_notes(paths.len(), &inline_notes);
            presentation.set_deck(paths, inline_notes, sidecar_notes, rendered_notes);
        }
        self.request_presentation_notes();
    }

    fn prepare_project_fonts(&self, project: Project, source: String) {
        if self.font_in_flight.get() {
            self.pending_fonts.replace(Some((project, source)));
            return;
        }
        self.font_in_flight.set(true);
        let root = project.root().to_path_buf();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = GoogleFontCache::default().prepare_project_report(&project, &source);
            let _ = sender.send(result);
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(30), move || {
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err(BackendError::Process("Font worker stopped".into()))
                }
            };
            if let Some(this) = weak.upgrade() {
                this.font_in_flight.set(false);
                let pending = this.pending_fonts.borrow_mut().take();
                let refresh = result.is_ok_and(|report| report.changed)
                    && this.settings.borrow().google_fonts
                    && this
                        .project
                        .borrow()
                        .as_ref()
                        .is_some_and(|project| project.root() == root);
                if refresh {
                    this.preview_compiler.invalidate_fonts();
                    if let Some(workspace) = this.workspace.borrow().as_ref() {
                        workspace.request_compile();
                    }
                } else if this.settings.borrow().google_fonts {
                    if let Some((project, source)) = pending {
                        if this
                            .project
                            .borrow()
                            .as_ref()
                            .is_some_and(|current| current.root() == project.root())
                        {
                            this.prepare_project_fonts(project, source);
                        }
                    }
                }
            }
            glib::ControlFlow::Break
        });
    }

    fn request_presentation_notes(&self) {
        if self.notes_in_flight.get()
            || self.compile_in_flight.get()
            || self.notes_generation.get() == Some(self.compile_generation.get())
            || self
                .presentation
                .borrow()
                .as_ref()
                .is_none_or(|view| view.window_count() == 0)
        {
            return;
        }
        let (Some(project), Some(main)) = (
            self.project.borrow().clone(),
            self.compiled_main.borrow().clone(),
        ) else {
            return;
        };
        let generation = self.compile_generation.get();
        let mut options = match self.compile_options() {
            Ok(options) => options,
            Err(error) => {
                if let Some(workspace) = self.workspace.borrow().as_ref() {
                    workspace.set_compile_status(&format!("Speaker notes unavailable: {error}"));
                }
                return;
            }
        };
        self.notes_in_flight.set(true);
        options.cancellation = self.compile_cancellation.borrow().clone();
        if self.settings.borrow().google_fonts {
            if let Some(path) = GoogleFontCache::default().cached_directory() {
                options.font_paths.push(path);
            }
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result =
                TypstTool::detect().and_then(|tool| tool.query_notes(&project, &main, &options));
            let _ = sender.send(result);
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(20), move || {
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err(BackendError::Process("Notes worker stopped".into()))
                }
            };
            if let Some(this) = weak.upgrade() {
                this.notes_in_flight.set(false);
                if generation == this.compile_generation.get() {
                    match result {
                        Ok(notes) => {
                            this.notes_generation.set(Some(generation));
                            if let Some(presentation) = this.presentation.borrow().as_ref() {
                                let (inline, _) =
                                    this.load_presentation_notes(presentation.page_count(), &notes);
                                presentation.set_inline_notes(inline);
                            }
                        }
                        Err(error) => {
                            if let Some(workspace) = this.workspace.borrow().as_ref() {
                                workspace.set_compile_status(&format!(
                                    "Speaker notes unavailable: {error}"
                                ));
                            }
                        }
                    }
                } else {
                    this.request_presentation_notes();
                }
            }
            glib::ControlFlow::Break
        });
    }

    fn load_presentation_notes(
        &self,
        page_count: usize,
        inline: &[InlineNote],
    ) -> (Vec<String>, Vec<String>) {
        let project = self.project.borrow();
        let current = self.compiled_main.borrow();
        notes::load(project.as_ref().zip(current.as_deref()), page_count, inline)
    }

    fn save_presentation_note(&self, slide: usize, text: &str) -> bool {
        let (Some(project), Some(current)) = (
            self.project.borrow().as_ref().cloned(),
            self.compiled_main.borrow().clone(),
        ) else {
            self.show_error(
                "Could not save speaker notes",
                "There is no open Typst project to receive this note.",
            );
            return false;
        };
        if let Err(error) = notes::save(&project, &current, slide, text) {
            self.show_error("Could not save speaker notes", &error.to_string());
            return false;
        }
        true
    }

    fn persist_presentation_note_font_size(&self, size: u32) {
        self.settings.borrow_mut().presentation_notes_font_size = size.clamp(12, 34);
        if let Some(store) = &self.state_store {
            let settings = settings_to_backend(&self.settings.borrow());
            if let Err(error) = store.save_settings(&settings) {
                self.show_error("Could not save presentation settings", &error.to_string());
            }
        }
    }

    fn choose_document_export(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("Export document")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("export", "Export");
        dialog.set_response_appearance("export", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("export"));
        dialog.set_close_response("cancel");
        let format = adw::ComboRow::builder()
            .title("Format")
            .model(&gtk::StringList::new(&[
                "PDF",
                "SVG pages (ZIP)",
                "PNG pages (ZIP)",
                "HTML (experimental)",
            ]))
            .build();
        let mut profiles = vec!["Default PDF"];
        profiles.extend(PdfStandard::ALL.iter().map(|standard| standard.label));
        let profile = adw::ComboRow::builder()
            .title("PDF profile")
            .model(&gtk::StringList::new(&profiles))
            .build();
        let group = adw::PreferencesGroup::new();
        group.add(&format);
        group.add(&profile);
        let description = gtk::Label::new(Some("Typst checks the selected profile's requirements during export. Documents may need a title, language, or image descriptions."));
        description.set_wrap(true);
        description.set_xalign(0.0);
        description.add_css_class("dim-label");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&group);
        content.append(&description);
        dialog.set_extra_child(Some(&content));
        format.connect_selected_notify({
            let profile = profile.clone();
            move |format| {
                profile.set_sensitive(format.selected() == 0);
                description.set_text(match format.selected() {
                    1 | 2 => "Every page is saved in one ZIP. Choose a new filename; page exports do not replace existing files.",
                    3 => "HTML export is experimental. It exports document structure and does not preserve the PDF page layout.",
                    _ => "Typst checks the selected profile's requirements during export. Documents may need a title, language, or image descriptions.",
                });
            }
        });
        let weak = self.weak();
        dialog.connect_response(None, move |_, response| {
            if response == "export" {
                if let Some(this) = weak.upgrade() {
                    let format = match format.selected() {
                        1 => ExportFormat::Svg,
                        2 => ExportFormat::Png,
                        3 => ExportFormat::Html,
                        _ => ExportFormat::Pdf(
                            profile
                                .selected()
                                .checked_sub(1)
                                .and_then(|index| PdfStandard::ALL.get(index as usize))
                                .copied(),
                        ),
                    };
                    this.export_document(format);
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    fn export_pdf(&self) {
        self.export_document(ExportFormat::Pdf(None));
    }

    fn export_document(&self, format: ExportFormat) {
        let (Some(project), Some(current)) = (
            self.project.borrow().clone(),
            self.current_file.borrow().clone(),
        ) else {
            self.show_error("Export document", "Open a Typst project first.");
            return;
        };
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            if !workspace.save_before_navigation() {
                return;
            }
        }
        let main = match project.resolve_main_file(Some(&current)) {
            Ok(main) => main,
            Err(error) => {
                self.show_error("Export failed", &error.to_string());
                return;
            }
        };
        let mut options = match self.compile_options() {
            Ok(options) => options,
            Err(error) => {
                self.show_error("Export failed", &error.to_string());
                return;
            }
        };
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.set_compile_status(&format!("Rendering {}…", format.label()));
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        options.cancellation = Some(cancellation.clone());
        let progress = adw::AlertDialog::builder()
            .heading("Exporting document")
            .body("Preparing fonts and compiling with Typst…")
            .build();
        progress.add_response("cancel", "Cancel");
        progress.set_close_response("cancel");
        progress.connect_response(None, {
            let cancellation = cancellation.clone();
            move |_, _| {
                cancellation.store(true, Ordering::Relaxed);
            }
        });
        progress.present(Some(&self.window));
        let google_fonts = self.settings.borrow().google_fonts;
        let source = project
            .read_file(&main)
            .ok()
            .and_then(|file| match file.content {
                FileContent::Text(source) => Some(source),
                FileContent::Binary(_) => None,
            })
            .unwrap_or_default();
        let project_name = project.name();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut options = options;
            if google_fonts {
                if let Ok(Some(path)) =
                    GoogleFontCache::default().prepare_project(&project, &source)
                {
                    options.font_paths.push(path);
                }
            }
            let result = TypstTool::detect()
                .and_then(|tool| tool.compile_export(&project, &main, format, &options));
            let _ = sender.send(result);
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            match receiver.try_recv() {
                Ok(result) => {
                    let cancelled = cancellation.load(Ordering::Relaxed);
                    progress.force_close();
                    if let Some(this) = weak.upgrade() {
                        if cancelled {
                            if let Some(workspace) = this.workspace.borrow().as_ref() {
                                workspace.set_compile_status("Export cancelled");
                            }
                            return glib::ControlFlow::Break;
                        }
                        match result {
                            Ok(output) => {
                                if let Some(artifact) = output.artifact {
                                    this.choose_export_destination(&project_name, artifact);
                                    if let Some(workspace) = this.workspace.borrow().as_ref() {
                                        workspace.set_compile_status(&format!(
                                            "{} rendered in {} ms",
                                            format.label(),
                                            output.elapsed.as_millis()
                                        ));
                                    }
                                } else {
                                    if let Some(workspace) = this.workspace.borrow().as_ref() {
                                        workspace.set_compile_status("Export failed");
                                    }
                                    this.show_error("Export failed", output.stderr.trim());
                                }
                            }
                            Err(error) => {
                                if let Some(workspace) = this.workspace.borrow().as_ref() {
                                    workspace.set_compile_status("Export failed");
                                }
                                this.show_error("Export failed", &error.to_string());
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    progress.force_close();
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn export_current_project(&self) {
        let Some(project) = self.project.borrow().clone() else {
            self.show_error("Export project", "Open a project first.");
            return;
        };
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            if !workspace.save_before_navigation() {
                return;
            }
        }
        let chooser = gtk::FileDialog::builder()
            .title("Export project ZIP")
            .accept_label("Export")
            .build();
        chooser.set_initial_name(Some(&format!("{}.zip", project.name())));
        chooser.save(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(destination)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        if let Err(error) = export_project(&project, destination) {
                            this.show_error("Could not export project", &error.to_string());
                        }
                    }
                }
            }
        });
    }

    fn choose_export_destination(&self, project_name: &str, artifact: ExportArtifact) {
        let chooser = gtk::FileDialog::builder()
            .title(format!("Export {}", artifact.format.label()))
            .accept_label("Export")
            .build();
        chooser.set_initial_name(Some(&format!(
            "{project_name}.{}",
            artifact.format.extension()
        )));
        chooser.save(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let Some(path) = result.as_ref().ok().and_then(|file| file.path()) {
                        if let Err(error) = artifact.save(&path) {
                            if let Some(this) = weak.upgrade() {
                                this.show_error("Could not save export", &error.to_string());
                            }
                        }
                    }
                }
            }
        });
    }

    fn present_single(&self) {
        if let Some(presentation) = self.presentation.borrow().as_ref() {
            presentation.start_single();
        }
        self.request_presentation_notes();
    }

    fn present_dual(&self) {
        if let Some(presentation) = self.presentation.borrow().as_ref() {
            presentation.start_presenter();
        }
        self.request_presentation_notes();
    }

    fn search(&self, mode: SearchMode, query: String, reply: SearchReply) {
        if mode == SearchMode::Commands {
            let rows = [
                ("Compile document", "Ctrl+Enter", "compile"),
                ("Format document", "Ctrl+Shift+I", "format"),
                ("Export PDF", "Ctrl+Shift+E", "export"),
                ("Show cursor in preview", "Ctrl+Shift+J", "jump-to-preview"),
                (
                    "Export document…",
                    "PDF profiles, SVG, PNG, HTML",
                    "export-document",
                ),
                ("Present here", "F5", "present"),
                ("Presenter view", "Shift+F5", "presenter"),
                ("New file", "File tree", "new-file"),
                ("Settings", "Ctrl+,", "settings"),
            ]
            .into_iter()
            .filter(|(name, _, _)| name.to_lowercase().contains(&query.to_lowercase()))
            .map(|(name, shortcut, command)| SearchResultRow {
                primary: name.into(),
                secondary: shortcut.into(),
                path: Some(format!(":command:{command}")),
                line: None,
                column: None,
            })
            .collect();
            reply(rows);
            return;
        }
        if self.search_in_flight.get() {
            self.pending_search
                .replace(Some(PendingSearch { mode, query, reply }));
            return;
        }
        let Some(project) = self.project.borrow().clone() else {
            reply(Vec::new());
            return;
        };
        self.search_in_flight.set(true);
        let hidden = *self.hidden_files.borrow();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rows = search_project(&project, mode, &query, hidden);
            let _ = sender.send(rows);
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(20), move || {
            let rows = match receiver.try_recv() {
                Ok(rows) => rows,
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
            };
            if let Some(this) = weak.upgrade() {
                this.search_in_flight.set(false);
                let pending = this.pending_search.borrow_mut().take();
                if let Some(pending) = pending {
                    this.search(pending.mode, pending.query, pending.reply);
                } else {
                    reply(rows);
                }
            }
            glib::ControlFlow::Break
        });
    }

    fn prompt_create_file(&self) {
        self.prompt_name("New file", "Path, for example chapter.typ", |this, name| {
            let Some(project) = this.project.borrow().clone() else {
                return;
            };
            match project.create_text_file(&name, "") {
                Ok(_) => {
                    this.refresh_project_files();
                    this.select_file(name);
                }
                Err(error) => this.show_error("Could not create file", &error.to_string()),
            }
        });
    }

    fn prompt_create_folder(&self) {
        self.prompt_name("New folder", "Folder path", |this, name| {
            let Some(project) = this.project.borrow().clone() else {
                return;
            };
            match project.create_folder(&name) {
                Ok(_) => this.refresh_project_files(),
                Err(error) => this.show_error("Could not create folder", &error.to_string()),
            }
        });
    }

    fn import_files(&self) {
        let chooser = gtk::FileDialog::builder()
            .title("Import files into vault")
            .accept_label("Import")
            .build();

        chooser.open_multiple(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if let Ok(files) = result {
                    if let Some(this) = weak.upgrade() {
                        let paths = (0..files.n_items())
                            .filter_map(|index| files.item(index))
                            .filter_map(|item| item.downcast::<gio::File>().ok())
                            .filter_map(|file| file.path())
                            .collect::<Vec<_>>();
                        if !paths.is_empty() {
                            this.import_paths(paths);
                        }
                    }
                }
            }
        });
    }

    fn import_paths(&self, paths: Vec<PathBuf>) {
        self.import_paths_at((paths, String::new()));
    }

    fn import_paths_at(&self, (paths, target_directory): (Vec<PathBuf>, String)) {
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        let mut conversion_warnings = Vec::new();
        let mut import_errors = Vec::new();
        for path in paths {
            let Some(name) = path.file_name() else {
                continue;
            };
            let base = unique_project_path(&project, &Path::new(&target_directory).join(name));
            if path.is_dir()
                && path.canonicalize().is_ok_and(|source| {
                    source.starts_with(project.root()) || project.root().starts_with(source)
                })
            {
                import_errors.push(format!(
                    "{}: cannot recursively import a project into itself",
                    path.display()
                ));
                continue;
            }
            if let Err(error) = import_path_tree(&project, &path, &base, &mut conversion_warnings) {
                import_errors.push(format!("{}: {error}", path.display()));
            }
        }
        self.refresh_project_files();
        if !conversion_warnings.is_empty() {
            self.show_notice(
                "LaTeX imported with warnings",
                &format!(
                    "{} construct(s) need review:\n\n{}",
                    conversion_warnings.len(),
                    conversion_warnings.join("\n")
                ),
            );
        }
        if !import_errors.is_empty() {
            self.show_error(
                "Some files could not be imported",
                &import_errors.join("\n"),
            );
        }
    }

    fn toggle_hidden(&self) {
        let value = !*self.hidden_files.borrow();
        self.hidden_files.replace(value);
        if let (Some(store), Some(project)) = (&self.state_store, self.project.borrow().as_ref()) {
            if let Err(error) = store.set_hidden_files_visible(project.root(), value) {
                self.show_error("Could not save hidden-file setting", &error.to_string());
            }
        }
        self.refresh_project_files();
    }

    fn rename_path(&self, path: String) {
        self.prompt_name("Rename path", "New relative path", move |this, name| {
            if this
                .workspace
                .borrow()
                .as_ref()
                .is_some_and(|workspace| !workspace.save_before_navigation())
            {
                return;
            }
            let Some(project) = this.project.borrow().clone() else {
                return;
            };
            match project.rename(&path, &name) {
                Ok(()) => {
                    let current = this.current_file.borrow().clone();
                    if let Some(current) = current {
                        if let Some(rewritten) = rewrite_descendant_path(&current, &path, &name) {
                            this.current_file.replace(Some(rewritten));
                        }
                    }
                    this.refresh_project_files();
                }
                Err(error) => this.show_error("Could not rename path", &error.to_string()),
            }
        });
    }

    fn move_path(&self, (source, target_directory): (String, String)) {
        if self
            .workspace
            .borrow()
            .as_ref()
            .is_some_and(|workspace| !workspace.save_before_navigation())
        {
            return;
        }
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        if !target_directory.is_empty() && is_path_or_descendant(&target_directory, &source) {
            self.show_error(
                "Could not move path",
                "A folder cannot be moved into itself.",
            );
            return;
        }
        let Some(name) = Path::new(&source).file_name() else {
            return;
        };
        let target = Path::new(&target_directory).join(name);
        if target == Path::new(&source) {
            return;
        }
        match project.rename(&source, &target) {
            Ok(()) => {
                let target = target.to_string_lossy().into_owned();
                if let Some(current) = self.current_file.borrow().clone() {
                    if let Some(rewritten) = rewrite_descendant_path(&current, &source, &target) {
                        self.current_file.replace(Some(rewritten));
                    }
                }
                self.refresh_project_files();
            }
            Err(error) => self.show_error("Could not move path", &error.to_string()),
        }
    }

    fn duplicate_path(&self, path: String) {
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        let source = Path::new(&path);
        let stem = source
            .file_stem()
            .map(|v| v.to_string_lossy())
            .unwrap_or_default();
        let extension = source
            .extension()
            .map(|v| format!(".{}", v.to_string_lossy()))
            .unwrap_or_default();
        let target = source.with_file_name(format!("{stem} copy{extension}"));
        let target = unique_project_path(&project, &target);
        match project.duplicate(&path, &target) {
            Ok(_) => self.refresh_project_files(),
            Err(error) => self.show_error("Could not duplicate path", &error.to_string()),
        }
    }

    fn trash_path(&self, path: String) {
        if self
            .workspace
            .borrow()
            .as_ref()
            .is_some_and(|workspace| !workspace.save_before_navigation())
        {
            return;
        }
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        match project.move_to_trash(&path) {
            Ok(()) => {
                if self
                    .current_file
                    .borrow()
                    .as_deref()
                    .is_some_and(|current| is_path_or_descendant(current, &path))
                {
                    self.current_file.replace(None);
                }
                self.refresh_project_files();
            }
            Err(error) => self.show_error("Could not move path to trash", &error.to_string()),
        }
    }

    fn reveal_path(&self, path: String) {
        let Some(project) = self.project.borrow().as_ref().cloned() else {
            return;
        };
        let target = project.root().join(path);
        if reveal_in_file_manager(&target) {
            return;
        }
        let folder = target
            .parent()
            .unwrap_or_else(|| project.root())
            .to_path_buf();
        let uri = gio::File::for_path(folder).uri();
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            self.show_error("Could not reveal path", &error.to_string());
        }
    }

    fn open_external(&self, path: String) {
        let Some(project) = self.project.borrow().as_ref().cloned() else {
            return;
        };
        let uri = gio::File::for_path(project.root().join(path)).uri();
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            self.show_error("Could not open path", &error.to_string());
        }
    }

    fn preview_asset(&self, path: String) {
        let Some(project) = self.project.borrow().as_ref().cloned() else {
            return;
        };
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            if workspace.save_before_navigation() {
                workspace.show_binary_file(&project.root().join(path));
            }
        }
    }

    fn show_changelog(&self) {
        let notes =
            typsmthng_gtk::backend::release_notes::unseen_releases(None, env!("CARGO_PKG_VERSION"));
        self.present_changelog(&notes);
    }

    fn show_unseen_changelog(&self) {
        let Some(store) = &self.state_store else {
            return;
        };
        let Ok(metadata) = store.load_metadata() else {
            return;
        };
        let notes = typsmthng_gtk::backend::release_notes::unseen_releases(
            metadata.last_seen_version.as_deref(),
            env!("CARGO_PKG_VERSION"),
        );
        if !notes.is_empty() {
            self.present_changelog(&notes);
        }
    }

    fn present_changelog(&self, notes: &[typsmthng_gtk::backend::release_notes::ReleaseNotes]) {
        let existing = self.changelog_dialog.borrow().clone();
        if let Some(dialog) = existing {
            dialog.present(Some(&self.window));
            return;
        }
        if notes.is_empty() {
            return;
        }
        let dialog = super::updates::changelog_dialog(notes);
        let weak = self.weak();
        dialog.connect_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.changelog_dialog.replace(None);
                if let Some(store) = &this.state_store {
                    if let Err(error) = store.mark_release_notes_seen(env!("CARGO_PKG_VERSION")) {
                        this.show_error("Could not remember the changelog", &error.to_string());
                    }
                }
            }
        });
        self.changelog_dialog.replace(Some(dialog.clone()));
        dialog.present(Some(&self.window));
    }

    fn present_update_dialog(&self) {
        let existing = self
            .update_dialog
            .borrow()
            .as_ref()
            .map(|view| view.dialog.clone());
        if let Some(dialog) = existing {
            dialog.present(Some(&self.window));
            return;
        }
        let weak = self.weak();
        let view = UpdateDialog::new(move || {
            if let Some(this) = weak.upgrade() {
                this.activate_update_primary();
            }
        });
        let weak = self.weak();
        view.dialog.connect_closed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.update_dialog.replace(None);
            }
        });
        let dialog = view.dialog.clone();
        self.update_dialog.replace(Some(view));
        self.refresh_update_controls();
        dialog.present(Some(&self.window));
    }

    fn refresh_update_controls(&self) {
        let phase = self.update_phase.get();
        let (received, total) = self.update_progress.get();
        let header = if phase == UpdatePhase::Downloading {
            total.filter(|total| *total > 0).map_or_else(
                || format!("Downloading… {:.1} MB", received as f64 / 1_048_576.0),
                |total| {
                    format!(
                        "Downloading… {}%",
                        (received.saturating_mul(100) / total).min(100)
                    )
                },
            )
        } else {
            phase.header_label().to_string()
        };
        self.update_button.set_label(&header);
        self.update_button.set_visible(true);
        self.update_button
            .set_sensitive(phase != UpdatePhase::Installing);
        if matches!(
            phase,
            UpdatePhase::Available
                | UpdatePhase::Ready
                | UpdatePhase::DownloadFailed
                | UpdatePhase::InstallFailed
        ) {
            self.update_button.add_css_class("suggested-action");
        } else {
            self.update_button.remove_css_class("suggested-action");
        }
        let updates = self.available_update.borrow();
        let available = updates.as_ref().and_then(|status| match status {
            UpdateStatus::Available {
                latest,
                release_notes,
                release_url,
                asset,
                ..
            } => Some((latest, release_notes, release_url, asset)),
            _ => None,
        });
        self.update_button
            .set_tooltip_text(Some(&available.map_or_else(
                || {
                    format!(
                        "Installed version {}. Check GitHub for updates.",
                        env!("CARGO_PKG_VERSION")
                    )
                },
                |(latest, _, _, _)| {
                    format!(
                        "Version {latest} is available. Review the changes and update typsmthng."
                    )
                },
            )));
        let dialogs = self.update_dialog.borrow();
        let Some(view) = dialogs.as_ref() else {
            return;
        };
        let current = env!("CARGO_PKG_VERSION");
        let summary = match phase {
            UpdatePhase::Checking => format!("Checking GitHub for a newer stable release. Installed version: {current}."),
            UpdatePhase::Current => format!("You're up to date. No newer stable release is available. Installed version: {current}."),
            UpdatePhase::Ready | UpdatePhase::InstallFailed => "The update is verified and ready. Restart to save your work and install it.".into(),
            UpdatePhase::Installing => "Preparing the updater. typsmthng will restart when it is ready.".into(),
            UpdatePhase::Downloading => if total.is_some_and(|total| total > 0 && received >= total) { "Verifying the downloaded update…".into() } else { "Downloading the update. You can keep working while it downloads.".into() },
            UpdatePhase::Available | UpdatePhase::DownloadFailed => available.map_or_else(|| "No compatible installer is available.".into(), |(latest, _, _, asset)| {
                let mut message = format!("Version {latest} is available. You're using {current}.");
                if asset.is_none() {
                    message.push_str(if managed_install() { "\n\nThis installation updates through your system software manager. GitHub may publish a version before it reaches your package repository." } else { "\n\nNo compatible installer was published. View the release for installation options." });
                    if managed_install() { message.push_str(&format!("\n\n{}", package_update_hint())); }
                }
                message
            }),
            UpdatePhase::CheckFailed => "Could not check GitHub for updates. Check your connection and try again.".into(),
            UpdatePhase::Idle => format!("Installed version: {current}. Check GitHub for a newer stable release."),
        };
        let message = self
            .update_error
            .borrow()
            .as_ref()
            .map_or_else(|| summary.clone(), |error| format!("{summary}\n\n{error}"));
        view.status.set_text(&message);
        let was_sensitive = view.primary.is_sensitive();
        view.primary.set_sensitive(!phase.busy());
        if !was_sensitive
            && !phase.busy()
            && view.dialog.is_mapped()
            && view
                .dialog
                .focus()
                .is_none_or(|focus| focus == view.primary.clone().upcast::<gtk::Widget>())
        {
            view.primary.grab_focus();
        }
        view.primary.set_label(match phase {
            UpdatePhase::Available | UpdatePhase::DownloadFailed => {
                if available.is_some_and(|(_, _, _, asset)| asset.is_some()) {
                    if phase == UpdatePhase::DownloadFailed {
                        "Retry download"
                    } else {
                        "Download update"
                    }
                } else if managed_install() {
                    "Open Software Updates"
                } else {
                    "View release"
                }
            }
            UpdatePhase::Ready | UpdatePhase::InstallFailed => "Restart to update",
            UpdatePhase::Downloading => "Downloading…",
            UpdatePhase::Installing => "Preparing restart…",
            UpdatePhase::Checking => "Checking…",
            UpdatePhase::CheckFailed => "Retry check",
            _ => "Check again",
        });
        view.progress.set_visible(matches!(
            phase,
            UpdatePhase::Downloading | UpdatePhase::Installing
        ));
        if phase == UpdatePhase::Downloading {
            if let Some(total) = total.filter(|total| *total > 0) {
                view.progress
                    .set_fraction((received as f64 / total as f64).min(1.0));
            } else {
                view.progress.pulse();
            }
            view.progress.set_text(Some(&header));
        } else if phase == UpdatePhase::Installing {
            view.progress.pulse();
        }
        if let Some((_, notes, url, _)) = available {
            view.set_notes(if notes.trim().is_empty() {
                "Release notes have not been provided for this version."
            } else {
                notes
            });
            view.release_link.set_uri(&safe_release_url(url));
        } else {
            view.set_notes("");
        }
    }

    fn check_update(&self) {
        self.check_update_with_feedback(true);
    }

    fn check_update_with_feedback(&self, announce_current: bool) {
        if self.downloaded_update.borrow().is_some() || self.update_phase.get().busy() {
            if announce_current {
                self.present_update_dialog();
            }
            return;
        }
        self.update_phase.set(UpdatePhase::Checking);
        self.update_error.replace(None);
        self.refresh_update_controls();
        if announce_current {
            self.present_update_dialog();
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(UpdateClient::default().check(env!("CARGO_PKG_VERSION")));
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err(BackendError::Network("Update worker stopped".into()))
                }
            };
            if let Some(this) = weak.upgrade() {
                match result {
                    Ok(status @ UpdateStatus::Available { .. }) => {
                        this.available_update.replace(Some(status));
                        this.update_phase.set(UpdatePhase::Available);
                    }
                    Ok(UpdateStatus::UpToDate { .. }) => {
                        this.available_update.replace(None);
                        this.update_phase.set(UpdatePhase::Current);
                    }
                    Err(error) => {
                        this.update_error.replace(Some(error.to_string()));
                        this.update_phase.set(UpdatePhase::CheckFailed);
                    }
                }
                this.refresh_update_controls();
            }
            glib::ControlFlow::Break
        });
    }

    fn activate_update(&self) {
        match self.update_phase.get() {
            UpdatePhase::Idle | UpdatePhase::Current | UpdatePhase::CheckFailed => {
                self.check_update()
            }
            UpdatePhase::Ready | UpdatePhase::InstallFailed => self.install_downloaded_update(),
            _ => self.present_update_dialog(),
        }
    }

    fn activate_update_primary(&self) {
        match self.update_phase.get() {
            UpdatePhase::Available | UpdatePhase::DownloadFailed => self.begin_update_download(),
            UpdatePhase::Ready | UpdatePhase::InstallFailed => self.install_downloaded_update(),
            UpdatePhase::Idle | UpdatePhase::Current | UpdatePhase::CheckFailed => {
                self.check_update()
            }
            _ => {}
        }
    }

    fn begin_update_download(&self) {
        if self.update_phase.get().busy() {
            return;
        }
        let Some(UpdateStatus::Available {
            asset, release_url, ..
        }) = self.available_update.borrow().clone()
        else {
            return;
        };
        let Some(asset) = asset else {
            let result = if managed_install() {
                open_system_updates()
            } else {
                gio::AppInfo::launch_default_for_uri(
                    &safe_release_url(&release_url),
                    None::<&gio::AppLaunchContext>,
                )
                .map_err(|error| error.to_string())
            };
            if let Err(error) = result {
                self.update_error.replace(Some(error));
                self.refresh_update_controls();
            }
            return;
        };
        if let Err(error) = typsmthng_gtk::backend::update_install::installed_target() {
            self.update_error.replace(Some(error.to_string()));
            self.update_phase.set(UpdatePhase::DownloadFailed);
            self.refresh_update_controls();
            return;
        }
        self.update_phase.set(UpdatePhase::Downloading);
        self.update_error.replace(None);
        self.update_progress.set((0, None));
        self.refresh_update_controls();
        let progress = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let total = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_progress = progress.clone();
        let worker_total = total.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<_, String> {
                let cache = directories::ProjectDirs::from("dev", "typsmthng", "typsmthng")
                    .ok_or_else(|| "Cannot locate update cache".to_string())?;
                let root = cache.cache_dir().join("updates");
                std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
                let directory = tempfile::Builder::new()
                    .prefix("update-")
                    .tempdir_in(root)
                    .map_err(|error| error.to_string())?;
                let path = UpdateClient::default()
                    .download_with_progress(
                        &asset,
                        directory.path().join(&asset.name),
                        |bytes, total| {
                            worker_progress.store(bytes, Ordering::Relaxed);
                            worker_total.store(total.unwrap_or(0), Ordering::Relaxed);
                        },
                    )
                    .map_err(|error| error.to_string())?;
                let hash = typsmthng_gtk::backend::update_install::hash_file(&path)
                    .map_err(|error| error.to_string())?;
                Ok((directory, path, hash))
            })();
            let _ = sender.send(result);
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    this.update_progress.set((
                        progress.load(Ordering::Relaxed),
                        match total.load(Ordering::Relaxed) {
                            0 => None,
                            total => Some(total),
                        },
                    ));
                    this.refresh_update_controls();
                    return glib::ControlFlow::Continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("Update worker stopped".into())
                }
            };
            match result {
                Ok(downloaded) => {
                    this.downloaded_update.replace(Some(downloaded));
                    this.update_phase.set(UpdatePhase::Ready);
                }
                Err(error) => {
                    this.update_phase.set(UpdatePhase::DownloadFailed);
                    this.update_error.replace(Some(error));
                }
            }
            this.refresh_update_controls();
            glib::ControlFlow::Break
        });
    }

    fn install_downloaded_update(&self) {
        use typsmthng_gtk::backend::update_install::{handoff, installed_target, InstallJob};
        if self.update_phase.get().busy() {
            return;
        }
        if self
            .workspace
            .borrow()
            .as_ref()
            .is_some_and(|workspace| !workspace.save_before_navigation())
        {
            return;
        }
        if self
            .presentation
            .borrow()
            .as_ref()
            .is_some_and(|presentation| !presentation.end())
        {
            return;
        }
        let target = match installed_target() {
            Ok(target) => target,
            Err(error) => {
                self.update_error.replace(Some(error.to_string()));
                self.update_phase.set(UpdatePhase::InstallFailed);
                self.refresh_update_controls();
                return;
            }
        };
        let downloaded = self.downloaded_update.borrow();
        let Some((_, path, sha256)) = downloaded.as_ref() else {
            return;
        };
        let job = InstallJob {
            artifact: path.clone(),
            target,
            sha256: sha256.clone(),
        };
        drop(downloaded);
        if let Some(store) = &self.state_store {
            let _ = store.save_window_state(BackendWindowState {
                width: self.window.width(),
                height: self.window.height(),
                maximized: self.window.is_maximized(),
            });
        }
        self.update_phase.set(UpdatePhase::Installing);
        self.update_error.replace(None);
        self.refresh_update_controls();
        // Saving happens before this short handshake. Keep edits disabled until
        // it succeeds or fails so no new unsaved input can be lost on restart.
        self.window.set_sensitive(false);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(handoff(&job));
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    this.refresh_update_controls();
                    return glib::ControlFlow::Continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err(std::io::Error::other("Updater worker stopped"))
                }
            };
            this.window.set_sensitive(true);
            match result {
                Ok(()) => {
                    if let Some((directory, _, _)) = this.downloaded_update.borrow_mut().take() {
                        let _ = directory.keep();
                    }
                    this.application.quit();
                }
                Err(error) => {
                    this.update_phase.set(UpdatePhase::InstallFailed);
                    this.update_error.replace(Some(error.to_string()));
                    this.refresh_update_controls();
                }
            }
            glib::ControlFlow::Break
        });
    }

    fn refresh_project_files(&self) {
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        let entries = match project.entries(*self.hidden_files.borrow()) {
            Ok(entries) => entries,
            Err(error) => {
                self.show_error("Could not refresh project", &error.to_string());
                return;
            }
        };
        let main = project
            .resolve_main_file(self.current_file.borrow().as_deref())
            .ok();
        let rows = entries
            .iter()
            .map(|entry| FileRow {
                path: entry.path.clone(),
                name: entry.name.clone(),
                depth: entry.path.matches('/').count(),
                is_directory: entry.kind == EntryKind::Directory,
                is_binary: entry.is_binary,
                is_main: main.as_deref() == Some(entry.path.as_str()),
            })
            .collect::<Vec<_>>();
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.set_project_root(project.root());
            workspace.set_project(&project.name(), &rows);
        }
        if let Some(store) = &self.state_store {
            let count = entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::File)
                .count();
            if let Err(error) = store.upsert_recent(
                project.root(),
                project.name(),
                count,
                self.current_file.borrow().clone(),
                true,
            ) {
                self.show_error("Could not update project history", &error.to_string());
            }
        }
    }

    fn poll_external_changes(&self) {
        let events = self
            .watcher
            .borrow_mut()
            .as_mut()
            .and_then(|watcher| watcher.drain().ok())
            .unwrap_or_default();
        if events.is_empty() {
            return;
        }
        let current = self.current_file.borrow().clone();
        let dirty = self
            .workspace
            .borrow()
            .as_ref()
            .is_some_and(WorkspaceView::is_dirty);
        let mut refresh_tree = false;
        for event in events {
            if !event.is_directory
                && current.as_deref() == Some(event.path.as_str())
                && matches!(
                    event.kind,
                    ExternalEventKind::Changed
                        | ExternalEventKind::Removed
                        | ExternalEventKind::Renamed
                )
            {
                if let Some(destination) = event.renamed_to.as_ref() {
                    self.current_file.replace(Some(destination.clone()));
                    let disk = self.project.borrow().as_ref().and_then(|project| {
                        project
                            .read_file(destination)
                            .ok()
                            .and_then(|file| match file.content {
                                FileContent::Text(text) => Some(text),
                                FileContent::Binary(_) => None,
                            })
                    });
                    self.disk_baseline
                        .replace(disk.map(|source| (destination.clone(), source)));
                    if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.set_current_path(destination);
                    }
                    if let (Some(store), Some(project)) =
                        (&self.state_store, self.project.borrow().as_ref())
                    {
                        if let Err(error) =
                            store.persist_last_file(project.root(), Some(destination.clone()))
                        {
                            self.show_error("Could not update project history", &error.to_string());
                        }
                    }
                    refresh_tree = true;
                } else if dirty {
                    if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.show_conflict(&event.path);
                    }
                } else if event.kind == ExternalEventKind::Changed {
                    self.select_file(event.path);
                } else if matches!(
                    event.kind,
                    ExternalEventKind::Removed | ExternalEventKind::Renamed
                ) {
                    self.current_file.replace(None);
                    self.disk_baseline.replace(None);
                    refresh_tree = true;
                    let fallback = self
                        .project
                        .borrow()
                        .as_ref()
                        .and_then(|project| project.resolve_main_file(None).ok());
                    if let Some(main) = fallback {
                        self.select_file(main);
                    } else if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.show_missing_file(&event.path);
                    }
                }
            } else {
                refresh_tree = true;
            }
        }
        if refresh_tree {
            self.refresh_project_files();
        }
    }

    fn settings_changed(&self, settings: UiSettings) {
        self.settings.replace(settings.clone());
        self.apply_theme(settings.theme);
        self.apply_ui_font(&settings);
        if settings.translucent {
            self.window.add_css_class("translucent");
        } else {
            self.window.remove_css_class("translucent");
        }
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.apply_settings(settings.clone());
        }
        if let Some(store) = &self.state_store {
            if let Err(error) = store.save_settings(&settings_to_backend(&settings)) {
                self.show_error("Could not save settings", &error.to_string());
            }
        }
        if self.project.borrow().is_some() {
            if let Some(workspace) = self.workspace.borrow().as_ref() {
                if workspace.editor.is_editable() {
                    self.compile(workspace.source_text());
                }
            }
        }
    }

    fn set_view_mode(&self, mode: ViewMode) {
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.set_view_mode(mode);
        }
    }

    fn preferences_changed(&self, settings: UiSettings) {
        self.settings.replace(settings.clone());
        if let Some(store) = &self.state_store {
            if let Err(error) = store.save_settings(&settings_to_backend(&settings)) {
                self.show_error("Could not save settings", &error.to_string());
            }
        }
    }

    fn apply_ui_font(&self, settings: &UiSettings) {
        let Some(gtk_settings) = gtk::Settings::default() else {
            return;
        };
        match ui_font_name(settings, &self.system_font) {
            Some(name) => gtk_settings.set_gtk_font_name(Some(&name)),
            None => gtk_settings.set_gtk_font_name(Some(&self.system_font)),
        }
    }

    /// Google families chosen earlier live in the app's font cache, not the
    /// system; register them again, then re-apply so widgets pick them up.
    fn restore_downloaded_fonts(&self, settings: &UiSettings) {
        let missing = [&settings.ui_font_family, &settings.editor_font_family]
            .into_iter()
            .filter(|family| !family.is_empty() && !app_fonts::family_is_available(family))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }
        let weak = self.weak();
        super::font_picker::ensure_registered_async(missing, move |results| {
            for (family, result) in &results {
                if let Err(error) = result {
                    eprintln!("Could not restore font {family:?}: {error}");
                }
            }
            if let Some(this) = weak.upgrade() {
                let settings = this.settings.borrow().clone();
                this.apply_ui_font(&settings);
                if let Some(workspace) = this.workspace.borrow().as_ref() {
                    workspace.apply_settings(settings);
                }
            }
        });
    }

    fn cycle_theme(&self) {
        let mut settings = self.settings.borrow().clone();
        settings.theme = match settings.theme {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::System,
        };
        self.settings_changed(settings);
    }

    fn apply_theme(&self, theme: Theme) {
        adw::StyleManager::default().set_color_scheme(match theme {
            Theme::System => adw::ColorScheme::Default,
            Theme::Light => adw::ColorScheme::ForceLight,
            Theme::Dark => adw::ColorScheme::ForceDark,
        });
        if let Some(home) = self.home.borrow().as_ref() {
            home.apply_theme(theme);
        }
    }

    /// Present a standalone font picker for screenshots. Configure with
    /// `TYPSMTHNG_SMOKE_FONT_KIND=ui|editor`, `TYPSMTHNG_SMOKE_FONT_QUERY`
    /// (opens the Google Fonts page), `TYPSMTHNG_SMOKE_FONT_CURRENT` and
    /// `TYPSMTHNG_SMOKE_UI_FONT` (downloads a Google family for the UI).
    fn show_font_picker_smoke(&self) {
        use super::font_picker::{FontPicker, FontPickerKind};
        let kind = match std::env::var("TYPSMTHNG_SMOKE_FONT_KIND").as_deref() {
            Ok("ui") => FontPickerKind::Ui,
            _ => FontPickerKind::Editor,
        };
        let picker = FontPicker::new(kind);
        picker.set_current(
            std::env::var("TYPSMTHNG_SMOKE_FONT_CURRENT")
                .ok()
                .as_deref(),
        );
        picker.connect_selected(|choice| {
            println!(
                "TYPESMTHNG_FONT_SELECTED {:?} {:?}",
                choice.family, choice.source
            );
        });
        if let Ok(query) = std::env::var("TYPSMTHNG_SMOKE_FONT_QUERY") {
            picker.show_google(&query);
        }
        // Download and register a Google family, then use it for the whole UI.
        if let Ok(family) = std::env::var("TYPSMTHNG_SMOKE_UI_FONT") {
            super::font_picker::ensure_registered_async(vec![family], |results| {
                for (family, result) in results {
                    match result {
                        Ok(files) => {
                            println!("TYPESMTHNG_FONT_REGISTERED {family:?} {}", files.len());
                            if let Some(settings) = gtk::Settings::default() {
                                settings.set_gtk_font_name(Some(&format!("{family} 11")));
                            }
                        }
                        Err(error) => eprintln!("TYPESMTHNG_FONT_FAILED {family:?} {error}"),
                    }
                }
            });
        }
        picker.present(Some(&self.window));
        // The smoke run exits with the process; keep the picker's state alive.
        std::mem::forget(picker);
    }

    fn show_settings(&self) {
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.present_settings();
        }
    }

    fn toggle_favorite(&self, path: String) {
        if let Some(store) = &self.state_store {
            if let Err(error) = store.toggle_favorite(Path::new(&path)) {
                self.show_error("Could not update favorite", &error.to_string());
                return;
            }
        }
        self.refresh_recents();
    }

    fn remove_recent(&self, path: String) {
        if let Some(store) = &self.state_store {
            if let Err(error) = store.remove_recent(Path::new(&path)) {
                self.show_error("Could not remove recent project", &error.to_string());
                return;
            }
        }
        self.refresh_recents();
    }

    fn rename_recent(&self, path: String) {
        self.prompt_name(
            "Rename project folder",
            "New folder name",
            move |this, name| {
                let old = PathBuf::from(&path);
                let Some(parent) = old.parent() else { return };
                let new = parent.join(name);
                match std::fs::rename(&old, &new) {
                    Ok(()) => {
                        if let Some(store) = &this.state_store {
                            let _ = store.remove_recent(&old);
                        }
                        this.open_project(&new, None);
                    }
                    Err(error) => this.show_error("Could not rename project", &error.to_string()),
                }
            },
        );
    }

    fn create_workspace(&self) {
        self.prompt_name("New workspace", "Workspace name", |this, name| {
            if let Some(store) = &this.state_store {
                if let Err(error) = store.create_workspace(name) {
                    this.show_error("Could not create workspace", &error.to_string());
                }
            }
            this.refresh_recents();
        });
    }

    fn select_workspace(&self, id: String) {
        let selected = (!id.is_empty()).then_some(id);
        if let Some(store) = &self.state_store {
            let current = store
                .load_metadata()
                .ok()
                .and_then(|metadata| metadata.selected_home_workspace_id);
            if current != selected {
                match store.select_workspace(selected) {
                    Ok(_) => self.refresh_recents(),
                    Err(error) => self.show_error("Could not select workspace", &error.to_string()),
                }
            }
        }
    }

    fn manage_workspace(&self) {
        let Some(store) = &self.state_store else {
            return;
        };
        let Ok(metadata) = store.load_metadata() else {
            return;
        };
        let Some(selected) = metadata.selected_home_workspace_id else {
            self.show_error("Manage workspace", "Select a workspace first.");
            return;
        };
        let Some(workspace) = metadata
            .home_workspaces
            .iter()
            .find(|workspace| workspace.id == selected)
        else {
            return;
        };
        let dialog = adw::AlertDialog::builder()
            .heading(&workspace.name)
            .body("Rename this workspace or delete it. Projects remain in All projects.")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("other", "Rename…");
        dialog.add_response("reject", "Delete");
        dialog.set_response_appearance("reject", adw::ResponseAppearance::Destructive);
        let weak = self.weak();
        dialog.connect_response(None, move |_dialog, response| {
            if let Some(this) = weak.upgrade() {
                if response == "other" {
                    let selected = selected.clone();
                    this.prompt_name("Rename workspace", "Workspace name", move |this, name| {
                        if let Some(store) = &this.state_store {
                            let _ = store.rename_workspace(&selected, name);
                        }
                        this.refresh_recents();
                    });
                } else if response == "reject" {
                    if let Some(store) = &this.state_store {
                        let _ = store.delete_workspace(&selected);
                    }
                    this.refresh_recents();
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    fn assign_project_workspace(&self, path: String) {
        self.assign_projects_workspace(vec![path]);
    }

    fn assign_projects_workspace(&self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        let Some(store) = &self.state_store else {
            return;
        };
        let metadata = match store.load_metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                self.show_error("Could not load workspaces", &error.to_string());
                return;
            }
        };
        let mut ids = vec![None];
        let mut labels = vec!["No workspace".to_string()];
        for workspace in metadata.home_workspaces {
            ids.push(Some(workspace.id));
            labels.push(workspace.name);
        }
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
        let picker = gtk::DropDown::from_strings(&label_refs);
        let dialog = adw::AlertDialog::builder()
            .heading("Move project to workspace")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("accept", "Move");
        picker.set_margin_top(18);
        picker.set_margin_bottom(18);
        picker.set_margin_start(18);
        picker.set_margin_end(18);
        dialog.set_extra_child(Some(&picker));
        let weak = self.weak();
        dialog.connect_response(None, move |_dialog, response| {
            if response == "accept" {
                if let Some(this) = weak.upgrade() {
                    if let Some(store) = &this.state_store {
                        let selected = ids.get(picker.selected() as usize).cloned().flatten();
                        let mut failures = Vec::new();
                        for path in &paths {
                            if let Err(error) =
                                store.assign_workspace(Path::new(path), selected.clone())
                            {
                                failures.push(format!("{path}: {error}"));
                            }
                        }
                        if !failures.is_empty() {
                            this.show_error(
                                "Some projects could not be moved",
                                &failures.join("\n"),
                            );
                        }
                    }
                    this.refresh_recents();
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    fn show_import_options(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("Choose an import source")
            .body("Archives preserve complete projects. LaTeX files and folders are converted to editable Typst while their assets are copied alongside them.")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("other", "Archive…");
        dialog.add_response("other2", "LaTeX files…");
        dialog.add_response("other3", "LaTeX folder…");
        let weak = self.weak();
        dialog.connect_response(None, move |_dialog, response| {
            if let Some(this) = weak.upgrade() {
                match response {
                    "other" => this.import_project_archive(),
                    "other2" => this.choose_latex_files(),
                    "other3" => this.choose_latex_folder(),
                    _ => {}
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    fn choose_latex_files(&self) {
        let chooser = gtk::FileDialog::builder()
            .title("Choose LaTeX files and assets")
            .accept_label("Continue")
            .build();

        chooser.open_multiple(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if let Ok(files) = result {
                    if let Some(this) = weak.upgrade() {
                        let paths = (0..files.n_items())
                            .filter_map(|index| files.item(index))
                            .filter_map(|item| item.downcast::<gio::File>().ok())
                            .filter_map(|file| file.path())
                            .collect::<Vec<_>>();
                        this.choose_latex_import_destination(paths);
                    }
                }
            }
        });
    }

    fn choose_latex_folder(&self) {
        let chooser = gtk::FileDialog::builder()
            .title("Choose a LaTeX project folder")
            .accept_label("Continue")
            .build();
        chooser.select_folder(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(path)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        this.choose_latex_import_destination(vec![path]);
                    }
                }
            }
        });
    }

    fn choose_latex_import_destination(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let chooser = gtk::FileDialog::builder()
            .title("Choose destination parent folder")
            .accept_label("Import here")
            .build();
        chooser.select_folder(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(parent)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        let paths = paths.clone();
                        this.prompt_name(
                            "Imported LaTeX project",
                            "Project name",
                            move |this, name| {
                                let destination = parent.join(&name);
                                if paths.iter().filter(|path| path.is_dir()).any(|path| {
                                    path.canonicalize()
                                        .is_ok_and(|source| destination.starts_with(source))
                                }) {
                                    this.show_error(
                                        "Could not import LaTeX",
                                        "The destination cannot be inside the source project.",
                                    );
                                    return;
                                }
                                match import_latex_sources(&paths, &parent, &name) {
                                    Ok((project, warnings)) => {
                                        this.open_project(project.root(), None);
                                        if !warnings.is_empty() {
                                            this.show_notice(
                                                "LaTeX imported with warnings",
                                                &warnings.join("\n"),
                                            );
                                        }
                                    }
                                    Err(error) => this
                                        .show_error("Could not import LaTeX", &error.to_string()),
                                }
                            },
                        );
                    }
                }
            }
        });
    }

    fn import_project_archive(&self) {
        let archive = gtk::FileDialog::builder()
            .title("Import project archive")
            .accept_label("Choose archive")
            .build();
        archive.open(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(archive_path)) =
                        (weak.upgrade(), result.as_ref().ok().and_then(|file| file.path()))
                    {
                        let parent = gtk::FileDialog::builder()
                            .title("Choose destination folder")
                            .accept_label("Import here")
                            .build();
                        parent.select_folder(Some(&this.window), None::<&gio::Cancellable>, {
                            let weak = this.weak();
                            move |result| {
                                if result.is_ok() {
                                    if let (Some(this), Some(folder)) =
                                        (weak.upgrade(), result.as_ref().ok().and_then(|file| file.path()))
                                    {
                                        let default_name = archive_path
                                            .file_stem()
                                            .map(|value| value.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| "Imported project".into());
                                        let extension = archive_path
                                            .extension()
                                            .and_then(|value| value.to_str())
                                            .unwrap_or_default()
                                            .to_ascii_lowercase();
                                        let imported = if extension == "tex" || extension == "ltx" {
                                            import_latex_file(&archive_path, &folder, &default_name)
                                                .map(|project| vec![project])
                                        } else {
                                            match import_projects(
                                                &archive_path,
                                                &folder,
                                                ArchiveLimits::default(),
                                            ) {
                                                Ok(projects) => Ok(projects),
                                                Err(BackendError::InvalidArchive(message))
                                                    if message == "multi-project archives must contain project folders" =>
                                                {
                                                    import_project(
                                                        &archive_path,
                                                        &folder,
                                                        &default_name,
                                                        ArchiveLimits::default(),
                                                    )
                                                    .map(|project| vec![project])
                                                }
                                                Err(error) => Err(error),
                                            }
                                            .and_then(|projects| {
                                                for project in &projects {
                                                    convert_latex_project_if_needed(project)?;
                                                }
                                                Ok(projects)
                                            })
                                        };
                                        match imported {
                                            Ok(projects) => {
                                                for project in &projects {
                                                    if let Some(store) = &this.state_store {
                                                        let count = project
                                                            .entries(false)
                                                            .map(|entries| {
                                                                entries
                                                                    .iter()
                                                                    .filter(|entry| entry.kind == EntryKind::File)
                                                                    .count()
                                                            })
                                                            .unwrap_or_default();
                                                        let main = project.resolve_main_file(None).ok();
                                                        let _ = store.upsert_recent(
                                                            project.root(),
                                                            project.name(),
                                                            count,
                                                            main,
                                                            true,
                                                        );
                                                    }
                                                }
                                                if let Some(project) = projects.first() {
                                                    this.open_project(project.root(), None);
                                                }
                                            }
                                            Err(error) => this.show_error(
                                                "Could not import project",
                                                &error.to_string(),
                                            ),
                                        }
                                    }
                                }

                            }
                        });

                    }
                }

            }
        });
    }

    fn export_all_projects(&self) {
        let paths = match self.state_store.as_ref().map(StateStore::load_metadata) {
            Some(Ok(metadata)) => metadata
                .recent_projects
                .into_iter()
                .map(|project| project.root_path.to_string_lossy().into_owned())
                .collect(),
            Some(Err(error)) => {
                self.show_error("Could not load project history", &error.to_string());
                return;
            }
            None => Vec::new(),
        };
        self.export_projects_by_paths(
            paths,
            "Export all recent projects",
            "typsmthng-all-projects.zip",
        );
    }

    fn export_selected_projects(&self, paths: Vec<String>) {
        self.export_projects_by_paths(
            paths,
            "Export selected projects",
            "typsmthng-selected-projects.zip",
        );
    }

    fn export_projects_by_paths(&self, paths: Vec<String>, title: &str, filename: &str) {
        if paths.is_empty() {
            return;
        }
        let chooser = gtk::FileDialog::builder()
            .title(title)
            .accept_label("Export")
            .build();
        chooser.set_initial_name(Some(filename));
        chooser.save(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(destination)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        let mut projects = Vec::new();
                        let mut failures = Vec::new();
                        for path in &paths {
                            match Project::open(path) {
                                Ok(project) => projects.push(project),
                                Err(error) => failures.push(format!("{path}: {error}")),
                            }
                        }
                        if !projects.is_empty() {
                            if let Err(error) = export_projects(&projects, &destination) {
                                failures.push(error.to_string());
                            }
                        }
                        if !failures.is_empty() {
                            this.show_error(
                                "Some projects were not exported",
                                &failures.join("\n"),
                            );
                        }
                    }
                }
            }
        });
    }

    fn create_from_template(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("New from template")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("accept", "Use starter");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        let starter_label = gtk::Label::new(Some("BUILT-IN STARTERS"));
        starter_label.set_halign(gtk::Align::Start);
        starter_label.add_css_class("eyebrow");
        content.append(&starter_label);
        let choices =
            gtk::DropDown::from_strings(&["Research starter", "Article", "Slides 16:9", "Report"]);
        content.append(&choices);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let universe_label = gtk::Label::new(Some("TYPST UNIVERSE"));
        universe_label.set_halign(gtk::Align::Start);
        universe_label.add_css_class("eyebrow");
        content.append(&universe_label);
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let search = gtk::SearchEntry::new();
        search.set_hexpand(true);
        search.set_placeholder_text(Some("Search templates, or paste @preview/name:version"));
        let search_button = gtk::Button::with_label("Search");
        search_row.append(&search);
        search_row.append(&search_button);
        content.append(&search_row);
        let status = gtk::Label::new(Some("Search the official Typst package index."));
        status.set_halign(gtk::Align::Start);
        status.add_css_class("muted");
        content.append(&status);
        let results = gtk::ListBox::new();
        results.set_selection_mode(gtk::SelectionMode::Single);
        results.add_css_class("boxed-list");
        let result_templates = Rc::new(RefCell::new(Vec::<UniverseTemplate>::new()));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_min_content_height(240);
        scroll.set_min_content_width(480);
        scroll.set_child(Some(&results));
        content.append(&scroll);
        dialog.set_extra_child(Some(&content));

        let run_search: Rc<dyn Fn()> = {
            let search = search.clone();
            let status = status.clone();
            let results = results.clone();
            let result_templates = result_templates.clone();
            Rc::new(move || {
                let query = search.text().trim().to_string();
                if query.starts_with("@preview/") || query.starts_with("@local/") {
                    let name = query
                        .split('/')
                        .nth(1)
                        .unwrap_or("Template")
                        .split(':')
                        .next()
                        .unwrap_or("Template")
                        .to_string();
                    let version = query
                        .rsplit(':')
                        .next()
                        .and_then(|value| semver::Version::parse(value).ok())
                        .unwrap_or_else(|| semver::Version::new(0, 0, 0));
                    result_templates.replace(vec![UniverseTemplate {
                        name,
                        version,
                        description: "Direct Typst package specification".into(),
                        spec: query,
                    }]);
                    populate_universe_results(&results, &result_templates.borrow());
                    status.set_text("Select the template below to initialize it.");
                    return;
                }
                status.set_text("Searching Typst Universe…");
                search.set_sensitive(false);
                let (sender, receiver) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = sender.send(UniverseClient::default().search(&query, 40));
                });
                let search = search.clone();
                let status = status.clone();
                let results = results.clone();
                let result_templates = result_templates.clone();
                glib::timeout_add_local(Duration::from_millis(25), move || {
                    match receiver.try_recv() {
                        Ok(Ok(found)) => {
                            search.set_sensitive(true);
                            status.set_text(&format!("{} templates found", found.len()));
                            result_templates.replace(found);
                            populate_universe_results(&results, &result_templates.borrow());
                            glib::ControlFlow::Break
                        }
                        Ok(Err(error)) => {
                            search.set_sensitive(true);
                            status.set_text(&format!("Search failed: {error}"));
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            search.set_sensitive(true);
                            status.set_text("Search worker stopped unexpectedly");
                            glib::ControlFlow::Break
                        }
                    }
                });
            })
        };
        search_button.connect_clicked({
            let run_search = run_search.clone();
            move |_| run_search()
        });
        search.connect_activate({
            let run_search = run_search.clone();
            move |_| run_search()
        });
        results.connect_row_activated({
            let weak = self.weak();
            let result_templates = result_templates.clone();
            let dialog = dialog.clone();
            move |_, row| {
                if let (Some(this), Some(template)) = (
                    weak.upgrade(),
                    result_templates.borrow().get(row.index() as usize).cloned(),
                ) {
                    dialog.close();
                    this.choose_universe_template(template);
                }
            }
        });
        dialog.connect_response(None, {
            let weak = self.weak();
            move |_dialog, response| {
                if response == "accept" {
                    if let Some(this) = weak.upgrade() {
                        let (template_id, source, auxiliary, layout_locked) = match choices.selected() {
                            1 => (
                                "article",
                                "#set page(paper: \"a4\")\n#set text(size: 11pt)\n\n= Article title\n\nStart writing here.\n",
                                Vec::new(),
                                true,
                            ),
                            2 => (
                                "slides-16-9",
                                "#set page(width: 13.333in, height: 7.5in, margin: 0.7in)\n#set text(size: 28pt)\n\n= Presentation\n\n#pagebreak()\n\n= Next slide\n",
                                Vec::new(),
                                true,
                            ),
                            3 => (
                                "report",
                                "#set page(paper: \"a4\", margin: 25mm)\n#set text(size: 11pt)\n\n= Report\n\n== Summary\n",
                                Vec::new(),
                                true,
                            ),
                            _ => (
                                "research-starter",
                                "= Research Starter\n\n== Abstract\nSummarize your contribution, methods, and key findings.\n\n== Introduction\nDescribe the context, problem, and why the work matters.\n\n== Method\nExplain your approach, assumptions, and data.\n\n== Results\nReport your most relevant outcomes.\n\n== Discussion\nInterpret results, limits, and future work.\n\n== References\n#bibliography(\"refs.bib\")\n",
                                vec![(
                                    "refs.bib".to_string(),
                                    "@article{sample2026,\n  title = {Replace with your first citation},\n  author = {Doe, Jane},\n  journal = {Journal Name},\n  year = {2026}\n}\n"
                                        .to_string(),
                                )],
                                false,
                            ),
                        };
                        this.choose_builtin_template(
                            template_id.to_string(),
                            source.to_string(),
                            auxiliary,
                            layout_locked,
                        );
                    }
                }
                }
        });
        dialog.present(Some(&self.window));
    }

    fn choose_builtin_template(
        &self,
        template_id: String,
        source: String,
        auxiliary: Vec<(String, String)>,
        layout_locked: bool,
    ) {
        let chooser = gtk::FileDialog::builder()
            .title("Choose a parent folder")
            .accept_label("Choose")
            .build();
        chooser.select_folder(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(parent)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        let source = source.clone();
                        let auxiliary = auxiliary.clone();
                        let template_id = template_id.clone();
                        this.prompt_name("Template project", "Project name", move |this, name| {
                            match Project::create(&parent, &name) {
                                Ok(project) => {
                                    let written = project
                                        .write_text_atomic("main.typ", &source)
                                        .and_then(|_| {
                                            for (path, contents) in &auxiliary {
                                                project.write_text_atomic(path, contents)?;
                                            }
                                            write_template_metadata(
                                                &project,
                                                "built-in",
                                                &format!("built-in/{template_id}"),
                                                "main.typ",
                                                layout_locked,
                                            )
                                        });
                                    match written {
                                        Ok(_) => this
                                            .open_project(project.root(), Some("main.typ".into())),
                                        Err(error) => this.show_error(
                                            "Could not create project",
                                            &error.to_string(),
                                        ),
                                    }
                                }
                                Err(error) => {
                                    this.show_error("Could not create project", &error.to_string())
                                }
                            }
                        });
                    }
                }
            }
        });
    }

    fn choose_universe_template(&self, template: UniverseTemplate) {
        let chooser = gtk::FileDialog::builder()
            .title(format!("Create from {}", template.spec))
            .accept_label("Choose parent")
            .build();
        chooser.select_folder(Some(&self.window), None::<&gio::Cancellable>, {
            let weak = self.weak();
            move |result| {
                if result.is_ok() {
                    if let (Some(this), Some(parent)) = (
                        weak.upgrade(),
                        result.as_ref().ok().and_then(|file| file.path()),
                    ) {
                        let template = template.clone();
                        this.prompt_name(
                            "Universe template project",
                            &template.name,
                            move |this, name| {
                                this.initialize_universe_template(
                                    template.spec.clone(),
                                    parent.join(name),
                                )
                            },
                        );
                    }
                }
            }
        });
    }

    fn initialize_universe_template(&self, spec: String, destination: PathBuf) {
        let progress = adw::AlertDialog::builder()
            .heading("Initializing Typst template…")
            .body(&spec)
            .build();
        progress.present(Some(&self.window));
        let (sender, receiver) = std::sync::mpsc::channel();
        let result_path = destination.clone();
        std::thread::spawn(move || {
            let result = TypstTool::detect().and_then(|tool| {
                tool.init_template(&spec, &destination, &CompileOptions::default())?;
                let project = Project::open(&destination)?;
                let entrypoint = project
                    .resolve_main_file(None)
                    .unwrap_or_else(|_| "main.typ".into());
                write_template_metadata(&project, "universe", &spec, &entrypoint, true)
            });
            let _ = sender.send(result.map(|()| result_path));
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            match receiver.try_recv() {
                Ok(result) => {
                    progress.close();
                    if let Some(this) = weak.upgrade() {
                        match result {
                            Ok(path) => this.open_project(&path, None),
                            Err(error) => {
                                this.show_error("Could not initialize template", &error.to_string())
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn show_guide(&self) {
        super::guide::window(&self.window).present();
    }

    fn prompt_name(
        &self,
        title: &str,
        placeholder: &str,
        complete: impl Fn(&Self, String) + 'static,
    ) {
        let dialog = adw::AlertDialog::builder().heading(title).build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("accept", "Create");
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("accept"));
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(placeholder));
        entry.set_activates_default(true);
        entry.set_margin_top(18);
        entry.set_margin_bottom(18);
        entry.set_margin_start(18);
        entry.set_margin_end(18);
        dialog.set_extra_child(Some(&entry));
        let weak = self.weak();
        let response_entry = entry.clone();
        dialog.connect_response(None, move |_dialog, response| {
            if response == "accept" {
                let value = response_entry.text().trim().to_owned();
                if !value.is_empty() {
                    if let Some(this) = weak.upgrade() {
                        complete(&this, value);
                    }
                }
            }
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    fn refresh_recents(&self) {
        let metadata = match self.state_store.as_ref().map(StateStore::load_metadata) {
            Some(Ok(metadata)) => metadata,
            Some(Err(error)) => {
                self.show_error("Could not load project history", &error.to_string());
                Default::default()
            }
            None => Default::default(),
        };
        let selected = metadata.selected_home_workspace_id.clone();
        let projects = metadata
            .recent_projects
            .into_iter()
            .filter(|project| {
                selected.as_ref().is_none_or(|workspace| {
                    metadata
                        .project_workspace_assignments
                        .get(project.root_path.to_string_lossy().as_ref())
                        == Some(workspace)
                })
            })
            .map(|project| RecentProjectRow {
                name: project.name,
                path: project.root_path.to_string_lossy().into_owned(),
                detail: project
                    .file_count
                    .map(|count| format!("{count} {}", if count == 1 { "file" } else { "files" }))
                    .unwrap_or_else(|| "Recent".into()),
                favorite: project.favorite,
            })
            .collect::<Vec<_>>();
        if let Some(home) = self.home.borrow().as_ref() {
            let workspaces = metadata
                .home_workspaces
                .into_iter()
                .map(|workspace| (workspace.id, workspace.name))
                .collect::<Vec<_>>();
            home.set_workspaces(&workspaces, selected.as_deref());
            home.set_recents(&projects);
        }
    }

    fn show_error(&self, title: &str, detail: &str) {
        let dialog = adw::AlertDialog::builder()
            .heading(title)
            .body(detail)
            .build();
        dialog.add_response("close", "Close");

        dialog.present(Some(&self.window));
    }

    fn show_notice(&self, title: &str, detail: &str) {
        let dialog = adw::AlertDialog::builder()
            .heading(title)
            .body(detail)
            .build();
        dialog.add_response("close", "Close");

        dialog.present(Some(&self.window));
    }

    fn weak(&self) -> Weak<Self> {
        self.self_weak.borrow().clone()
    }
}

fn managed_install() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none_or(|value| value.is_empty())
}

fn package_update_hint() -> &'static str {
    if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file() {
        "You can also run: flatpak update dev.typsmthng.Typsmthng"
    } else if glib::find_program_in_path("dnf").is_some() {
        "You can also run: sudo dnf upgrade --refresh typsmthng"
    } else if glib::find_program_in_path("apt").is_some() {
        "You can also run: sudo apt update && sudo apt install --only-upgrade typsmthng"
    } else {
        "Use your package manager to update typsmthng."
    }
}

fn open_system_updates() -> Result<(), String> {
    if std::env::var_os("FLATPAK_ID").is_none() && !Path::new("/.flatpak-info").is_file() {
        let gnome = ("gnome-software", &["--mode=updates"][..]);
        let kde = ("plasma-discover", &["--mode", "update"][..]);
        let programs = if std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("kde")
        {
            [kde, gnome]
        } else {
            [gnome, kde]
        };
        for (name, args) in programs {
            if let Some(program) = glib::find_program_in_path(name) {
                let arguments = std::iter::once(program.as_os_str())
                    .chain(args.iter().map(std::ffi::OsStr::new))
                    .collect::<Vec<_>>();
                gio::Subprocess::newv(&arguments, gio::SubprocessFlags::NONE)
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }
        }
    }
    gio::AppInfo::launch_default_for_uri(
        "appstream://dev.typsmthng.Typsmthng",
        None::<&gio::AppLaunchContext>,
    )
    .map_err(|error| {
        format!(
            "Could not open the system software manager: {error}. {}",
            package_update_hint()
        )
    })
}

fn safe_release_url(candidate: &str) -> String {
    url::Url::parse(candidate)
        .ok()
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("github.com")
                && url.username().is_empty()
                && url.password().is_none()
                && url
                    .path()
                    .starts_with("/aaditagrawal/typsmthng-desktop/releases/")
        })
        .map_or_else(
            || "https://github.com/aaditagrawal/typsmthng-desktop/releases".into(),
            |url| url.to_string(),
        )
}

fn find_css_widget(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_css_widget(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn search_project(
    project: &Project,
    mode: SearchMode,
    query: &str,
    hidden: bool,
) -> Vec<SearchResultRow> {
    match mode {
        SearchMode::Files => project
            .search_paths(query, 80, hidden)
            .map(|(rows, _)| {
                rows.into_iter()
                    .map(|row| SearchResultRow {
                        primary: row.entry.name,
                        secondary: row.entry.path.clone(),
                        path: Some(row.entry.path),
                        line: None,
                        column: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        SearchMode::Contents => project
            .search_text(query, 80, hidden)
            .map(|(rows, _)| {
                rows.into_iter()
                    .map(|row| SearchResultRow {
                        primary: format!("{}:{}:{}", row.path, row.line, row.column),
                        secondary: row.preview,
                        path: Some(row.path),
                        line: Some(row.line),
                        column: Some(row.column),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        SearchMode::Commands => unreachable!(),
    }
}

fn callback0(weak: &Weak<AppController>, callback: fn(&AppController)) -> Rc<dyn Fn()> {
    let weak = weak.clone();
    Rc::new(move || {
        if let Some(this) = weak.upgrade() {
            callback(&this);
        }
    })
}

fn callback1<T: 'static>(
    weak: &Weak<AppController>,
    callback: fn(&AppController, T),
) -> Rc<dyn Fn(T)> {
    let weak = weak.clone();
    Rc::new(move |value| {
        if let Some(this) = weak.upgrade() {
            callback(&this, value);
        }
    })
}

fn callback1_result<T: 'static>(
    weak: &Weak<AppController>,
    callback: fn(&AppController, T) -> bool,
) -> Rc<dyn Fn(T) -> bool> {
    let weak = weak.clone();
    Rc::new(move |value| weak.upgrade().is_some_and(|this| callback(&this, value)))
}

fn is_path_or_descendant(candidate: &str, parent: &str) -> bool {
    candidate == parent
        || candidate
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn rewrite_descendant_path(candidate: &str, from: &str, to: &str) -> Option<String> {
    if candidate == from {
        return Some(to.to_string());
    }
    candidate
        .strip_prefix(from)
        .filter(|suffix| suffix.starts_with('/'))
        .map(|suffix| format!("{to}{suffix}"))
}

fn settings_from_backend(settings: &UserSettings) -> UiSettings {
    UiSettings {
        theme: match settings.theme {
            BackendTheme::Light => Theme::Light,
            BackendTheme::Dark => Theme::Dark,
            BackendTheme::System => Theme::System,
        },
        font_size: settings.font_size.round() as u32,
        auto_compile: settings.auto_compile,
        preview_quality: settings.preview_quality.clone(),
        compile_delay_ms: settings.compile_delay_ms as u32,
        auto_save: settings.auto_save,
        auto_save_delay_ms: settings.auto_save_delay_ms.clamp(50, 60_000) as u32,
        save_on_focus_loss: settings.save_on_focus_loss,
        line_wrapping: settings.line_wrapping,
        line_numbers: settings.line_numbers,
        vim_mode: settings.vim_mode,
        page_size: settings.page_size.clone(),
        presentation_notes_layout: settings.presentation_notes_layout.clone(),
        presentation_notes_font_size: settings.presentation_notes_font_size.clamp(12, 34),
        system_fonts: settings.system_fonts_enabled,
        google_fonts: settings.google_fonts_enabled,
        translucent: settings.translucent,
        view_mode: ViewMode::from_id(&settings.view_mode),
        minimap: settings.minimap,
        centered_scrolling: settings.centered_scrolling,
        editor_font_family: settings.editor_font_family.clone(),
        editor_line_height: settings.editor_line_height.clamp(100, 200),
        editor_ligatures: settings.editor_ligatures,
        equation_highlighting: settings.equation_highlighting,
        equation_highlight_color: settings.equation_highlight_color.clone(),
        ui_font_family: settings.ui_font_family.clone(),
        ui_font_size: match settings.ui_font_size {
            0 => 0,
            size => size.clamp(6, 32),
        },
    }
}

fn settings_to_backend(settings: &UiSettings) -> UserSettings {
    UserSettings {
        font_size: settings.font_size as f64,
        auto_compile: settings.auto_compile,
        preview_quality: settings.preview_quality.clone(),
        compile_delay_ms: settings.compile_delay_ms as u64,
        auto_save: settings.auto_save,
        auto_save_delay_ms: settings.auto_save_delay_ms as u64,
        save_on_focus_loss: settings.save_on_focus_loss,
        line_wrapping: settings.line_wrapping,
        line_numbers: settings.line_numbers,
        theme: match settings.theme {
            Theme::Light => BackendTheme::Light,
            Theme::Dark => BackendTheme::Dark,
            Theme::System => BackendTheme::System,
        },
        vim_mode: settings.vim_mode,
        page_size: settings.page_size.clone(),
        presentation_notes_layout: settings.presentation_notes_layout.clone(),
        presentation_notes_font_size: settings.presentation_notes_font_size.clamp(12, 34),
        system_fonts_enabled: settings.system_fonts,
        google_fonts_enabled: settings.google_fonts,
        translucent: settings.translucent,
        view_mode: settings.view_mode.id().into(),
        minimap: settings.minimap,
        centered_scrolling: settings.centered_scrolling,
        editor_font_family: settings.editor_font_family.clone(),
        editor_line_height: settings.editor_line_height,
        editor_ligatures: settings.editor_ligatures,
        equation_highlighting: settings.equation_highlighting,
        equation_highlight_color: settings.equation_highlight_color.clone(),
        ui_font_family: settings.ui_font_family.clone(),
        ui_font_size: settings.ui_font_size,
        ..UserSettings::default()
    }
}

fn populate_universe_results(list: &gtk::ListBox, templates: &[UniverseTemplate]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for template in templates {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 3);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(10);
        row.set_margin_end(10);
        let title = gtk::Label::new(Some(&format!("{}  {}", template.name, template.version)));
        title.set_halign(gtk::Align::Start);
        title.add_css_class("section-title");
        row.append(&title);
        let detail = gtk::Label::new(Some(&template.description));
        detail.set_halign(gtk::Align::Start);
        detail.set_wrap(true);
        detail.add_css_class("muted");
        row.append(&detail);
        list.append(&row);
    }
}

fn reveal_in_file_manager(target: &Path) -> bool {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open")
        .arg("-R")
        .arg(target)
        .status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", target.display()))
        .status();
    #[cfg(target_os = "linux")]
    let status = {
        let uri = gio::File::for_path(target).uri();
        std::process::Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("['{uri}']"),
                "",
            ])
            .status()
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let status = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file manager reveal is unsupported",
    ));
    status.is_ok_and(|status| status.success())
}

fn write_template_metadata(
    project: &Project,
    source: &str,
    resolved_spec: &str,
    entrypoint: &str,
    layout_locked: bool,
) -> typsmthng_gtk::backend::Result<()> {
    project.create_folder(".typsmthng").or_else(|error| {
        if project.root().join(".typsmthng").is_dir() {
            Ok(())
        } else {
            Err(error)
        }
    })?;
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let metadata = serde_json::json!({
        "source": source,
        "resolvedSpec": resolved_spec,
        "templateEntrypoint": entrypoint.trim_start_matches('/'),
        "layoutLocked": layout_locked,
        "createdAt": created_at,
    });
    let serialized = serde_json::to_string_pretty(&metadata)
        .map_err(|error| BackendError::Process(error.to_string()))?;
    project
        .write_text_atomic(".typsmthng/template.json", &(serialized + "\n"))
        .map(|_| ())
}

fn compile_options_for_project(
    project: Option<&Project>,
    main: Option<&str>,
    settings: &UiSettings,
    locale_paper: &str,
) -> Result<CompileOptions, BackendError> {
    let owns_layout = project.is_some_and(|project| {
        project_layout_locked(project)
            || main
                .and_then(|main| project.read_file(main).ok())
                .and_then(|file| match file.content {
                    FileContent::Text(source) => Some(source),
                    FileContent::Binary(_) => None,
                })
                .is_some_and(|source| {
                    source
                        .lines()
                        .any(|line| line.trim_start().starts_with("#set page("))
                })
    });
    let page_preamble = if owns_layout {
        None
    } else {
        match effective_page_size(&settings.page_size, locale_paper) {
            "a4" if settings.page_size == "auto" => None,
            "presentation-16-9" => Some("#set page(width: 13.333in, height: 7.5in)".to_string()),
            paper => Some(format!("#set page(paper: \"{paper}\")")),
        }
    };
    CompileOptions {
        ignore_system_fonts: !settings.system_fonts,
        page_preamble,
        ..CompileOptions::default()
    }
    .resolved()
}

fn project_layout_locked(project: &Project) -> bool {
    project
        .read_file(".typsmthng/template.json")
        .ok()
        .and_then(|file| match file.content {
            FileContent::Text(source) => serde_json::from_str::<serde_json::Value>(&source).ok(),
            FileContent::Binary(_) => None,
        })
        .and_then(|metadata| {
            metadata
                .get("layoutLocked")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use typsmthng_gtk::backend::Project;

    use super::{project_layout_locked, write_template_metadata};

    #[test]
    fn background_compile_options_preserve_project_layout_without_gtk() {
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "background options").unwrap();
        let settings = super::UiSettings {
            page_size: "us-letter".into(),
            ..Default::default()
        };
        let check = |project: Project, settings: super::UiSettings| {
            std::thread::spawn(move || {
                let main = project.resolve_main_file(None).unwrap();
                super::compile_options_for_project(Some(&project), Some(&main), &settings, "a4")
                    .unwrap()
                    .page_preamble
            })
            .join()
            .unwrap()
        };
        assert_eq!(
            check(project.clone(), settings.clone()).as_deref(),
            Some("#set page(paper: \"us-letter\")")
        );
        project
            .write_text_atomic("main.typ", "#set page(width: 100pt, height: 200pt)\nBody")
            .unwrap();
        assert_eq!(check(project.clone(), settings.clone()), None);
        project.write_text_atomic("main.typ", "Body").unwrap();
        write_template_metadata(
            &project,
            "universe",
            "@preview/demo:1.0.0",
            "main.typ",
            true,
        )
        .unwrap();
        assert_eq!(check(project, settings), None);
    }

    #[test]
    #[ignore = "requires a display; run one exact filter under Xvfb"]
    fn native_live_typing_bursts_keep_event_loop_responsive() {
        use adw::prelude::*;
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use std::time::{Duration, Instant};
        adw::init().unwrap();
        let app: gtk::Application = adw::Application::builder()
            .application_id("dev.typsmthng.TypingTest")
            .build()
            .upcast();
        app.register(None::<&gio::Cancellable>).unwrap();
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "typing homework").unwrap();
        let mut source = include_str!("../../tests/fixtures/homework/main.typ").to_string();
        while source.lines().count() < 160 {
            source.push_str("// Homework notes, equations, and references.\n");
        }
        project.write_text_atomic("main.typ", &source).unwrap();
        let controller = super::AppController::build(
            &app,
            super::LaunchOptions {
                smoke_test: true,
                startup_path: Some(project.root().to_path_buf()),
                ..Default::default()
            },
        );
        let pump_until = |ready: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !ready() && Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(ready(), "typing regression timed out");
        };
        pump_until(&|| !controller.compile_in_flight.get());
        let workspace = controller.workspace.borrow().as_ref().unwrap().clone();
        let buffer = workspace.editor.buffer();
        buffer.place_cursor(&buffer.end_iter());
        let edits = Rc::new(Cell::new(0));
        let durations = Rc::new(RefCell::new(Vec::new()));
        let mut tick = 0;
        glib::timeout_add_local(Duration::from_millis(25), {
            let edits = edits.clone();
            let durations = durations.clone();
            move || {
                let phase = tick % 26;
                tick += 1;
                // Four bursts, with idle pauses that also exercise timed
                // saves and live compilation while the input loop is active.
                if phase >= 16 {
                    return glib::ControlFlow::Continue;
                }
                let began = Instant::now();
                buffer.insert_at_cursor("café λ ");
                durations.borrow_mut().push(began.elapsed());
                edits.set(edits.get() + 1);
                if edits.get() == 64 {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
        });
        let gaps = Rc::new(RefCell::new(Vec::new()));
        let heartbeat = glib::timeout_add_local(Duration::from_millis(5), {
            let gaps = gaps.clone();
            let mut previous = Instant::now();
            move || {
                gaps.borrow_mut().push(previous.elapsed());
                previous = Instant::now();
                glib::ControlFlow::Continue
            }
        });
        pump_until(&|| edits.get() == 64);
        workspace.request_compile();
        pump_until(&|| {
            !controller.compile_in_flight.get()
                && controller.pending_compile_source.borrow().is_none()
                && !workspace.is_dirty()
        });
        heartbeat.remove();
        let mut durations = durations.borrow_mut();
        durations.sort_unstable();
        let mut gaps = gaps.borrow_mut();
        gaps.sort_unstable();
        assert!(!gaps.is_empty(), "GTK must deliver heartbeats while typing");
        assert_eq!(
            workspace.source_text(),
            format!("{source}{}", "café λ ".repeat(64))
        );
        assert_eq!(
            fs::read_to_string(project.root().join("main.typ")).unwrap(),
            workspace.source_text()
        );
        println!(
            "160-line math homework, 64 edits at 25ms with 250ms pauses: insert median={:?} max={:?}; 5ms heartbeat median={:?} max={:?}",
            durations[durations.len() / 2], durations.last().unwrap(),
            gaps[gaps.len() / 2], gaps.last().unwrap()
        );
        controller.window.close();
    }

    #[test]
    #[ignore = "requires a display; run under Xvfb"]
    fn native_export_dialog_keeps_default_pdf_and_scopes_profiles() {
        use adw::prelude::*;
        adw::init().unwrap();
        let app: gtk::Application = adw::Application::builder()
            .application_id("dev.typsmthng.ExportTest")
            .build()
            .upcast();
        app.register(None::<&gio::Cancellable>).unwrap();
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "Export review").unwrap();
        let original = fs::read(project.root().join("main.typ")).unwrap();
        let controller = super::AppController::build(
            &app,
            super::LaunchOptions {
                smoke_test: true,
                startup_path: Some(project.root().to_path_buf()),
                ..Default::default()
            },
        );
        assert!(app.lookup_action("export-document").is_some());
        assert!(app
            .accels_for_action("app.export")
            .iter()
            .any(|accel| gtk::accelerator_parse(accel)
                == gtk::accelerator_parse("<Primary><Shift>e")));
        let capture = |phase: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
            while std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if let Some(base) = std::env::var_os("TYPSMTHNG_SNAPSHOT_DIR") {
                std::env::set_var(
                    "TYPSMTHNG_SNAPSHOT_DIR",
                    std::path::PathBuf::from(&base).join(phase),
                );
                super::super::smoke::capture_windows(&app);
                std::env::set_var("TYPSMTHNG_SNAPSHOT_DIR", base);
            }
        };
        capture("before");
        controller.choose_document_export();
        let window = controller
            .window
            .downcast_ref::<adw::ApplicationWindow>()
            .unwrap();
        let dialog = window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        fn find_rows(widget: &gtk::Widget, rows: &mut Vec<adw::ComboRow>) {
            if let Some(row) = widget.downcast_ref::<adw::ComboRow>() {
                rows.push(row.clone());
                return;
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                find_rows(&widget, rows);
                child = widget.next_sibling();
            }
        }
        let mut rows = Vec::new();
        find_rows(&dialog.extra_child().unwrap(), &mut rows);
        assert_eq!(rows.len(), 2);
        let format = &rows[0];
        let profile = &rows[1];
        assert_eq!(format.model().unwrap().n_items(), 4);
        assert_eq!(profile.model().unwrap().n_items(), 18);
        assert_eq!((format.selected(), profile.selected()), (0, 0));
        profile.set_selected(17);
        for selected in 1..=3 {
            format.set_selected(selected);
            assert!(!profile.is_sensitive());
            assert_eq!(profile.selected(), 17);
        }
        format.set_selected(0);
        assert!(profile.is_sensitive());
        capture("after");
        dialog.emit_by_name::<()>("response", &[&"cancel"]);
        dialog.force_close();
        assert_eq!(fs::read(project.root().join("main.typ")).unwrap(), original);
        assert!(!project.root().join("Export review.pdf").exists());
        assert!(!project.root().join("Export review.zip").exists());
        controller.window.close();
    }

    #[test]
    #[ignore = "requires a display; run under Xvfb"]
    #[cfg(target_os = "linux")]
    fn native_update_button_downloads_only_after_click() {
        use adw::prelude::*;
        use std::io::{Read, Write};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        adw::init().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let body = b"example update";
        use sha2::Digest;
        let checksum = hex::encode(sha2::Sha256::digest(body));
        let metadata = serde_json::json!({"tag_name": "v99.0.0", "html_url": base,
        "body": "- Review release notes before updating.\n- Render <b>literal text</b> safely.",
        "assets": [
            {"name": "typsmthng-linux-x64.AppImage", "browser_download_url": format!("{base}/app")},
            {"name": "SHA256SUMS", "browser_download_url": format!("{base}/sums")}
        ]})
        .to_string();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while server_requests.load(Ordering::SeqCst) < 5 && std::time::Instant::now() < deadline
            {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                };
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 4096];
                let count = socket.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                let response = if request.starts_with("GET /app ") {
                    if server_requests.load(Ordering::SeqCst) == 1 {
                        b"corrupt first download".to_vec()
                    } else {
                        body.to_vec()
                    }
                } else if request.starts_with("GET /sums ") {
                    format!("{checksum}  typsmthng-linux-x64.AppImage\n").into_bytes()
                } else {
                    metadata.as_bytes().to_vec()
                };
                server_requests.fetch_add(1, Ordering::SeqCst);
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                )
                .unwrap();
                socket.write_all(&response).unwrap();
            }
        });
        let installed = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("APPIMAGE", installed.path());
        std::env::set_var("TYPSMTHNG_UPDATE_API_URL", &base);
        let app: gtk::Application = adw::Application::builder()
            .application_id("dev.typsmthng.UpdateTest")
            .build()
            .upcast();
        app.register(None::<&gio::Cancellable>).unwrap();
        let controller = super::AppController::build(
            &app,
            super::LaunchOptions {
                smoke_test: true,
                ..Default::default()
            },
        );
        controller.check_update_with_feedback(false);
        controller.check_update_with_feedback(false); // Single-flight check.
        let pump_until = |condition: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !condition() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(condition(), "update UI timed out");
        };
        pump_until(&|| !controller.update_phase.get().busy());
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "background check downloaded an artifact"
        );
        assert!(controller.update_button.is_visible());
        assert_eq!(
            controller.update_button.label().as_deref(),
            Some("Update available")
        );
        controller.update_button.emit_clicked();
        assert!(controller.update_dialog.borrow().is_some());
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "reviewing release notes must not download the installer"
        );
        controller
            .update_dialog
            .borrow()
            .as_ref()
            .unwrap()
            .primary
            .emit_clicked();
        controller.check_update_with_feedback(false);
        pump_until(&|| controller.update_phase.get() == super::UpdatePhase::DownloadFailed);
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        assert!(controller.downloaded_update.borrow().is_none());
        assert!(controller
            .update_error
            .borrow()
            .as_deref()
            .unwrap()
            .contains("checksum mismatch"));
        controller
            .update_dialog
            .borrow()
            .as_ref()
            .unwrap()
            .primary
            .emit_clicked();
        pump_until(&|| controller.downloaded_update.borrow().is_some());
        assert_eq!(requests.load(Ordering::SeqCst), 5);
        assert_eq!(
            controller.update_button.label().as_deref(),
            Some("Restart to update")
        );
        assert_eq!(
            std::fs::read(installed.path()).unwrap(),
            b"",
            "download installed without restart click"
        );
        controller.check_update_with_feedback(false);
        controller.check_update_with_feedback(true);
        assert_eq!(controller.update_phase.get(), super::UpdatePhase::Ready);
        assert_eq!(
            requests.load(Ordering::SeqCst),
            5,
            "checking again must preserve a verified update"
        );
        server.join().unwrap();
        controller.window.close();
        std::env::remove_var("APPIMAGE");
        std::env::remove_var("TYPSMTHNG_UPDATE_API_URL");
    }

    #[test]
    #[ignore = "requires a display; run one exact filter under Xvfb"]
    #[cfg(target_os = "linux")]
    fn native_changelog_acknowledgment_and_manual_reopen() {
        use adw::prelude::*;
        let isolated = tempdir().unwrap();
        let previous_environment = [
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "TYPSMTHNG_UPDATE_API_URL",
        ]
        .map(|key| (key, std::env::var_os(key)));
        std::env::set_var("XDG_DATA_HOME", isolated.path().join("data"));
        std::env::set_var("XDG_CACHE_HOME", isolated.path().join("cache"));
        std::env::set_var("TYPSMTHNG_UPDATE_API_URL", "http://127.0.0.1:1/offline");
        adw::init().unwrap();
        let app: gtk::Application = adw::Application::builder()
            .application_id("dev.typsmthng.ChangelogTest")
            .build()
            .upcast();
        app.register(None::<&gio::Cancellable>).unwrap();
        let controller = super::AppController::build(&app, super::LaunchOptions::default());
        let store = controller.state_store.as_ref().unwrap();
        let pump = |ready: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !ready() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(ready(), "changelog lifecycle timed out");
        };
        pump(&|| controller.changelog_dialog.borrow().is_some());
        let first = controller
            .changelog_dialog
            .borrow()
            .clone()
            .expect("first launch changelog");
        pump(&|| {
            first
                .default_widget()
                .is_some_and(|button| button.has_focus())
        });
        assert!(
            store.load_metadata().unwrap().last_seen_version.is_none(),
            "display alone must not acknowledge unread notes"
        );
        controller.show_unseen_changelog();
        assert_eq!(controller.changelog_dialog.borrow().as_ref(), Some(&first));
        first
            .default_widget()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        pump(&|| controller.changelog_dialog.borrow().is_none());
        assert_eq!(
            store.load_metadata().unwrap().last_seen_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        controller.show_unseen_changelog();
        assert!(
            controller.changelog_dialog.borrow().is_none(),
            "same version should not show again"
        );
        app.activate_action("whats-new", None);
        let manual = controller
            .changelog_dialog
            .borrow()
            .clone()
            .expect("manual changelog action");
        pump(&|| {
            manual
                .default_widget()
                .is_some_and(|button| button.has_focus())
        });
        manual.close();
        pump(&|| controller.changelog_dialog.borrow().is_none());
        let mut metadata = store.load_metadata().unwrap();
        metadata.last_seen_version = Some("0.1.8".into());
        store.save_metadata(&metadata).unwrap();
        controller.show_unseen_changelog();
        let missed = controller.changelog_dialog.borrow().clone().unwrap();
        pump(&|| {
            missed
                .default_widget()
                .is_some_and(|button| button.has_focus())
        });
        fn labels(widget: &gtk::Widget, text: &mut Vec<String>) {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                text.push(label.text().into());
                assert!(!label.uses_markup());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                labels(&widget, text);
                child = widget.next_sibling();
            }
        }
        let mut text = Vec::new();
        labels(missed.child().unwrap().upcast_ref(), &mut text);
        for version in ["Version 0.1.9", "Version 0.1.10", "Version 0.1.11"] {
            assert!(
                text.iter().any(|line| line == version),
                "missing skipped release {version}"
            );
        }
        missed.close();
        pump(&|| controller.changelog_dialog.borrow().is_none());
        controller.window.close();
        for (key, value) in previous_environment {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    #[ignore = "requires a display; run one exact filter under Xvfb"]
    #[cfg(target_os = "linux")]
    fn native_update_check_errors_are_retryable_and_current_button_stays_visible() {
        use adw::prelude::*;
        use std::io::{Read, Write};
        use std::{
            sync::{atomic::Ordering, Arc},
            time::Duration,
        };
        adw::init().unwrap();
        let previous_appimage = std::env::var_os("APPIMAGE");
        std::env::remove_var("APPIMAGE");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_requests = requests.clone();
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            for (index, status) in ["503 Service Unavailable", "200 OK", "200 OK"]
                .into_iter()
                .enumerate()
            {
                let mut socket = loop {
                    if let Ok((socket, _)) = listener.accept() {
                        break socket;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "missing retry request"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).unwrap() > 0);
                worker_requests.fetch_add(1, Ordering::SeqCst);
                let body = serde_json::json!({"tag_name": if index == 2 { "v99.0.0".to_string() } else { format!("v{}", env!("CARGO_PKG_VERSION")) }, "html_url": "https://github.com/aaditagrawal/typsmthng-desktop/releases", "body": "- A new managed update.", "assets": [{"name": "typsmthng-linux-x64.AppImage", "browser_download_url": "https://example.invalid/not-requested"}]}).to_string();
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        std::env::set_var("TYPSMTHNG_UPDATE_API_URL", endpoint);
        let app: gtk::Application = adw::Application::builder()
            .application_id("dev.typsmthng.UpdateRetryTest")
            .build()
            .upcast();
        app.register(None::<&gio::Cancellable>).unwrap();
        let controller = super::AppController::build(
            &app,
            super::LaunchOptions {
                smoke_test: true,
                ..Default::default()
            },
        );
        assert!(controller.update_button.is_visible());
        assert_eq!(
            controller.update_button.label().as_deref(),
            Some("Check for updates")
        );
        let pump = || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while controller.update_phase.get().busy() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!controller.update_phase.get().busy());
        };
        controller.update_button.emit_clicked();
        pump();
        assert_eq!(
            controller.update_phase.get(),
            super::UpdatePhase::CheckFailed
        );
        assert!(controller.update_error.borrow().is_some());
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        controller.update_button.emit_clicked();
        pump();
        assert_eq!(controller.update_phase.get(), super::UpdatePhase::Current);
        assert!(controller.update_button.is_visible());
        assert_eq!(
            controller.update_button.label().as_deref(),
            Some("Check for updates")
        );
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        controller.update_button.emit_clicked();
        pump();
        assert_eq!(controller.update_phase.get(), super::UpdatePhase::Available);
        assert!(matches!(
            controller.available_update.borrow().as_ref(),
            Some(typsmthng_gtk::backend::UpdateStatus::Available { asset: None, .. })
        ));
        assert_eq!(
            controller
                .update_dialog
                .borrow()
                .as_ref()
                .unwrap()
                .primary
                .label()
                .as_deref(),
            Some("Open Software Updates")
        );
        assert!(controller.downloaded_update.borrow().is_none());
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        worker.join().unwrap();
        controller.window.close();
        std::env::remove_var("TYPSMTHNG_UPDATE_API_URL");
        if let Some(previous) = previous_appimage {
            std::env::set_var("APPIMAGE", previous);
        }
    }

    #[test]
    #[ignore = "changes process environment; run one exact filter"]
    #[cfg(target_os = "linux")]
    fn system_update_handoff_uses_the_desktop_software_manager() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempdir().unwrap();
        let marker = directory.path().join("arguments");
        for name in ["gnome-software", "plasma-discover"] {
            let path = directory.path().join(name);
            fs::write(
                &path,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$TYPSMTHNG_UPDATE_TEST_MARKER\"\n",
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let previous = [
            "PATH",
            "XDG_CURRENT_DESKTOP",
            "FLATPAK_ID",
            "TYPSMTHNG_UPDATE_TEST_MARKER",
        ]
        .map(|key| (key, std::env::var_os(key)));
        std::env::set_var("PATH", directory.path());
        std::env::remove_var("FLATPAK_ID");
        std::env::set_var("TYPSMTHNG_UPDATE_TEST_MARKER", &marker);
        for (desktop, expected) in [("GNOME", "--mode=updates\n"), ("KDE", "--mode\nupdate\n")] {
            std::env::set_var("XDG_CURRENT_DESKTOP", desktop);
            let _ = fs::remove_file(&marker);
            super::open_system_updates().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while fs::read_to_string(&marker).ok().as_deref() != Some(expected)
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert_eq!(fs::read_to_string(&marker).unwrap(), expected);
        }
        for (key, value) in previous {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    fn release_links_cannot_open_other_uri_schemes_or_repositories() {
        let fallback = "https://github.com/aaditagrawal/typsmthng-desktop/releases";
        for candidate in [
            "javascript:alert(1)",
            "file:///tmp/app",
            "https://example.com/update",
            "https://github.com/another/app/releases/tag/v1",
        ] {
            assert_eq!(super::safe_release_url(candidate), fallback);
        }
        assert_eq!(
            super::safe_release_url(&format!("{fallback}/tag/v0.1.11")),
            format!("{fallback}/tag/v0.1.11")
        );
    }

    #[test]
    fn background_search_keeps_file_and_content_modes_distinct() {
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "search").unwrap();
        project
            .write_text_atomic("chapter.typ", "first\nneedle café")
            .unwrap();
        project
            .write_text_atomic("needle.typ", "unrelated")
            .unwrap();
        project.write_text_atomic(".secret.typ", "needle").unwrap();
        let files = super::search_project(&project, super::SearchMode::Files, "needle", false);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path.as_deref(), Some("needle.typ"));
        assert_eq!(files[0].line, None);
        let contents =
            super::search_project(&project, super::SearchMode::Contents, "needle", false);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].path.as_deref(), Some("chapter.typ"));
        assert_eq!((contents[0].line, contents[0].column), (Some(2), Some(1)));
        let hidden = super::search_project(&project, super::SearchMode::Contents, "needle", true);
        assert_eq!(hidden.len(), 2);
    }

    #[test]
    fn template_metadata_is_native_and_controls_layout() {
        let directory = tempdir().unwrap();
        let project = Project::create(directory.path(), "template").unwrap();
        write_template_metadata(
            &project,
            "universe",
            "@preview/demo:1.0.0",
            "main.typ",
            true,
        )
        .unwrap();
        assert!(project_layout_locked(&project));
        let metadata = fs::read_to_string(project.root().join(".typsmthng/template.json")).unwrap();
        assert!(metadata.contains("\"layoutLocked\": true"));
        assert!(!metadata.contains("electrobun"));
    }
}
