use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::prelude::*;
use typsmthng_gtk::backend::{
    convert_latex_to_typst, export_project, export_projects, import_project, import_projects,
    ArchiveLimits, BackendError, CompileOptions, CompileOutput, DiagnosticSeverity, EntryKind,
    ExternalEventKind, ExternalWatcher, FileContent, GoogleFontCache, InlineNote, Project,
    StateStore, SvgPage, Theme as BackendTheme, TypstTool, UniverseClient, UniverseTemplate,
    UpdateClient, UpdateStatus, UserSettings, WindowState as BackendWindowState,
};

use super::home::{HomeCallbacks, HomeView, RecentProjectRow};
use super::model::{resolve_startup_path, SearchMode, Theme, UiSettings};
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
}

pub fn launch(options: LaunchOptions) -> glib::ExitCode {
    let application = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

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
    state_store: Option<StateStore>,
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
    compile_generation: Cell<u64>,
    compile_in_flight: Cell<bool>,
    pending_compile_source: RefCell<Option<String>>,
}

struct CompileFinished {
    generation: u64,
    main: String,
    result: Result<(CompileOutput<Vec<SvgPage>>, Vec<InlineNote>), String>,
}

impl AppController {
    fn build(application: &gtk::Application, options: LaunchOptions) -> Rc<Self> {
        let state_store = StateStore::discover().ok();
        let backend_settings = state_store
            .as_ref()
            .and_then(|store| store.load_settings().ok())
            .unwrap_or_default();
        let window_state = state_store
            .as_ref()
            .and_then(|store| store.load_metadata().ok())
            .map(|metadata| metadata.window_state)
            .unwrap_or_default();
        let settings = settings_from_backend(&backend_settings);
        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("typsmthng")
            .default_width(window_state.width)
            .default_height(window_state.height)
            .build();
        window.set_size_request(760, 520);
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(120);
        window.set_child(Some(&stack));

        let controller = Rc::new(Self {
            self_weak: RefCell::new(Weak::new()),
            application: application.clone(),
            window,
            stack,
            state_store,
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
            compile_generation: Cell::new(0),
            compile_in_flight: Cell::new(false),
            pending_compile_source: RefCell::new(None),
        });
        controller.self_weak.replace(Rc::downgrade(&controller));

        controller.install_views();
        controller.install_actions();
        controller.refresh_recents();
        controller
            .workspace
            .borrow()
            .as_ref()
            .unwrap()
            .apply_settings(settings.clone());
        if let Some(gtk_settings) = gtk::Settings::default() {
            gtk_settings.set_gtk_application_prefer_dark_theme(settings.theme == Theme::Dark);
        }
        if settings.translucent {
            controller.window.add_css_class("translucent");
        }
        controller.window.present();
        if window_state.maximized {
            controller.window.maximize();
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

        if !options.smoke_test && !options.presentation_smoke_test {
            let weak = Rc::downgrade(&controller);
            glib::timeout_add_local_once(Duration::from_secs(2), move || {
                if let Some(this) = weak.upgrade() {
                    this.check_update_with_feedback(false);
                }
            });
        }
        if options.presentation_smoke_test {
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
                if presentation.page_count() > 0 {
                    presentation.start_presenter();
                    println!(
                        "TYPESMTHNG_PRESENTATION_READY {{\"gtk\":true,\"pages\":{},\"windows\":{}}}",
                        presentation.page_count(),
                        presentation.window_count()
                    );
                    let _ = std::io::stdout().flush();
                    let application = application.clone();
                    glib::timeout_add_local_once(Duration::from_millis(900), move || {
                        application.quit()
                    });
                    glib::ControlFlow::Break
                } else if attempt >= 200 {
                    eprintln!("TYPESMTHNG_PRESENTATION_FAILED {{\"reason\":\"compile-timeout\"}}");
                    std::process::exit(1);
                } else {
                    glib::ControlFlow::Continue
                }
            });
        } else if options.smoke_test {
            let application = application.clone();
            glib::idle_add_local_once(move || {
                println!("TYPESMTHNG_SMOKE_READY {{\"gtk\":true,\"window\":true}}");
                let _ = std::io::stdout().flush();
                glib::timeout_add_local_once(Duration::from_millis(900), move || {
                    application.quit()
                });
            });
        }
        CONTROLLERS.with(|controllers| controllers.borrow_mut().push(controller.clone()));
        controller
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
            import_project: callback0(&weak, Self::import_project_archive),
            export_all: callback0(&weak, Self::export_all_projects),
            create_from_template: callback0(&weak, Self::create_from_template),
            create_workspace: callback0(&weak, Self::create_workspace),
            select_workspace: callback1(&weak, Self::select_workspace),
            manage_workspace: callback0(&weak, Self::manage_workspace),
            assign_workspace: callback1(&weak, Self::assign_project_workspace),
        });
        self.stack.add_named(&home.root, Some("home"));
        self.home.replace(Some(home));

        let workspace = WorkspaceView::new(
            &self.window,
            WorkspaceCallbacks {
                go_home: callback0(&weak, Self::close_to_home),
                open_project: callback0(&weak, Self::choose_open_project),
                save: callback1_result(&weak, Self::save_current),
                force_save: callback1_result(&weak, Self::force_save_current),
                select_file: callback1(&weak, Self::select_file),
                create_file: callback0(&weak, Self::prompt_create_file),
                create_folder: callback0(&weak, Self::prompt_create_folder),
                import_files: callback0(&weak, Self::import_files),
                drop_files: callback1(&weak, Self::import_paths),
                move_path: callback1(&weak, Self::move_path),
                toggle_hidden: callback0(&weak, Self::toggle_hidden),
                rename_path: callback1(&weak, Self::rename_path),
                duplicate_path: callback1(&weak, Self::duplicate_path),
                trash_path: callback1(&weak, Self::trash_path),
                reveal_path: callback1(&weak, Self::reveal_path),
                open_external: callback1(&weak, Self::open_external),
                check_update: callback0(&weak, Self::check_update),
                export_pdf: callback0(&weak, Self::export_pdf),
                export_project: callback0(&weak, Self::export_current_project),
                present_single: callback0(&weak, Self::present_single),
                present_dual: callback0(&weak, Self::present_dual),
                refresh_compile: callback1(&weak, Self::compile),
                search: {
                    let weak = weak.clone();
                    Rc::new(move |mode, query| {
                        weak.upgrade()
                            .map(|this| this.search(mode, &query))
                            .unwrap_or_default()
                    })
                },
                settings_changed: callback1(&weak, Self::settings_changed),
            },
        );
        self.stack.add_named(&workspace.root, Some("workspace"));
        self.workspace.replace(Some(workspace));
        let save_note: Rc<dyn Fn(usize, String)> = {
            let weak = weak.clone();
            Rc::new(move |slide, text| {
                if let Some(this) = weak.upgrade() {
                    this.save_presentation_note(slide, &text);
                }
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
        self.add_action("settings", &["<Primary>comma"], Self::show_settings);
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
        self.add_action("duplicate-line", &["<Primary>d"], |this| {
            if let Some(workspace) = this.workspace.borrow().as_ref() {
                workspace.duplicate_lines();
            }
        });
        self.add_action("cycle-theme", &["<Primary>j"], Self::cycle_theme);
        self.add_action("quit", &["<Primary>q"], |this| this.application.quit());
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
                    if this
                        .workspace
                        .borrow()
                        .as_ref()
                        .is_some_and(|workspace| !workspace.save_before_navigation())
                    {
                        return glib::Propagation::Stop;
                    }
                    if let Some(store) = &this.state_store {
                        let _ = store.save_window_state(BackendWindowState {
                            width: this.window.width(),
                            height: this.window.height(),
                            x: None,
                            y: None,
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
            presentation.end();
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
                    let _ = store.upsert_recent(
                        project.root(),
                        project.name(),
                        count,
                        main.clone(),
                        true,
                    );
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
        let chooser = gtk::FileChooserNative::builder()
            .title("Open a Typst project")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::SelectFolder)
            .accept_label("Open")
            .cancel_label("Cancel")
            .build();
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(path)) =
                        (weak.upgrade(), chooser.file().and_then(|file| file.path()))
                    {
                        this.open_project(&path, None);
                    }
                }
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn choose_create_project(&self) {
        let chooser = gtk::FileChooserNative::builder()
            .title("Choose a parent folder")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::SelectFolder)
            .accept_label("Choose")
            .cancel_label("Cancel")
            .build();
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(parent)) =
                        (weak.upgrade(), chooser.file().and_then(|file| file.path()))
                    {
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
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn select_file(&self, path: String) {
        if let Some(command) = path.strip_prefix(":command:") {
            match command {
                "compile" => {
                    if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.request_compile();
                    }
                }
                "export" => self.export_pdf(),
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
                self.current_file.replace(Some(path.clone()));
                if let Some(store) = &self.state_store {
                    let _ = store.persist_last_file(project.root(), Some(path.clone()));
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
        let main = match project.resolve_main_file(self.current_file.borrow().as_deref()) {
            Ok(main) => main,
            Err(error) => {
                workspace.set_compile_status(&format!("Compile error: {error}"));
                return;
            }
        };
        if let Some(current) = self.current_file.borrow().as_deref() {
            if !self.write_current_checked(&project, current, &source, "Could not save file") {
                return;
            }
        }
        let generation = self.compile_generation.get().wrapping_add(1);
        self.compile_generation.set(generation);
        if self.compile_in_flight.get() {
            self.pending_compile_source.replace(Some(source));
            workspace.set_compile_status("Compilation queued…");
            return;
        }
        self.compile_in_flight.set(true);
        workspace.set_compiling();
        let options = self.compile_options();
        let google_fonts = self.settings.borrow().google_fonts;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn({
            let main = main.clone();
            move || {
                let mut options = options;
                if google_fonts {
                    if let Ok(Some(path)) =
                        GoogleFontCache::default().prepare_project(&project, &source)
                    {
                        options.font_paths.push(path);
                    }
                }
                let result = TypstTool::detect()
                    .and_then(|tool| {
                        let output = tool.compile_svg_with_options(&project, &main, &options)?;
                        let notes = if output.artifact.is_some() {
                            tool.query_notes(&project, &main, &options)
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        Ok((output, notes))
                    })
                    .map_err(|error| error.to_string());
                let _ = sender.send(CompileFinished {
                    generation,
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
                        if let Some(source) = this.pending_compile_source.borrow_mut().take() {
                            this.compile(source);
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn compile_options(&self) -> CompileOptions {
        let settings = self.settings.borrow();
        let page_preamble = match settings.page_size.as_str() {
            "a3" | "a4" | "a5" | "a6" | "us-letter" | "us-legal" | "iso-b5" => {
                Some(format!("#set page(paper: \"{}\")", settings.page_size))
            }
            "presentation-16-9" => Some("#set page(width: 13.333in, height: 7.5in)".to_string()),
            _ => None,
        };
        CompileOptions {
            ignore_system_fonts: !settings.system_fonts,
            page_preamble,
            ..CompileOptions::default()
        }
    }

    fn apply_compile_result(&self, finished: CompileFinished) {
        if finished.generation != self.compile_generation.get() {
            return;
        }
        let Some(workspace) = self.workspace.borrow().as_ref().cloned() else {
            return;
        };
        let (output, inline_notes) = match finished.result {
            Ok(output) => output,
            Err(error) => {
                workspace.set_compile_status(&format!("Compile error: {error}"));
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
        let Some(pages) = output.artifact else {
            let detail = if output.stderr.trim().is_empty() {
                "Typst did not produce pages"
            } else {
                output.stderr.trim()
            };
            workspace.set_compile_status(&format!("Compile error: {detail}"));
            return;
        };
        let cache = match tempfile::tempdir() {
            Ok(cache) => cache,
            Err(error) => {
                workspace.set_compile_status(&format!("Preview cache error: {error}"));
                return;
            }
        };
        let split_notes = match self.settings.borrow().presentation_notes_layout.as_str() {
            "right-half" => true,
            "whole" => false,
            _ => {
                !pages.is_empty()
                    && pages.iter().all(|page| {
                        page.width_points
                            .zip(page.height_points)
                            .is_some_and(|(width, height)| height > 0.0 && width / height >= 2.6)
                    })
            }
        };
        let mut paths = Vec::with_capacity(pages.len());
        let mut rendered_notes = Vec::with_capacity(pages.len());
        for page in pages {
            let path = cache.path().join(format!("page-{:04}.svg", page.page));
            let (slide_svg, note_svg) = if split_notes {
                let width = page.width_points.unwrap_or_default();
                let height = page.height_points.unwrap_or_default();
                (
                    crop_svg(&page.svg, 0.0, width / 2.0, height),
                    Some(crop_svg(&page.svg, width / 2.0, width / 2.0, height)),
                )
            } else {
                (page.svg, None)
            };
            if let Err(error) = std::fs::write(&path, slide_svg) {
                workspace.set_compile_status(&format!("Preview cache error: {error}"));
                return;
            }
            paths.push(path);
            let note_path = note_svg.and_then(|svg| {
                let path = cache.path().join(format!("notes-{:04}.svg", page.page));
                std::fs::write(&path, svg).ok().map(|()| path)
            });
            rendered_notes.push(note_path);
        }
        let source_line_count = self
            .project
            .borrow()
            .as_ref()
            .and_then(|project| project.read_file(&finished.main).ok())
            .and_then(|file| match file.content {
                FileContent::Text(source) => Some(source.lines().count().max(1)),
                FileContent::Binary(_) => None,
            })
            .unwrap_or(1);
        self.render_cache.replace(Some(cache));
        self.compiled_main.replace(Some(finished.main.clone()));
        workspace.set_compiled_preview(&paths, &finished.main, source_line_count);
        workspace.set_compile_status(&format!("Compiled in {} ms", output.elapsed.as_millis()));
        if let Some(presentation) = self.presentation.borrow().as_ref() {
            let (inline_notes, sidecar_notes) =
                self.load_presentation_notes(paths.len(), &inline_notes);
            presentation.set_deck(paths, inline_notes, sidecar_notes, rendered_notes);
        }
    }

    fn load_presentation_notes(
        &self,
        page_count: usize,
        inline: &[InlineNote],
    ) -> (Vec<String>, Vec<String>) {
        let mut inline_notes = vec![String::new(); page_count];
        for note in inline {
            if let Some(target) = note
                .page
                .checked_sub(1)
                .and_then(|index| inline_notes.get_mut(index))
            {
                if !target.is_empty() {
                    target.push_str("\n\n");
                }
                target.push_str(&note.text);
            }
        }
        let (Some(project), Some(current)) = (
            self.project.borrow().as_ref().cloned(),
            self.compiled_main.borrow().clone(),
        ) else {
            return (inline_notes, vec![String::new(); page_count]);
        };
        let mut sidecar_notes = vec![String::new(); page_count];
        let sidecar = format!("{}.notes.md", current.trim_end_matches(".typ"));
        if let Ok(file) = project.read_file(&sidecar) {
            if let FileContent::Text(text) = file.content {
                let mut slide = None;
                for line in text.lines() {
                    if let Some(number) = line
                        .strip_prefix("## Slide ")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                    {
                        slide = number.checked_sub(1).filter(|index| *index < page_count);
                    } else if let Some(index) = slide {
                        if !sidecar_notes[index].is_empty() {
                            sidecar_notes[index].push('\n');
                        }
                        sidecar_notes[index].push_str(line);
                    }
                }
            }
        }
        if let Ok(file) = project.read_file(&current) {
            if let FileContent::Text(text) = file.content {
                for (index, line) in text
                    .lines()
                    .filter_map(|line| line.trim().strip_prefix("// note:").map(str::trim))
                    .enumerate()
                {
                    if index < page_count && inline_notes[index].is_empty() {
                        inline_notes[index] = line.to_string();
                    }
                }
            }
        }
        (inline_notes, sidecar_notes)
    }

    fn save_presentation_note(&self, slide: usize, text: &str) {
        let (Some(project), Some(current)) = (
            self.project.borrow().as_ref().cloned(),
            self.compiled_main.borrow().clone(),
        ) else {
            return;
        };
        let sidecar = format!("{}.notes.md", current.trim_end_matches(".typ"));
        let existing = project
            .read_file(&sidecar)
            .ok()
            .and_then(|file| match file.content {
                FileContent::Text(text) => Some(text),
                FileContent::Binary(_) => None,
            })
            .unwrap_or_default();
        let mut sections = parse_note_sections(&existing);
        if text.trim().is_empty() {
            sections.remove(&(slide + 1));
        } else {
            sections.insert(slide + 1, text.trim().to_string());
        }
        if sections.is_empty() && existing.is_empty() {
            return;
        }
        let markdown = sections
            .into_iter()
            .map(|(number, note)| format!("## Slide {number}\n\n{note}\n"))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = project.write_text_atomic(&sidecar, &markdown);
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

    fn export_pdf(&self) {
        let (Some(project), Some(current)) = (
            self.project.borrow().clone(),
            self.current_file.borrow().clone(),
        ) else {
            self.show_error("Export PDF", "Open a Typst project first.");
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
                self.show_error("Export PDF failed", &error.to_string());
                return;
            }
        };
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.set_compile_status("Rendering PDF…");
        }
        let options = self.compile_options();
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
                .and_then(|tool| tool.compile_pdf_with_options(&project, &main, &options));
            let _ = sender.send(result);
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            match receiver.try_recv() {
                Ok(result) => {
                    if let Some(this) = weak.upgrade() {
                        match result {
                            Ok(output) => {
                                if let Some(pdf) = output.artifact {
                                    this.choose_pdf_destination(&project_name, pdf);
                                    if let Some(workspace) = this.workspace.borrow().as_ref() {
                                        workspace.set_compile_status(&format!(
                                            "PDF rendered in {} ms",
                                            output.elapsed.as_millis()
                                        ));
                                    }
                                } else {
                                    this.show_error("Export PDF failed", output.stderr.trim());
                                }
                            }
                            Err(error) => this.show_error("Export PDF failed", &error.to_string()),
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
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
        let chooser = gtk::FileChooserNative::builder()
            .title("Export project ZIP")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::Save)
            .accept_label("Export")
            .cancel_label("Cancel")
            .build();
        chooser.set_current_name(&format!("{}.zip", project.name()));
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(destination)) =
                        (weak.upgrade(), chooser.file().and_then(|file| file.path()))
                    {
                        if let Err(error) = export_project(&project, destination) {
                            this.show_error("Could not export project", &error.to_string());
                        }
                    }
                }
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn choose_pdf_destination(&self, project_name: &str, pdf: Vec<u8>) {
        let chooser = gtk::FileChooserNative::builder()
            .title("Export PDF")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::Save)
            .accept_label("Export")
            .cancel_label("Cancel")
            .build();
        chooser.set_current_name(&format!("{project_name}.pdf"));
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(path) = chooser.file().and_then(|file| file.path()) {
                        if let Err(error) = std::fs::write(&path, &pdf) {
                            if let Some(this) = weak.upgrade() {
                                this.show_error("Could not save PDF", &error.to_string());
                            }
                        }
                    }
                }
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn present_single(&self) {
        if let Some(presentation) = self.presentation.borrow().as_ref() {
            presentation.start_single();
        }
    }

    fn present_dual(&self) {
        if let Some(presentation) = self.presentation.borrow().as_ref() {
            presentation.start_presenter();
        }
    }

    fn search(&self, mode: SearchMode, query: &str) -> Vec<SearchResultRow> {
        if mode == SearchMode::Commands {
            return [
                ("Compile document", "Ctrl+Enter", "compile"),
                ("Export PDF", "Ctrl+Shift+E", "export"),
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
        }
        let Some(project) = self.project.borrow().clone() else {
            return Vec::new();
        };
        match mode {
            SearchMode::Files => project
                .search_paths(query, 80, *self.hidden_files.borrow())
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
                .search_text(query, 80, *self.hidden_files.borrow())
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
        let chooser = gtk::FileChooserNative::builder()
            .title("Import files into vault")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::Open)
            .accept_label("Import")
            .cancel_label("Cancel")
            .build();
        chooser.set_select_multiple(true);
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(this) = weak.upgrade() {
                        let files = chooser.files();
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
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn import_paths(&self, paths: Vec<PathBuf>) {
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        let mut conversion_warnings = Vec::new();
        for path in paths {
            let Some(name) = path.file_name() else {
                continue;
            };
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if extension == "tex" || extension == "ltx" {
                match std::fs::read_to_string(&path) {
                    Ok(source) => {
                        let converted = convert_latex_to_typst(&source);
                        conversion_warnings.extend(
                            converted.warnings.iter().map(|warning| {
                                format!("{}: {}", warning.construct, warning.message)
                            }),
                        );
                        let source_target = unique_project_path(&project, Path::new(name));
                        let target =
                            unique_project_path(&project, &source_target.with_extension("typ"));
                        let result = project
                            .create_binary_file(&source_target, source.as_bytes())
                            .and_then(|_| project.write_text_atomic(&target, &converted.typst));
                        if let Err(error) = result {
                            self.show_error("Could not import LaTeX", &error.to_string());
                        }
                    }
                    Err(error) => self.show_error("Could not import LaTeX", &error.to_string()),
                }
            } else if path.is_file() {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let target = unique_project_path(&project, Path::new(name));
                        if let Err(error) = project.create_binary_file(&target, &bytes) {
                            self.show_error("Could not import file", &error.to_string());
                        }
                    }
                    Err(error) => self.show_error("Could not import file", &error.to_string()),
                }
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
    }

    fn toggle_hidden(&self) {
        let value = !*self.hidden_files.borrow();
        self.hidden_files.replace(value);
        if let (Some(store), Some(project)) = (&self.state_store, self.project.borrow().as_ref()) {
            let _ = store.set_hidden_files_visible(project.root(), value);
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
        let folder = if target.is_dir() {
            target
        } else {
            target.parent().unwrap_or(project.root()).to_path_buf()
        };
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

    fn check_update(&self) {
        self.check_update_with_feedback(true);
    }

    fn check_update_with_feedback(&self, announce_current: bool) {
        let progress = announce_current.then(|| {
            let dialog = gtk::MessageDialog::builder()
                .transient_for(&self.window)
                .modal(true)
                .message_type(gtk::MessageType::Info)
                .buttons(gtk::ButtonsType::None)
                .text("Checking for updates…")
                .secondary_text("Contacting the stable GitHub release channel.")
                .build();
            dialog.present();
            dialog
        });
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(UpdateClient::default().check(env!("CARGO_PKG_VERSION")));
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            match receiver.try_recv() {
                Ok(result) => {
                    if let Some(progress) = &progress {
                        progress.close();
                    }
                    if let Some(this) = weak.upgrade() {
                        this.show_update_result(result, announce_current);
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    fn show_update_result(
        &self,
        result: typsmthng_gtk::backend::Result<UpdateStatus>,
        announce_current: bool,
    ) {
        match result {
            Ok(UpdateStatus::UpToDate { current }) if announce_current => {
                let dialog = gtk::MessageDialog::builder()
                    .transient_for(&self.window)
                    .modal(true)
                    .message_type(gtk::MessageType::Info)
                    .buttons(gtk::ButtonsType::Close)
                    .text(format!("typsmthng {current} is current"))
                    .secondary_text("No newer stable release is available.")
                    .build();
                dialog.connect_response(|dialog, _| dialog.close());
                dialog.present();
            }
            Ok(UpdateStatus::UpToDate { .. }) => {}
            Ok(UpdateStatus::Available {
                latest,
                release_url,
                asset,
                ..
            }) => {
                let dialog = gtk::MessageDialog::builder()
                    .transient_for(&self.window)
                    .modal(true)
                    .message_type(gtk::MessageType::Info)
                    .buttons(gtk::ButtonsType::None)
                    .text(format!("typsmthng {latest} is available"))
                    .secondary_text("Review the release or download the installer for this system.")
                    .build();
                dialog.add_button("Later", gtk::ResponseType::Cancel);
                dialog.add_button("Release notes", gtk::ResponseType::Other(1));
                if asset.is_some() {
                    dialog.add_button("Download…", gtk::ResponseType::Accept);
                }
                let weak = self.weak();
                dialog.connect_response(move |dialog, response| {
                    dialog.close();
                    if let Some(this) = weak.upgrade() {
                        if response == gtk::ResponseType::Other(1) {
                            let _ = gio::AppInfo::launch_default_for_uri(
                                &release_url,
                                None::<&gio::AppLaunchContext>,
                            );
                        } else if response == gtk::ResponseType::Accept {
                            if let Some(asset) = asset.clone() {
                                this.choose_update_destination(asset);
                            }
                        }
                    }
                });
                dialog.present();
            }
            Err(error) if announce_current => {
                self.show_error("Update check failed", &error.to_string())
            }
            Err(_) => {}
        }
    }

    fn choose_update_destination(&self, asset: typsmthng_gtk::backend::ReleaseAsset) {
        let chooser = gtk::FileChooserNative::builder()
            .title("Download typsmthng update")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::Save)
            .accept_label("Download")
            .cancel_label("Cancel")
            .build();
        chooser.set_current_name(&asset.name);
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(path)) =
                        (weak.upgrade(), chooser.file().and_then(|file| file.path()))
                    {
                        this.download_update(asset.clone(), path);
                    }
                }
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn download_update(&self, asset: typsmthng_gtk::backend::ReleaseAsset, path: PathBuf) {
        let progress = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .message_type(gtk::MessageType::Info)
            .buttons(gtk::ButtonsType::None)
            .text("Downloading update…")
            .secondary_text(&asset.name)
            .build();
        progress.present();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(UpdateClient::default().download(&asset, path));
        });
        let weak = self.weak();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            match receiver.try_recv() {
                Ok(result) => {
                    progress.close();
                    if let Some(this) = weak.upgrade() {
                        match result {
                            Ok(path) => {
                                let dialog = gtk::MessageDialog::builder()
                                    .transient_for(&this.window)
                                    .modal(true)
                                    .message_type(gtk::MessageType::Info)
                                    .buttons(gtk::ButtonsType::None)
                                    .text("Update downloaded")
                                    .secondary_text(path.to_string_lossy())
                                    .build();
                                dialog.add_button("Close", gtk::ResponseType::Close);
                                dialog.add_button("Open installer", gtk::ResponseType::Accept);
                                dialog.connect_response(move |dialog, response| {
                                    dialog.close();
                                    if response == gtk::ResponseType::Accept {
                                        let uri = gio::File::for_path(&path).uri();
                                        let _ = gio::AppInfo::launch_default_for_uri(
                                            &uri,
                                            None::<&gio::AppLaunchContext>,
                                        );
                                    }
                                });
                                dialog.present();
                            }
                            Err(error) => {
                                this.show_error("Update download failed", &error.to_string())
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

    fn refresh_project_files(&self) {
        let Some(project) = self.project.borrow().clone() else {
            return;
        };
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            if !workspace.save_before_navigation() {
                return;
            }
        }
        // Drop the RefCell guard before `open_project` selects the file and
        // mutates `current_file`. Watcher-driven refreshes otherwise panic.
        let current_file = self.current_file.borrow().clone();
        self.open_project(project.root(), current_file);
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
                if dirty {
                    if let Some(workspace) = self.workspace.borrow().as_ref() {
                        workspace.show_conflict(&event.path);
                    }
                } else if event.kind == ExternalEventKind::Changed {
                    self.select_file(event.path);
                } else if event.kind == ExternalEventKind::Removed {
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
        if let Some(gtk_settings) = gtk::Settings::default() {
            gtk_settings.set_gtk_application_prefer_dark_theme(settings.theme == Theme::Dark);
        }
        if settings.translucent {
            self.window.add_css_class("translucent");
        } else {
            self.window.remove_css_class("translucent");
        }
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.apply_settings(settings.clone());
        }
        if let Some(store) = &self.state_store {
            let _ = store.save_settings(&settings_to_backend(&settings));
        }
        if self.project.borrow().is_some() {
            if let Some(workspace) = self.workspace.borrow().as_ref() {
                if workspace.editor.is_editable() {
                    self.compile(workspace.source_text());
                }
            }
        }
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

    fn show_settings(&self) {
        if let Some(workspace) = self.workspace.borrow().as_ref() {
            workspace.present_settings();
        }
    }

    fn toggle_favorite(&self, path: String) {
        if let Some(store) = &self.state_store {
            let _ = store.toggle_favorite(Path::new(&path));
        }
        self.refresh_recents();
    }

    fn remove_recent(&self, path: String) {
        if let Some(store) = &self.state_store {
            let _ = store.remove_recent(Path::new(&path));
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
                let _ = store.select_workspace(selected);
                self.refresh_recents();
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
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .buttons(gtk::ButtonsType::None)
            .text(&workspace.name)
            .secondary_text("Rename this workspace or delete it. Projects remain in All projects.")
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Rename…", gtk::ResponseType::Other(1));
        dialog.add_button("Delete", gtk::ResponseType::Reject);
        let weak = self.weak();
        dialog.connect_response(move |dialog, response| {
            dialog.close();
            if let Some(this) = weak.upgrade() {
                if response == gtk::ResponseType::Other(1) {
                    let selected = selected.clone();
                    this.prompt_name("Rename workspace", "Workspace name", move |this, name| {
                        if let Some(store) = &this.state_store {
                            let _ = store.rename_workspace(&selected, name);
                        }
                        this.refresh_recents();
                    });
                } else if response == gtk::ResponseType::Reject {
                    if let Some(store) = &this.state_store {
                        let _ = store.delete_workspace(&selected);
                    }
                    this.refresh_recents();
                }
            }
        });
        dialog.present();
    }

    fn assign_project_workspace(&self, path: String) {
        let Some(store) = &self.state_store else {
            return;
        };
        let Ok(metadata) = store.load_metadata() else {
            return;
        };
        let mut ids = vec![None];
        let mut labels = vec!["No workspace".to_string()];
        for workspace in metadata.home_workspaces {
            ids.push(Some(workspace.id));
            labels.push(workspace.name);
        }
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
        let picker = gtk::DropDown::from_strings(&label_refs);
        let dialog = gtk::Dialog::builder()
            .title("Move project to workspace")
            .transient_for(&self.window)
            .modal(true)
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Move", gtk::ResponseType::Accept);
        picker.set_margin_top(18);
        picker.set_margin_bottom(18);
        picker.set_margin_start(18);
        picker.set_margin_end(18);
        dialog.content_area().append(&picker);
        let weak = self.weak();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                if let Some(this) = weak.upgrade() {
                    if let Some(store) = &this.state_store {
                        let selected = ids.get(picker.selected() as usize).cloned().flatten();
                        let _ = store.assign_workspace(Path::new(&path), selected);
                    }
                    this.refresh_recents();
                }
            }
            dialog.close();
        });
        dialog.present();
    }

    fn import_project_archive(&self) {
        let archive = gtk::FileChooserNative::builder()
            .title("Import project archive")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::Open)
            .accept_label("Choose archive")
            .cancel_label("Cancel")
            .build();
        archive.connect_response({
            let weak = self.weak();
            move |archive, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(archive_path)) =
                        (weak.upgrade(), archive.file().and_then(|file| file.path()))
                    {
                        let parent = gtk::FileChooserNative::builder()
                            .title("Choose destination folder")
                            .transient_for(&this.window)
                            .action(gtk::FileChooserAction::SelectFolder)
                            .accept_label("Import here")
                            .cancel_label("Cancel")
                            .build();
                        parent.connect_response({
                            let weak = this.weak();
                            move |parent, response| {
                                if response == gtk::ResponseType::Accept {
                                    if let (Some(this), Some(folder)) =
                                        (weak.upgrade(), parent.file().and_then(|file| file.path()))
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
                                parent.destroy();
                            }
                        });
                        parent.show();
                    }
                }
                archive.destroy();
            }
        });
        archive.show();
    }

    fn export_all_projects(&self) {
        let chooser = gtk::FileChooserNative::builder()
            .title("Export all recent projects")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::Save)
            .accept_label("Export")
            .cancel_label("Cancel")
            .build();
        chooser.set_current_name("typsmthng-all-projects.zip");
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(destination)) =
                        (weak.upgrade(), chooser.file().and_then(|file| file.path()))
                    {
                        let mut projects = Vec::new();
                        let mut failures = Vec::new();
                        if let Some(store) = &this.state_store {
                            for recent in store.load_metadata().unwrap_or_default().recent_projects
                            {
                                match Project::open(&recent.root_path) {
                                    Ok(project) => projects.push(project),
                                    Err(error) => {
                                        failures.push(format!("{}: {error}", recent.name))
                                    }
                                }
                            }
                        }
                        if failures.is_empty() {
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
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn create_from_template(&self) {
        let dialog = gtk::Dialog::builder()
            .title("New from template")
            .transient_for(&self.window)
            .modal(true)
            .default_width(720)
            .default_height(560)
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Use starter", gtk::ResponseType::Accept);
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
        scroll.set_child(Some(&results));
        content.append(&scroll);
        dialog.content_area().append(&content);

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
        dialog.connect_response({
            let weak = self.weak();
            move |dialog, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(this) = weak.upgrade() {
                        let (source, auxiliary) = match choices.selected() {
                            1 => (
                                "#set page(paper: \"a4\")\n#set text(size: 11pt)\n\n= Article title\n\nStart writing here.\n",
                                Vec::new(),
                            ),
                            2 => (
                                "#set page(width: 13.333in, height: 7.5in, margin: 0.7in)\n#set text(size: 28pt)\n\n= Presentation\n\n#pagebreak()\n\n= Next slide\n",
                                Vec::new(),
                            ),
                            3 => (
                                "#set page(paper: \"a4\", margin: 25mm)\n#set text(size: 11pt)\n\n= Report\n\n== Summary\n",
                                Vec::new(),
                            ),
                            _ => (
                                "= Research Starter\n\n== Abstract\nSummarize your contribution, methods, and key findings.\n\n== Introduction\nDescribe the context, problem, and why the work matters.\n\n== Method\nExplain your approach, assumptions, and data.\n\n== Results\nReport your most relevant outcomes.\n\n== Discussion\nInterpret results, limits, and future work.\n\n== References\n#bibliography(\"refs.bib\")\n",
                                vec![(
                                    "refs.bib".to_string(),
                                    "@article{sample2026,\n  title = {Replace with your first citation},\n  author = {Doe, Jane},\n  journal = {Journal Name},\n  year = {2026}\n}\n"
                                        .to_string(),
                                )],
                            ),
                        };
                        this.choose_builtin_template(source.to_string(), auxiliary);
                    }
                }
                dialog.close();
            }
        });
        dialog.present();
    }

    fn choose_builtin_template(&self, source: String, auxiliary: Vec<(String, String)>) {
        let chooser = gtk::FileChooserNative::builder()
            .title("Choose a parent folder")
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::SelectFolder)
            .accept_label("Choose")
            .cancel_label("Cancel")
            .build();
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(parent)) =
                        (weak.upgrade(), chooser.file().and_then(|file| file.path()))
                    {
                        let source = source.clone();
                        let auxiliary = auxiliary.clone();
                        this.prompt_name("Template project", "Project name", move |this, name| {
                            match Project::create(&parent, &name) {
                                Ok(project) => {
                                    let written = project
                                        .write_text_atomic("main.typ", &source)
                                        .and_then(|_| {
                                            for (path, contents) in &auxiliary {
                                                project.write_text_atomic(path, contents)?;
                                            }
                                            Ok(())
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
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn choose_universe_template(&self, template: UniverseTemplate) {
        let chooser = gtk::FileChooserNative::builder()
            .title(format!("Create from {}", template.spec))
            .transient_for(&self.window)
            .action(gtk::FileChooserAction::SelectFolder)
            .accept_label("Choose parent")
            .cancel_label("Cancel")
            .build();
        chooser.connect_response({
            let weak = self.weak();
            move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let (Some(this), Some(parent)) =
                        (weak.upgrade(), chooser.file().and_then(|file| file.path()))
                    {
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
                chooser.destroy();
            }
        });
        chooser.show();
    }

    fn initialize_universe_template(&self, spec: String, destination: PathBuf) {
        let progress = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .buttons(gtk::ButtonsType::None)
            .text("Initializing Typst template…")
            .secondary_text(&spec)
            .build();
        progress.present();
        let (sender, receiver) = std::sync::mpsc::channel();
        let result_path = destination.clone();
        std::thread::spawn(move || {
            let result = TypstTool::detect().and_then(|tool| {
                tool.init_template(&spec, &destination, &CompileOptions::default())
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
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .message_type(gtk::MessageType::Info)
            .buttons(gtk::ButtonsType::Close)
            .text("typsmthng quick guide")
            .secondary_text("Open an ordinary folder containing Typst files. Edit with GtkSourceView, inspect the live page stack, search with Ctrl+K, compile with Ctrl+Enter, and present with F5. Presenter view uses a second GTK window and supports notes, timer, grid, blackout, laser, pen, highlighter, and eraser controls.")
            .build();
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
    }

    fn prompt_name(
        &self,
        title: &str,
        placeholder: &str,
        complete: impl Fn(&Self, String) + 'static,
    ) {
        let dialog = gtk::Dialog::builder()
            .title(title)
            .transient_for(&self.window)
            .modal(true)
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Create", gtk::ResponseType::Accept);
        dialog.set_default_response(gtk::ResponseType::Accept);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(placeholder));
        entry.set_margin_top(18);
        entry.set_margin_bottom(18);
        entry.set_margin_start(18);
        entry.set_margin_end(18);
        dialog.content_area().append(&entry);
        let weak = self.weak();
        let response_entry = entry.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                let value = response_entry.text().trim().to_owned();
                if !value.is_empty() {
                    if let Some(this) = weak.upgrade() {
                        complete(&this, value);
                    }
                }
            }
            dialog.close();
        });
        dialog.present();
        entry.grab_focus();
    }

    fn refresh_recents(&self) {
        let metadata = self
            .state_store
            .as_ref()
            .and_then(|store| store.load_metadata().ok())
            .unwrap_or_default();
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
                    .map(|count| format!("{count} files"))
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
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .message_type(gtk::MessageType::Error)
            .buttons(gtk::ButtonsType::Close)
            .text(title)
            .secondary_text(detail)
            .build();
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
    }

    fn show_notice(&self, title: &str, detail: &str) {
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .message_type(gtk::MessageType::Warning)
            .buttons(gtk::ButtonsType::Close)
            .text(title)
            .secondary_text(detail)
            .build();
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
    }

    fn weak(&self) -> Weak<Self> {
        self.self_weak.borrow().clone()
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
        compile_delay_ms: settings.compile_delay_ms as u32,
        line_wrapping: settings.line_wrapping,
        line_numbers: settings.line_numbers,
        vim_mode: settings.vim_mode,
        page_size: settings.page_size.clone(),
        presentation_notes_layout: settings.presentation_notes_layout.clone(),
        presentation_notes_font_size: settings.presentation_notes_font_size.clamp(12, 34),
        system_fonts: settings.system_fonts_enabled,
        google_fonts: settings.google_fonts_enabled,
        translucent: settings.translucent,
    }
}

fn settings_to_backend(settings: &UiSettings) -> UserSettings {
    UserSettings {
        font_size: settings.font_size as f64,
        auto_compile: settings.auto_compile,
        compile_delay_ms: settings.compile_delay_ms as u64,
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
        ..UserSettings::default()
    }
}

fn parse_note_sections(markdown: &str) -> std::collections::BTreeMap<usize, String> {
    let mut sections = std::collections::BTreeMap::new();
    let mut slide = None;
    let mut lines = Vec::new();
    let flush = |slide: Option<usize>,
                 lines: &mut Vec<&str>,
                 sections: &mut std::collections::BTreeMap<usize, String>| {
        if let Some(slide) = slide {
            let note = lines.join("\n").trim().to_string();
            if !note.is_empty() {
                sections.insert(slide, note);
            }
        }
        lines.clear();
    };
    for line in markdown.lines() {
        if let Some(number) = line
            .strip_prefix("## Slide ")
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            flush(slide, &mut lines, &mut sections);
            slide = Some(number);
        } else if slide.is_some() {
            lines.push(line);
        }
    }
    flush(slide, &mut lines, &mut sections);
    sections
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

fn crop_svg(svg: &str, x: f64, width: f64, height: f64) -> String {
    let view_box = regex::Regex::new(r#"\bviewBox="[^"]*""#).unwrap();
    let root_width = regex::Regex::new(r#"\bwidth="[0-9.]+(?:pt)?""#).unwrap();
    let svg = view_box
        .replacen(
            svg,
            1,
            format!("viewBox=\"{x:.3} 0 {width:.3} {height:.3}\""),
        )
        .into_owned();
    root_width
        .replacen(&svg, 1, format!("width=\"{width:.3}pt\""))
        .into_owned()
}

fn import_latex_file(
    source_path: &Path,
    parent: &Path,
    project_name: &str,
) -> typsmthng_gtk::backend::Result<Project> {
    let project = Project::create(parent, project_name)?;
    let source = std::fs::read_to_string(source_path).map_err(|error| {
        typsmthng_gtk::backend::BackendError::Process(format!(
            "could not read {}: {error}",
            source_path.display()
        ))
    })?;
    let converted = convert_latex_to_typst(&source);
    project.write_text_atomic("main.typ", &converted.typst)?;
    project.write_text_atomic("source.tex", &source)?;
    Ok(project)
}

fn convert_latex_project_if_needed(project: &Project) -> typsmthng_gtk::backend::Result<()> {
    let entries = project.entries(true)?;
    for source_path in entries.iter().filter_map(|entry| {
        let lower = entry.path.to_ascii_lowercase();
        (entry.kind == EntryKind::File && (lower.ends_with(".tex") || lower.ends_with(".ltx")))
            .then_some(entry.path.clone())
    }) {
        let source = match project.read_file(&source_path)?.content {
            FileContent::Text(source) => source,
            FileContent::Binary(_) => continue,
        };
        let converted = convert_latex_to_typst(&source);
        let target = Path::new(&source_path).with_extension("typ");
        project.write_text_atomic(&target, &converted.typst)?;
    }
    Ok(())
}

fn unique_project_path(project: &Project, desired: &Path) -> PathBuf {
    if project.read_file(desired).is_err() {
        return desired.to_path_buf();
    }
    let parent = desired.parent().unwrap_or_else(|| Path::new(""));
    let stem = desired
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imported".into());
    let extension = desired.extension().map(|value| value.to_string_lossy());
    for index in 2..10_000 {
        let mut name = format!("{stem}-{index}");
        if let Some(extension) = &extension {
            name.push('.');
            name.push_str(extension);
        }
        let candidate = parent.join(name);
        if project.read_file(&candidate).is_err() {
            return candidate;
        }
    }
    parent.join(format!("{stem}-imported"))
}
