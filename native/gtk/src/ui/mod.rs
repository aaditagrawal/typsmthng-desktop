mod app;
mod home;
pub mod model;
mod presentation;
mod workspace;

pub use app::{launch, LaunchOptions};

pub fn install_css() {
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
