use std::rc::Rc;

use gtk::prelude::*;

#[derive(Debug, Clone)]
pub struct RecentProjectRow {
    pub name: String,
    pub path: String,
    pub detail: String,
    pub favorite: bool,
}

#[derive(Clone)]
pub struct HomeCallbacks {
    pub open_folder: Rc<dyn Fn()>,
    pub create_project: Rc<dyn Fn()>,
    pub open_recent: Rc<dyn Fn(String)>,
    pub show_guide: Rc<dyn Fn()>,
    pub show_settings: Rc<dyn Fn()>,
    pub toggle_favorite: Rc<dyn Fn(String)>,
    pub remove_recent: Rc<dyn Fn(String)>,
    pub rename_recent: Rc<dyn Fn(String)>,
    pub import_project: Rc<dyn Fn()>,
    pub export_all: Rc<dyn Fn()>,
    pub export_selected: Rc<dyn Fn(Vec<String>)>,
    pub create_from_template: Rc<dyn Fn()>,
    pub create_workspace: Rc<dyn Fn()>,
    pub select_workspace: Rc<dyn Fn(String)>,
    pub manage_workspace: Rc<dyn Fn()>,
    pub assign_workspace: Rc<dyn Fn(String)>,
    pub assign_selected_workspace: Rc<dyn Fn(Vec<String>)>,
}

#[derive(Clone)]
pub struct HomeView {
    pub root: gtk::Box,
    recent_list: gtk::FlowBox,
    recent_paths: Rc<std::cell::RefCell<Vec<String>>>,
    workspace_picker: gtk::DropDown,
    workspace_model: gtk::StringList,
    workspace_ids: Rc<std::cell::RefCell<Vec<String>>>,
    callbacks: HomeCallbacks,
    updating_workspaces: Rc<std::cell::Cell<bool>>,
}

impl HomeView {
    pub fn new(callbacks: HomeCallbacks) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        root.add_css_class("home");
        let header = gtk::HeaderBar::new();
        header.add_css_class("home-titlebar");
        header.set_show_title_buttons(true);
        header.set_title_widget(Some(&gtk::Label::new(None)));
        root.append(&header);

