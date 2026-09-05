use gtk::prelude::*;

mod app;
mod home;
pub mod model;
mod presentation;
mod smoke;
mod workspace;

pub use app::{launch, LaunchOptions};

pub fn install_css() {
    // Transient windows are separate CSS roots. Inherit the application theme
    // when they map, including dialogs created after the theme was selected.
    gtk::Window::toplevels().connect_items_changed(|model, position, _, added| {
        for index in position..position + added {
            if let Some(window) = model.item(index).and_downcast::<gtk::Window>() {
                window.connect_map(|window| {
                    if let Some(parent) = window.transient_for() {
                        if parent.has_css_class("dark") {
                            window.add_css_class("dark");
                        } else {
                            window.remove_css_class("dark");
                        }
                    }
                });
            }
        }
    });
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("../../resources/app.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
