mod app;
mod appearance;
mod controls;
mod editor_tools;
mod equation_style;
mod font_picker;
mod guide;
mod home;
mod minimap;
pub mod model;
mod page_paintable;
mod presentation;
mod smoke;
mod workspace;
mod zoom;

pub use app::{launch, LaunchOptions};

pub fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../../resources/app.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// The locale's default paper as a Typst paper name.
pub fn locale_page_size() -> &'static str {
    model::locale_page_size(&gtk::PaperSize::default())
}

/// Keep reusable utility windows alive while allowing Escape to dismiss them.
pub fn dismiss_on_escape(window: &gtk::Window) {
    use gtk::prelude::*;
    let keys = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
}