        // One scrolling surface keeps every action reachable on short windows.
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        let center = gtk::CenterBox::new();
        center.set_hexpand(true);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 20);
        content.set_size_request(712, -1);
        content.set_hexpand(false);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Start);
        content.set_margin_top(16);
        content.set_margin_bottom(36);
        content.set_margin_start(24);
        content.set_margin_end(24);

        let brand_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        brand_row.set_margin_bottom(12);
        let mark_texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(
            include_bytes!("../../../../icon.iconset/icon_32x32@2x.png"),
        ))
        .expect("bundled application icon");
        let mark = gtk::Image::from_paintable(Some(&mark_texture));
        mark.set_pixel_size(32);
        brand_row.append(&mark);
        let brand = gtk::Label::new(Some("TYPSMTHNG"));
        brand.add_css_class("home-mark");
        brand.set_hexpand(true);
        brand.set_halign(gtk::Align::Start);
        brand_row.append(&brand);
        let theme_button = icon_button("weather-clear-night-symbolic", "Change theme (Ctrl+J)");
        theme_button.set_action_name(Some("app.cycle-theme"));
        let guide_button = gtk::Button::with_label("Guide");
        guide_button.add_css_class("flat");
        let settings_button = icon_button("preferences-system-symbolic", "Settings");
        brand_row.append(&theme_button);
        brand_row.append(&guide_button);
        brand_row.append(&settings_button);
        content.append(&brand_row);

        let workspace_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        workspace_panel.add_css_class("workspace-panel");
        let workspace_label = gtk::Label::new(Some("WORKSPACES"));
        workspace_label.set_halign(gtk::Align::Start);
        workspace_label.add_css_class("eyebrow");
        workspace_panel.append(&workspace_label);
        let workspace_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let workspace_model = gtk::StringList::new(&["All projects"]);
        let workspace_picker =
            gtk::DropDown::new(Some(workspace_model.clone()), None::<gtk::Expression>);
        workspace_picker.set_hexpand(true);
        let add_workspace = gtk::Button::with_label("+ Add Workspace");
        let manage_workspace = gtk::Button::with_label("Manage…");
        manage_workspace.add_css_class("flat");
        workspace_row.append(&workspace_picker);
        workspace_row.append(&add_workspace);
        workspace_row.append(&manage_workspace);
        workspace_panel.append(&workspace_row);
        content.append(&workspace_panel);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let open_button = gtk::Button::with_label("Open folder…");
        open_button.add_css_class("flat");
        open_button.set_tooltip_text(Some("Open an existing Typst project (Ctrl+O)"));
        let template_button = gtk::Button::with_label("Templates…");
        template_button.add_css_class("flat");
        let select_button = gtk::ToggleButton::with_label("Select");
        select_button.add_css_class("flat");
        actions.append(&open_button);
        actions.append(&template_button);
        actions.append(&select_button);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);
        let assign_selected = gtk::Button::with_label("Move selected…");
        assign_selected.add_css_class("flat");
        assign_selected.set_sensitive(false);
        let export_selected = gtk::Button::with_label("Export selected…");
        export_selected.add_css_class("flat");
        export_selected.set_sensitive(false);
        actions.append(&assign_selected);
        actions.append(&export_selected);
        content.append(&actions);

        let recent_list = gtk::FlowBox::new();
        recent_list.set_selection_mode(gtk::SelectionMode::None);
        recent_list.set_activate_on_single_click(true);
        recent_list.set_min_children_per_line(2);
        recent_list.set_max_children_per_line(2);
        recent_list.set_column_spacing(10);
        recent_list.set_row_spacing(10);
        recent_list.set_homogeneous(true);
        recent_list.add_css_class("project-grid");
        content.append(&recent_list);
        let create_button = gtk::Button::with_label("+  New Project");
        create_button.add_css_class("new-project");
        create_button.set_tooltip_text(Some("Create a folder-backed project (Ctrl+N)"));
        content.append(&create_button);

        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        foot.add_css_class("home-footer");
        foot.set_halign(gtk::Align::Center);
        let import_button = gtk::Button::with_label("Import project…");
        import_button.add_css_class("flat");
        let export_button = gtk::Button::with_label("Export all…");
        export_button.add_css_class("flat");
        foot.append(&import_button);
        foot.append(&export_button);
        content.append(&foot);
        center.set_center_widget(Some(&content));
        scroll.set_child(Some(&center));
        root.append(&scroll);
        select_button.connect_toggled({
            let list = recent_list.clone();
            move |button| {
                list.unselect_all();
                list.set_selection_mode(if button.is_active() {
                    gtk::SelectionMode::Multiple
                } else {
                    gtk::SelectionMode::None
                });
                button.set_label(if button.is_active() {
                    "Cancel Select"
                } else {
                    "Select"
                });
            }
        });

        {
            let callback = callbacks.open_folder.clone();
            open_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.create_project.clone();
            create_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.show_guide.clone();
            guide_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.show_settings.clone();
            settings_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.import_project.clone();
            import_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.export_all.clone();
            export_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.create_from_template.clone();
            template_button.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.create_workspace.clone();
            add_workspace.connect_clicked(move |_| callback());
        }
        {
            let callback = callbacks.manage_workspace.clone();
            manage_workspace.connect_clicked(move |_| callback());
        }

        let workspace_ids = Rc::new(std::cell::RefCell::new(vec![String::new()]));
        let updating_workspaces = Rc::new(std::cell::Cell::new(false));
        workspace_picker.connect_selected_notify({
            let updating = updating_workspaces.clone();
            let callback = callbacks.select_workspace.clone();
            let workspace_ids = workspace_ids.clone();
            move |picker| {
                if updating.get() {
                    return;
                }
                let id = workspace_ids
                    .borrow()
                    .get(picker.selected() as usize)
                    .cloned();
                if let Some(id) = id {
                    callback(id);
                }
            }
        });

        let recent_paths = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        {
            let recent_paths = recent_paths.clone();
            let callback = callbacks.open_recent.clone();
            recent_list.connect_child_activated(move |list, row| {
                if list.selection_mode() != gtk::SelectionMode::None {
                    return;
                }
                let path = recent_paths.borrow().get(row.index() as usize).cloned();
                if let Some(path) = path {
                    callback(path);
                }
            });
        }
        recent_list.connect_selected_children_changed({
            let assign_selected = assign_selected.clone();
            let export_selected = export_selected.clone();
            move |list| {
                let has_selection = !list.selected_children().is_empty();
                assign_selected.set_sensitive(has_selection);
                export_selected.set_sensitive(has_selection);
            }
        });
        assign_selected.connect_clicked({
            let recent_list = recent_list.clone();
            let recent_paths = recent_paths.clone();
            let callback = callbacks.assign_selected_workspace.clone();
            move |_| callback(selected_recent_paths(&recent_list, &recent_paths.borrow()))
        });
        export_selected.connect_clicked({
            let recent_list = recent_list.clone();
            let recent_paths = recent_paths.clone();
            let callback = callbacks.export_selected.clone();
            move |_| callback(selected_recent_paths(&recent_list, &recent_paths.borrow()))
        });

        Self {
            root,
            recent_list,
            recent_paths,
            workspace_picker,
            workspace_model,
            workspace_ids,
            callbacks,
            updating_workspaces,
        }
    }

    pub fn set_workspaces(&self, workspaces: &[(String, String)], selected: Option<&str>) {
        self.updating_workspaces.set(true);
        self.workspace_model
            .splice(0, self.workspace_model.n_items(), &[]);
        self.workspace_model.append("All projects");
        let mut ids = vec![String::new()];
        let mut selected_index = 0;
        for (index, (id, name)) in workspaces.iter().enumerate() {
            self.workspace_model.append(name);
            ids.push(id.clone());
            if selected == Some(id.as_str()) {
                selected_index = (index + 1) as u32;
            }
        }
        self.workspace_ids.replace(ids);
        self.workspace_picker.set_selected(selected_index);
        self.updating_workspaces.set(false);
    }

    pub fn set_recents(&self, projects: &[RecentProjectRow]) {
        while let Some(child) = self.recent_list.first_child() {
            self.recent_list.remove(&child);
        }
        self.recent_paths.borrow_mut().clear();

        self.recent_list
            .set_min_children_per_line(if projects.is_empty() { 1 } else { 2 });
        self.recent_list
            .set_max_children_per_line(if projects.is_empty() { 1 } else { 2 });
        if projects.is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 5);
            empty.set_margin_top(30);
            empty.set_margin_bottom(30);
            let label = gtk::Label::new(Some("No projects yet"));
            label.add_css_class("section-title");
            empty.append(&label);
            let hint = gtk::Label::new(Some("Open a folder or create a project to begin."));
            hint.add_css_class("muted");
            empty.append(&hint);
            self.recent_list.insert(&empty, -1);
            return;
        }

        for project in projects {
            self.recent_paths.borrow_mut().push(project.path.clone());
            let row = gtk::FlowBoxChild::new();
            row.add_css_class("recent-row");
            let line = gtk::Box::new(gtk::Orientation::Vertical, 10);
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.set_margin_top(10);
            line.set_margin_bottom(10);
            line.set_margin_start(12);
            line.set_margin_end(12);
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            let name = gtk::Label::new(Some(&project.name));
            name.set_halign(gtk::Align::Start);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_max_width_chars(28);
            name.add_css_class("section-title");
            labels.append(&name);
            let path = gtk::Label::new(Some(&project.path));
            path.set_halign(gtk::Align::Start);
            path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            path.add_css_class("muted");
            path.add_css_class("mono");
            path.set_max_width_chars(28);
            path.add_css_class("caption");
            labels.append(&path);
            heading.append(&labels);
            let detail = gtk::Label::new(Some(&project.detail));
            detail.add_css_class("muted");
            detail.set_halign(gtk::Align::Start);
            detail.add_css_class("caption");
            let favorite = icon_button(
                if project.favorite {
                    "starred-symbolic"
                } else {
                    "non-starred-symbolic"
                },
                "Toggle favorite",
            );
            favorite.connect_clicked({
                let callback = self.callbacks.toggle_favorite.clone();
                let path = project.path.clone();
                move |_| callback(path.clone())
            });
            let more = gtk::MenuButton::new();
            more.set_icon_name("view-more-symbolic");
            more.add_css_class("flat");
            let popover = gtk::Popover::new();
            let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
            menu.set_margin_top(5);
            menu.set_margin_bottom(5);
            menu.set_margin_start(5);
            menu.set_margin_end(5);
            let rename = gtk::Button::with_label("Rename folder…");
            let remove = gtk::Button::with_label("Remove from recents");
            let assign = gtk::Button::with_label("Move to workspace…");
            remove.add_css_class("destructive-action");
            rename.connect_clicked({
                let callback = self.callbacks.rename_recent.clone();
                let path = project.path.clone();
                move |_| callback(path.clone())
            });
            assign.connect_clicked({
                let callback = self.callbacks.assign_workspace.clone();
                let path = project.path.clone();
                move |_| callback(path.clone())
            });
            remove.connect_clicked({
                let callback = self.callbacks.remove_recent.clone();
                let path = project.path.clone();
                move |_| callback(path.clone())
            });
            menu.append(&rename);
            menu.append(&assign);
            menu.append(&remove);
            popover.set_child(Some(&menu));
            more.set_popover(Some(&popover));
            heading.append(&favorite);
            heading.append(&more);
            line.append(&heading);
            line.append(&detail);
            row.set_child(Some(&line));
            self.recent_list.insert(&row, -1);
        }
    }
}

fn selected_recent_paths(list: &gtk::FlowBox, paths: &[String]) -> Vec<String> {
    list.selected_children()
        .into_iter()
        .filter_map(|row| paths.get(row.index() as usize).cloned())
        .collect()
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button
}
