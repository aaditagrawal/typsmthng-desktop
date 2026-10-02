//! Override only the equation region background; syntax colors remain inherited.
use super::model::UiSettings;
use gtk::gdk;

pub fn customized_scheme(
    base: &sourceview5::StyleScheme,
    settings: &UiSettings,
) -> Option<sourceview5::StyleScheme> {
    let color = if !settings.equation_highlighting {
        gdk::RGBA::TRANSPARENT
    } else {
        gdk::RGBA::parse(settings.equation_highlight_color.trim()).ok()?
    };
    let directory = tempfile::tempdir().ok()?;
    // GtkSourceView recognizes hexadecimal colors, but interprets rgb()/rgba()
    // strings as named scheme colors. Canonical hex also cannot inject XML.
    let color = format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        (color.red() * 255.0).round() as u8,
        (color.green() * 255.0).round() as u8,
        (color.blue() * 255.0).round() as u8,
        (color.alpha() * 255.0).round() as u8,
    );
    let xml = format!(
        r#"<?xml version="1.0"?>
<style-scheme id="typsmthng-custom-equations" name="Custom equations" version="1.0" parent-scheme="{}">
  <style name="typst:math" background="{}"/>
</style-scheme>"#,
        base.id(),
        color,
    );
    std::fs::write(directory.path().join("equations.xml"), xml).ok()?;
    let manager = sourceview5::StyleSchemeManager::new();
    let search = sourceview5::StyleSchemeManager::default();
    let search_paths = search.search_path();
    manager.set_search_path(
        &search_paths
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>(),
    );
    manager.append_search_path(directory.path().to_str()?);
    manager.scheme("typsmthng-custom-equations")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sourceview5::prelude::*;

    fn capture_if_requested(window: &gtk::Window, editor: &sourceview5::View, name: &str) {
        let Some(directory) = std::env::var_os("TYPSMTHNG_SNAPSHOT_DIR") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let context = glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(60);
        while std::time::Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let paintable = gtk::WidgetPaintable::new(Some(editor));
        let frame = gtk::Snapshot::new();
        paintable.snapshot(&frame, editor.width() as f64, editor.height() as f64);
        window
            .renderer()
            .unwrap()
            .render_texture(frame.to_node().unwrap(), None)
            .save_to_png(directory.join(name))
            .unwrap();
    }

    #[test]
    #[ignore = "requires a display; run one exact filter under Xvfb"]
    fn native_equation_background_settings_preserve_token_styles() {
        adw::init().unwrap();
        super::super::install_css();
        sourceview5::init();
        let schemes = sourceview5::StyleSchemeManager::default();
        schemes.append_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/styles"));
        let languages = sourceview5::LanguageManager::new();
        languages.append_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/language-specs"));
        let buffer = sourceview5::Buffer::with_language(&languages.language("typst").unwrap());
        buffer.set_text("Math $x + alpha$ and prose\n\n$ integral_0^1 x^2 dif x = 1/3 $\n\nEquation backgrounds can follow the theme, use a custom color, or be disabled.");
        let highlighted_editor = sourceview5::View::with_buffer(&buffer);
        highlighted_editor.add_css_class("typst-editor");
        highlighted_editor.set_cursor_visible(false);
        let highlighted_window = gtk::Window::builder()
            .default_width(680)
            .default_height(180)
            .child(&highlighted_editor)
            .build();
        highlighted_window.present();
        for id in ["typsmthng-light", "typsmthng-dark"] {
            adw::StyleManager::default().set_color_scheme(if id.ends_with("dark") {
                adw::ColorScheme::ForceDark
            } else {
                adw::ColorScheme::ForceLight
            });
            let base = schemes.scheme(id).unwrap();
            assert!(customized_scheme(&base, &UiSettings::default()).is_none());
            buffer.set_style_scheme(Some(&base));
            buffer.ensure_highlight(&buffer.start_iter(), &buffer.end_iter());
            capture_if_requested(
                &highlighted_window,
                &highlighted_editor,
                &format!("{id}-equations-default.png"),
            );
            for (enabled, custom, expected) in
                [(false, "", "rgba(0,0,0,0)"), (true, "#f0cafe", "#f0cafe")]
            {
                let settings = UiSettings {
                    equation_highlighting: enabled,
                    equation_highlight_color: custom.into(),
                    ..UiSettings::default()
                };
                let scheme = customized_scheme(&base, &settings).unwrap();
                let style = scheme.style("typst:math").unwrap();
                assert_eq!(
                    gdk::RGBA::parse(style.background().unwrap()).unwrap(),
                    gdk::RGBA::parse(expected).unwrap()
                );
                assert_eq!(
                    scheme.style("typst:math-symbol").unwrap().foreground(),
                    base.style("typst:math-symbol").unwrap().foreground()
                );
                buffer.set_style_scheme(Some(&scheme));
                buffer.ensure_highlight(&buffer.start_iter(), &buffer.end_iter());
                let tags = buffer.iter_at_offset(10).tags();
                assert!(tags.iter().any(|tag| tag.is_background_set()
                    && tag.background_rgba() == Some(gdk::RGBA::parse(expected).unwrap())));
                assert!(tags.iter().any(|tag| tag.is_foreground_set()));
                capture_if_requested(
                    &highlighted_window,
                    &highlighted_editor,
                    &format!(
                        "{id}-equations-{}.png",
                        if enabled { "custom" } else { "disabled" }
                    ),
                );
            }
            let invalid = UiSettings {
                equation_highlight_color: "invalid\"/>".into(),
                ..UiSettings::default()
            };
            assert!(customized_scheme(&base, &invalid).is_none());
        }
        highlighted_window.destroy();
        // Both switch states produce GTK-valid CSS under fatal warnings.
        let provider = gtk::CssProvider::new();
        let mut settings = UiSettings::default();
        provider.load_from_string(&super::super::model::editor_css(&settings));
        settings.editor_ligatures = false;
        provider.load_from_string(&super::super::model::editor_css(&settings));
        // Confirm GtkSourceView actually changes glyph shaping, using a font
        // with standard fi/ffi ligatures installed on the GTK test image.
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_text("office affine ffi fi fl");
        let editor = sourceview5::View::with_buffer(&buffer);
        editor.add_css_class("typst-editor");
        editor.set_cursor_visible(false);
        settings.editor_font_family = "DejaVu Serif".into();
        settings.font_size = 24;
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let window = gtk::Window::builder()
            .default_width(620)
            .default_height(120)
            .child(&editor)
            .build();
        window.present();
        let snapshot = |enabled| {
            settings.editor_ligatures = enabled;
            provider.load_from_string(&super::super::model::editor_css(&settings));
            let context = glib::MainContext::default();
            let until = std::time::Instant::now() + std::time::Duration::from_millis(80);
            while std::time::Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let paintable = gtk::WidgetPaintable::new(Some(&editor));
            let frame = gtk::Snapshot::new();
            paintable.snapshot(&frame, editor.width() as f64, editor.height() as f64);
            let texture = window
                .renderer()
                .unwrap()
                .render_texture(frame.to_node().unwrap(), None);
            let mut pixels = vec![0; (texture.width() * texture.height() * 4) as usize];
            texture.download(&mut pixels, texture.width() as usize * 4);
            pixels
        };
        let mut snapshot = snapshot;
        let enabled = snapshot(true);
        let disabled = snapshot(false);
        assert!(
            enabled != disabled,
            "ligature toggle must change rendered glyphs"
        );
        assert!(
            enabled == snapshot(true),
            "turning ligatures back on restores shaping"
        );
        window.destroy();
        gtk::style_context_remove_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &provider,
        );
    }
}
