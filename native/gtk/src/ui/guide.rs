//! Offline, task-based help. Keep button names and shortcuts aligned with the UI.
use gtk::prelude::*;

struct Topic {
    title: &'static str,
    sections: &'static [(&'static str, &'static str)],
}

const TOPICS: &[Topic] = &[
    Topic {
        title: "Start a document",
        sections: &[
            ("Create your first project", "1. On the home screen, choose + New Project.\n2. Choose the parent folder where you want to keep your work.\n3. Enter a project name and choose Create. The app creates a folder with main.typ and opens the editor.\n4. Replace the starter text with your own. The preview updates as you type.\n5. Press Ctrl+S to save, then Ctrl+Shift+E to export a PDF."),
            ("Open work you already have", "Choose Open folder… on the home screen, or press Ctrl+O. Select the project folder containing your Typst files and assets. Click a file in the editor's file tree to edit it. You can also launch the app with a folder or .typ file from your file manager or terminal."),
            ("Start from a template", "Choose Templates… to pick a built-in starter or search Typst Universe. Choose the template, then choose a parent folder and name for your new project. Universe templates need an internet connection. Template layout settings may be locked; edit the template's source to change its layout."),
            ("Return home", "Click t. at the top left of the editor, or press Ctrl+Shift+H. Your project remains on disk and appears on the home screen. On macOS, use Command wherever this guide says Ctrl."),
        ],
    },
    Topic {
        title: "Workspaces and folders",
        sections: &[
            ("What lives where", "A project is a real folder on your computer. Its .typ files, images, bibliography, and subfolders live there and can be opened with other editors.\n\nA workspace is a named group of projects on the home screen. It stores a label and project assignments in the app's preferences. It does not create a folder or move any files. Projects in one workspace can live in different places on disk."),
            ("Create a workspace and add projects", "1. Choose + Add Workspace, enter a name, and choose Create. The new workspace starts empty.\n2. Switch the workspace picker to All projects.\n3. Choose Select and click the project cards you want to group.\n4. Choose Move selected…, choose the workspace, and confirm.\n5. Choose the workspace in the picker to see that group.\n\nEach project can belong to one workspace at a time. All projects shows both assigned and unassigned projects."),
            ("Find a new or missing project", "Creating or opening a project does not automatically add it to the selected workspace. Switch to All projects to find it, then use Select and Move selected… to assign it. If a project is no longer listed, choose Open folder… and open its folder again."),
            ("Rename, ungroup, or delete", "Select a workspace and choose Manage… to rename or delete the group. Deleting a workspace keeps its project folders and leaves those projects in All projects. To ungroup individual projects, use Move selected… and choose No workspace. Despite the word Move, this only changes the home-screen grouping.\n\nTo organize files inside a project, use folders in the editor's file tree. Those folders are real directories on disk."),
        ],
    },
    Topic {
        title: "Write and manage files",
        sections: &[
            ("A small Typst example", "Type this into main.typ to try a heading, text styles, a list, and an equation:\n\n= My first document\nHello, *bold* and _italic_ text.\n\n- First item\n- Second item\n\n$ x^2 + y^2 = z^2 $"),
            ("Add images and references", "Use the file tree's import control or drop files into the app to bring in assets. Keep them inside the project folder. An image at images/figure.png can be used with #image(\"images/figure.png\"). Add a bibliography file with #bibliography(\"refs.bib\") and cite an entry with @key, replacing key with its entry ID."),
            ("Work with project files", "Use New file and New folder above the file tree. Right-click a file for actions such as rename, duplicate, move, and delete. File deletion uses the system Trash when available. Renaming or moving an asset can require updating paths in your source. Ctrl+\\ hides or shows the file tree."),
            ("Search and edit", "Ctrl+F or Ctrl+H opens find and replace for the current document. Ctrl+K searches project files, content, and commands. Ctrl+Space opens Typst completion; hover over supported expressions for help.\n\nCtrl+/ toggles comments, Ctrl+D duplicates selected lines, and Ctrl+Shift+I formats the current Typst file. Formatting is one undo step and leaves files with syntax errors unchanged. Settings → Editor includes wrapping and optional Vim input."),
            ("Save your changes", "Auto-save writes the current file 100 ms after you stop typing by default. Settings → Saving lets you change the delay, turn off timed saving, or save when the window loses focus. Ctrl+S saves immediately. Switching files and closing also save. A failed save keeps your edits and stops navigation so you can resolve the error."),
        ],
    },
    Topic {
        title: "Preview and fix errors",
        sections: &[
            ("Read the compiled document", "Choose source, split, or preview view in the editor toolbar, or use Ctrl+1, Ctrl+2, and Ctrl+3. The preview compiles as you type, including unsaved edits. Ctrl+Enter requests a compile immediately. Typing pauses briefly before diagnostics appear."),
            ("Navigate and zoom", "Use the page arrows and zoom controls above the preview. The zoom menu includes Fit text width; pan sideways to inspect the page margins. The magnifier inspects text at 2× without changing the page zoom. Use the compiled headings menu to jump to a section, including headings from imported files."),
            ("Jump between source and preview", "Click rendered text or a formula to open its source location. To go the other way, place the cursor in the source and press Ctrl+Shift+J after compilation. Source links also work for local imported files."),
            ("When the preview does not update", "Check the diagnostics and activate an error to jump to its file and line. Correct the error, then press Ctrl+Enter. Check image and import paths against the project folder. Package downloads and automatic Google Fonts resolution may need internet access. Preview quality in Settings can be reduced if rendering is slow."),
            ("When another app changes a file", "The app watches files on disk. If an external edit conflicts with unsaved changes, a banner offers Reload from disk or Keep editor buffer. Reload replaces your unsaved text with the disk version. Keep editor buffer keeps your text for saving over the disk version. Resolve the conflict before continuing to save."),
        ],
    },
    Topic {
        title: "Import and export",
        sections: &[
            ("Import a project", "On the home screen, choose Import project… and select the appropriate import option. Project archives accept .typst or .zip files. LaTeX imports accept a .tex file, a directory, or an archive, and preserve relative assets. Choose a destination for the imported project and review the result in the editor. Custom macros, packages, and TikZ may need manual conversion; read any import warnings."),
            ("Export the rendered document", "Click the PDF toolbar button or press Ctrl+Shift+E, then choose a destination. This uses the default PDF profile. For other formats, open More actions and choose Export document. Select PDF with an optional PDF/A or PDF/UA profile, all SVG or PNG pages in a ZIP, or experimental HTML. Fix compilation errors before exporting."),
            ("Back up or share editable projects", "In the editor, use Export project from More actions or Ctrl+Shift+S. On the home screen, use Select and Export selected… to package chosen projects, or Export all… for the project collection. These archives contain source files and assets so the project can be edited elsewhere. Private .typsmthng app state is excluded; template metadata is retained."),
            ("Keep a backup", "Your project folders work with git, backup software, and file-sync tools. Auto-save is not version history. Keep a separate backup of important work, especially before importing, renaming assets, or resolving conflicting edits."),
        ],
    },
    Topic {
        title: "Present slides",
        sections: &[
            ("Start a presentation", "Open your slide project and wait for the preview to compile. F5 presents in the current window. Shift+F5 opens a presenter window and a separate audience window. The presentation menu also offers Present here and Presenter + audience. In presenter view, select the audience monitor before presenting."),
            ("Navigate the deck", "Right, Down, Page Down, Space, or Enter advances. Left, Up, Page Up, or Backspace goes back. Home and End jump to the first and last slides. Type a slide number and press Enter to jump directly. G or O opens the slide grid; Escape ends the presentation."),
            ("Notes and timer", "Presenter view shows current and next slides, speaker notes, and a timer. Type into the notes area for the current slide. Changes save automatically beside the deck, for example slides.notes.md for slides.typ. Inline notes from the document appear above the editable notes. S toggles notes, T pauses the timer, and R resets it. Note font controls help make text readable at a distance."),
            ("Point, draw, and blank the screen", "L selects the laser pointer, D the pen, H the highlighter, and E the eraser. C or Delete clears annotations on the current slide. B or period blanks to black; W or comma blanks to white. F or F11 toggles fullscreen."),
        ],
    },
    Topic {
        title: "Settings and shortcuts",
        sections: &[
            ("Make the editor comfortable", "Open Settings with Ctrl+, or the settings button. Choose editor fonts and font size, line wrapping, centered scrolling, and Vim input. Settings → Fonts controls ligatures. Settings → Equations controls equation backgrounds. Ctrl+J cycles system, light, and dark appearance."),
            ("Common shortcuts", "Ctrl+N   New project\nCtrl+O   Open folder\nCtrl+S   Save current file\nCtrl+Enter   Compile\nCtrl+F / Ctrl+H   Find and replace\nCtrl+K   Project search and commands\nCtrl+Space   Typst completion\nCtrl+Shift+I   Format document\nCtrl+Shift+J   Jump to preview\nCtrl+Shift+E   Export PDF\nCtrl+Shift+S   Export project\nCtrl+Shift+H   Return home\nF5 / Shift+F5   Present / presenter view\n\nOn macOS, use Command in place of Ctrl. Escape closes this guide."),
            ("Application data and updates", "Preferences, recent projects, workspace groups, and window state are stored in your platform's application-data directory, separately from project folders. Resetting app preferences can lose those groups without deleting your files.\n\nWhen an update button appears, click it to download the stable release, then restart to install. Linux system packages update through their package manager."),
        ],
    },
];

