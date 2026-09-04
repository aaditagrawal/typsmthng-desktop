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
    pub create_from_template: Rc<dyn Fn()>,
    pub create_workspace: Rc<dyn Fn()>,
    pub select_workspace: Rc<dyn Fn(String)>,
    pub manage_workspace: Rc<dyn Fn()>,
    pub assign_workspace: Rc<dyn Fn(String)>,
}

#[derive(Clone)]
pub struct HomeView {
    pub root: gtk::Box,
    recent_list: gtk::ListBox,
    recent_paths: Rc<std::cell::RefCell<Vec<String>>>,
    workspace_picker: gtk::DropDown,
    workspace_model: gtk::StringList,
    workspace_ids: Rc<std::cell::RefCell<Vec<String>>>,
    callbacks: HomeCallbacks,
}

impl HomeView {
    pub fn new(callbacks: HomeCallbacks) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        let header = gtk::HeaderBar::new();
        header.set_show_title_buttons(true);
        let brand = gtk::Label::new(Some("typsmthng"));
        brand.add_css_class("home-mark");
        header.set_title_widget(Some(&brand));
        let settings_button = icon_button("preferences-system-symbolic", "Settings");
        header.pack_end(&settings_button);
        root.append(&header);

        let center = gtk::CenterBox::new();
        center.set_vexpand(true);
        center.set_hexpand(true);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_size_request(680, -1);
        content.set_margin_top(40);
        content.set_margin_bottom(32);

        let eyebrow = gtk::Label::new(Some("NATIVE TYPST WORKSPACE"));
        eyebrow.set_halign(gtk::Align::Start);
        eyebrow.add_css_class("eyebrow");
        content.append(&eyebrow);

        let title = gtk::Label::new(Some("Write the source. Read the page."));
        title.set_halign(gtk::Align::Start);
        title.add_css_class("title");
        content.append(&title);

        let subtitle = gtk::Label::new(Some(
            "Folder-backed Typst editing, live native preview, and a complete presentation desk.",
        ));
        subtitle.set_halign(gtk::Align::Start);
        subtitle.set_wrap(true);
        subtitle.add_css_class("muted");
        content.append(&subtitle);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let open_button = gtk::Button::with_label("Open folder…");
        open_button.add_css_class("suggested-action");
        open_button.set_tooltip_text(Some("Open an existing Typst project (Ctrl+O)"));
        let create_button = gtk::Button::with_label("New project…");
        create_button.set_tooltip_text(Some("Create a folder-backed project (Ctrl+N)"));
        let guide_button = gtk::Button::with_label("Guide");
        guide_button.add_css_class("flat");
        let import_button = gtk::Button::with_label("Import…");
        import_button.add_css_class("flat");
        let export_button = gtk::Button::with_label("Export all…");
        export_button.add_css_class("flat");
        let template_button = gtk::Button::with_label("Templates…");
        template_button.add_css_class("flat");
        actions.append(&open_button);
        actions.append(&create_button);
        actions.append(&guide_button);
        actions.append(&template_button);
        actions.append(&import_button);
        actions.append(&export_button);
        content.append(&actions);

        let workspace_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let workspace_label = gtk::Label::new(Some("WORKSPACE"));
        workspace_label.add_css_class("eyebrow");
        let workspace_model = gtk::StringList::new(&["All projects"]);
        let workspace_picker =
            gtk::DropDown::new(Some(workspace_model.clone()), None::<gtk::Expression>);
        workspace_picker.set_hexpand(true);
        let add_workspace = gtk::Button::with_label("Add…");
        add_workspace.add_css_class("flat");
        let manage_workspace = gtk::Button::with_label("Manage…");
        manage_workspace.add_css_class("flat");
        workspace_row.append(&workspace_label);
        workspace_row.append(&workspace_picker);
        workspace_row.append(&add_workspace);
        workspace_row.append(&manage_workspace);
        content.append(&workspace_row);

        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.set_margin_top(8);
        content.append(&separator);

