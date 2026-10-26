use std::{cell::RefCell, sync::OnceLock};

use adw::prelude::*;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdatePhase {
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Installing,
    CheckFailed,
    DownloadFailed,
    InstallFailed,
}

impl UpdatePhase {
    pub fn busy(self) -> bool {
        matches!(self, Self::Checking | Self::Downloading | Self::Installing)
    }

    pub fn header_label(self) -> &'static str {
        match self {
            Self::Idle | Self::Current => "Check for updates",
            Self::Checking => "Checking for updates…",
            Self::Available => "Update available",
            Self::Downloading => "Downloading update…",
            Self::Ready => "Restart to update",
            Self::Installing => "Preparing restart…",
            Self::CheckFailed => "Retry update check",
            Self::DownloadFailed => "Retry update",
            Self::InstallFailed => "Retry restart",
        }
    }
}

pub(super) struct UpdateDialog {
    pub dialog: adw::Dialog,
    pub status: gtk::Label,
    pub progress: gtk::ProgressBar,
    pub primary: gtk::Button,
    pub release_link: gtk::LinkButton,
    notes: gtk::Box,
    rendered_notes: RefCell<Option<String>>,
}

fn content_dialog(title: &str) -> (adw::Dialog, gtk::Box, gtk::Box) {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(600)
        .content_height(560)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&content));
    layout.append(&scroll);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.set_halign(gtk::Align::End);
    footer.set_margin_start(24);
    footer.set_margin_end(24);
    footer.set_margin_top(16);
    footer.set_margin_bottom(16);
    layout.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    layout.append(&footer);
    toolbar.set_content(Some(&layout));
    dialog.set_child(Some(&toolbar));
    (dialog, content, footer)
}

fn close_button(dialog: &adw::Dialog, label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    let weak = dialog.downgrade();
    button.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    button
}

pub(super) fn text_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label
}

fn inline_text(text: &str) -> String {
    static LINKS: OnceLock<Regex> = OnceLock::new();
    LINKS
        .get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap())
        .replace_all(text, "$1")
        .replace("**", "")
        .replace('`', "")
}

fn append_markdown(content: &gtk::Box, markdown: &str) {
    let mut paragraph = Vec::new();
    let flush = |paragraph: &mut Vec<&str>| {
        if !paragraph.is_empty() {
            content.append(&text_label(&inline_text(&paragraph.join(" "))));
            paragraph.clear();
        }
    };
    // GitHub response text is untrusted. Keep it plain, bounded, and scrollable.
    let mut used = 0;
    let mut truncated = false;
    for (index, line) in markdown.lines().enumerate() {
        used += line.len();
        if index >= 200 || used > 32 * 1024 {
            truncated = true;
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            flush(&mut paragraph);
        } else if line.starts_with('#') {
            flush(&mut paragraph);
            if line.starts_with("# v") {
                continue;
            }
            let label = text_label(&inline_text(line.trim_start_matches('#').trim()));
            label.add_css_class("heading");
            content.append(&label);
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush(&mut paragraph);
            content.append(&text_label(&format!("• {}", inline_text(item))));
        } else {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph);
    if truncated {
        content.append(&text_label(
            "More changes are available in the full release notes on GitHub.",
        ));
    }
}

impl UpdateDialog {
    pub fn new(activate: impl Fn() + 'static) -> Self {
        let (dialog, content, footer) = content_dialog("Application updates");
        let status = text_label("");
        content.append(&status);
        let progress = gtk::ProgressBar::new();
        progress.set_show_text(true);
        progress.set_visible(false);
        content.append(&progress);
        let notes = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&notes);
        let release_link = gtk::LinkButton::with_label(
            "https://github.com/aaditagrawal/typsmthng-desktop/releases",
            "View release on GitHub",
        );
        release_link.set_halign(gtk::Align::Start);
        content.append(&release_link);
        footer.append(&close_button(&dialog, "Later"));
        let primary = gtk::Button::with_label("Check for updates");
        primary.add_css_class("suggested-action");
        primary.connect_clicked(move |_| activate());
        footer.append(&primary);
        dialog.set_default_widget(Some(&primary));
        dialog.set_focus(Some(&primary));
        Self {
            dialog,
            status,
            progress,
            primary,
            release_link,
            notes,
            rendered_notes: RefCell::new(None),
        }
    }

    pub fn set_notes(&self, markdown: &str) {
        if self.rendered_notes.borrow().as_deref() == Some(markdown) {
            return;
        }
        while let Some(child) = self.notes.first_child() {
            self.notes.remove(&child);
        }
        if !markdown.trim().is_empty() {
            let title = text_label("What's changed");
            title.add_css_class("heading");
            self.notes.append(&title);
            append_markdown(&self.notes, markdown);
        }
        self.rendered_notes.replace(Some(markdown.to_string()));
        self.dialog
            .set_content_height(if markdown.trim().is_empty() { 280 } else { 560 });
    }
}

pub(super) fn changelog_dialog(
    releases: &[typsmthng_gtk::backend::release_notes::ReleaseNotes],
) -> adw::Dialog {
    let (dialog, content, footer) = content_dialog("What's new in typsmthng");
    for release in releases {
        let title = text_label(&format!("Version {}", release.version));
        title.add_css_class("title-3");
        content.append(&title);
        append_markdown(&content, &release.markdown);
    }
    let continue_button = close_button(&dialog, "Continue");
    continue_button.add_css_class("suggested-action");
    footer.append(&continue_button);
    dialog.set_default_widget(Some(&continue_button));
    dialog.set_focus(Some(&continue_button));
    dialog
}