pub fn window(parent: &impl IsA<gtk::Window>) -> gtk::Window {
    let window = gtk::Window::builder()
        .title("User guide — typsmthng")
        .transient_for(parent)
        .destroy_with_parent(true)
        .default_width(760)
        .default_height(720)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some("typsmthng user guide"))));
    window.set_titlebar(Some(&header));

    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    navigation.set_margin_start(24);
    navigation.set_margin_end(24);
    navigation.set_margin_top(16);
    navigation.set_margin_bottom(16);
    let label = gtk::Label::new(Some("_Topic"));
    label.set_use_underline(true);
    let titles: Vec<_> = TOPICS.iter().map(|topic| topic.title).collect();
    let topics = gtk::DropDown::from_strings(&titles);
    topics.set_hexpand(true);
    label.set_mnemonic_widget(Some(&topics));
    navigation.append(&label);
    navigation.append(&topics);
    root.append(&navigation);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    for (index, topic) in TOPICS.iter().enumerate() {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content.set_margin_top(24);
        content.set_margin_bottom(32);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let title = text_label(topic.title);
        title.add_css_class("title-1");
        content.append(&title);
        for (heading, body) in topic.sections {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let heading = text_label(heading);
            heading.add_css_class("heading");
            section.append(&heading);
            section.append(&text_label(body));
            content.append(&section);
        }
        let scroll = gtk::ScrolledWindow::new();
        // A scrolled window has no useful minimum height. It must consume the
        // remaining window space, otherwise the guide collapses to a thin strip.
        scroll.set_vexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_child(Some(&content));
        stack.add_named(&scroll, Some(&index.to_string()));
    }
    topics.connect_selected_notify({
        let stack = stack.clone();
        move |topics| stack.set_visible_child_name(&topics.selected().to_string())
    });
    root.append(&stack);
    window.set_child(Some(&root));
    super::dismiss_on_escape(&window);
    window
}