        let recents_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        recents_header.append(&section_label("Recent vaults"));
        let recents_hint = gtk::Label::new(Some("Your projects remain ordinary folders"));
        recents_hint.set_hexpand(true);
        recents_hint.set_halign(gtk::Align::End);
        recents_hint.add_css_class("muted");
        recents_hint.add_css_class("caption");
        recents_header.append(&recents_hint);
        content.append(&recents_header);

        let recent_list = gtk::ListBox::new();
        recent_list.set_selection_mode(gtk::SelectionMode::None);
        recent_list.set_activate_on_single_click(true);
        recent_list.add_css_class("boxed-list");
        let recent_scroll = gtk::ScrolledWindow::new();
        recent_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        recent_scroll.set_min_content_height(220);
        recent_scroll.set_child(Some(&recent_list));
        content.append(&recent_scroll);

        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        foot.set_margin_top(4);
        let native = gtk::Label::new(Some("GTK 4 · GtkSourceView · no WebView"));
        native.add_css_class("eyebrow");
        native.set_hexpand(true);
        native.set_halign(gtk::Align::Start);
        foot.append(&native);
        let shortcut = gtk::Label::new(Some("Ctrl+K search  ·  F5 present"));
        shortcut.add_css_class("eyebrow");
        foot.append(&shortcut);
        content.append(&foot);

        center.set_center_widget(Some(&content));
        root.append(&center);

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
        workspace_picker.connect_selected_notify({
            let callback = callbacks.select_workspace.clone();
            let workspace_ids = workspace_ids.clone();
            move |picker| {
                if let Some(id) = workspace_ids.borrow().get(picker.selected() as usize) {
                    callback(id.clone());
                }
            }
        });

        let recent_paths = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        {
            let recent_paths = recent_paths.clone();
            let callback = callbacks.open_recent.clone();
            recent_list.connect_row_activated(move |_, row| {
                if let Some(path) = recent_paths.borrow().get(row.index() as usize) {
                    callback(path.clone());
                }
            });
        }

        Self {
            root,
            recent_list,
            recent_paths,
            workspace_picker,
            workspace_model,
            workspace_ids,
            callbacks,
        }
    }

    pub fn set_workspaces(&self, workspaces: &[(String, String)], selected: Option<&str>) {
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
    }

    pub fn set_recents(&self, projects: &[RecentProjectRow]) {
        while let Some(child) = self.recent_list.first_child() {
            self.recent_list.remove(&child);
        }
        self.recent_paths.borrow_mut().clear();

        if projects.is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 5);
            empty.set_margin_top(30);
            empty.set_margin_bottom(30);
            let label = gtk::Label::new(Some("No recent vaults"));
            label.add_css_class("section-title");
            empty.append(&label);
            let hint = gtk::Label::new(Some("Open a folder or create a project to begin."));
            hint.add_css_class("muted");
            empty.append(&hint);
            self.recent_list.append(&empty);
            return;
        }

        for project in projects {
            self.recent_paths.borrow_mut().push(project.path.clone());
            let row = gtk::ListBoxRow::new();
            row.set_activatable(true);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            line.set_margin_top(10);
            line.set_margin_bottom(10);
            line.set_margin_start(12);
            line.set_margin_end(12);
            let glyph = gtk::Label::new(Some(if project.favorite { "★" } else { "◇" }));
            glyph.add_css_class("mono");
            line.append(&glyph);
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
            labels.set_hexpand(true);
            let name = gtk::Label::new(Some(&project.name));
            name.set_halign(gtk::Align::Start);
            name.add_css_class("section-title");
            labels.append(&name);
            let path = gtk::Label::new(Some(&project.path));
            path.set_halign(gtk::Align::Start);
            path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            path.add_css_class("muted");
            path.add_css_class("mono");
            labels.append(&path);
            line.append(&labels);
            let detail = gtk::Label::new(Some(&project.detail));
            detail.add_css_class("muted");
            line.append(&detail);
            let arrow = gtk::Image::from_icon_name("go-next-symbolic");
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
            line.append(&favorite);
            line.append(&more);
            line.append(&arrow);
            row.set_child(Some(&line));
            self.recent_list.append(&row);
        }
    }
}

fn section_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_halign(gtk::Align::Start);
    label.add_css_class("section-title");
    label
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button
}