fn text_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a display; run under Xvfb"]
    fn guide_topics_have_readable_scrollable_content() {
        adw::init().unwrap();
        super::super::install_css();
        let parent = gtk::Window::new();
        let guide = window(&parent);
        guide.present();
        let root = guide.child().unwrap().downcast::<gtk::Box>().unwrap();
        let navigation = root.first_child().unwrap();
        let picker = navigation
            .last_child()
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap();
        let stack = root.last_child().unwrap().downcast::<gtk::Stack>().unwrap();
        let settle = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
            while std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        for (width, height) in [(760, 720), (460, 520)] {
            guide.set_default_size(width, height);
            for index in 0..TOPICS.len() {
                picker.set_selected(index as u32);
                settle();
                let scroll = stack
                    .visible_child()
                    .unwrap()
                    .downcast::<gtk::ScrolledWindow>()
                    .unwrap();
                assert!(scroll.height() > 250, "guide viewport collapsed");
                assert!(
                    guide.width() <= width,
                    "guide cannot shrink to requested width"
                );
                let vertical = scroll.vadjustment();
                if height < 600 {
                    assert!(
                        vertical.upper() > vertical.page_size(),
                        "narrow topic must scroll"
                    );
                }
                vertical.set_value(vertical.upper() - vertical.page_size());
                settle();
                assert!(
                    (vertical.value() + vertical.page_size() - vertical.upper()).abs() < 1.0,
                    "cannot reach end of topic"
                );
                let horizontal = scroll.hadjustment();
                assert!(
                    horizontal.upper() <= horizontal.page_size(),
                    "text clips horizontally"
                );
                if let Some(base) = std::env::var_os("TYPSMTHNG_SNAPSHOT_DIR") {
                    vertical.set_value(0.0);
                    settle();
                    let directory =
                        std::path::PathBuf::from(&base).join(format!("{width}-{index}"));
                    std::env::set_var("TYPSMTHNG_SNAPSHOT_DIR", directory);
                    super::super::smoke::capture_windows(&gtk::Application::default());
                    std::env::set_var("TYPSMTHNG_SNAPSHOT_DIR", base);
                }
            }
        }
        guide.close();
        settle();
        assert!(!guide.is_visible());
        parent.close();
    }
}
