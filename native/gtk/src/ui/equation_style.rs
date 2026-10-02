//! Override only the equation region background; syntax colors remain inherited.
use super::model::UiSettings;
use gtk::gdk;

pub fn parse_color(color: &str) -> Option<gdk::RGBA> {
    if color.trim().eq_ignore_ascii_case("transparent") {
        Some(gdk::RGBA::TRANSPARENT)
    } else {
        gdk::RGBA::parse(color.trim()).ok()
    }
}

pub fn customized_scheme(
    base: &sourceview5::StyleScheme,
    settings: &UiSettings,
) -> Option<sourceview5::StyleScheme> {
    let requested = if !settings.equation_highlighting {
        None
    } else {
        Some(parse_color(&settings.equation_highlight_color)?)
    };
    // An empty local style clears the inherited background. Transparent
    // GtkTextTag backgrounds are painted as black by GTK's Cairo renderer.
    let background_attribute = match requested.filter(|color| color.alpha() > 0.0) {
        None => String::new(),
        Some(color) => {
            let alpha = color.alpha();
            let paper = base.style("text")?.background()?;
            let paper = paper
                .strip_prefix('#')
                .filter(|value| value.starts_with("rgb"))
                .unwrap_or(paper.as_str());
            let background = gdk::RGBA::parse(paper).ok()?;
            // Blend alpha before GTK creates its opaque text background tag.
            let component = |ink: f32, paper: f32| {
                ((ink * alpha + paper * (1.0 - alpha)) * 255.0).round() as u8
            };
            format!(
                " background=\"#{:02x}{:02x}{:02x}\"",
                component(color.red(), background.red()),
                component(color.green(), background.green()),
                component(color.blue(), background.blue())
            )
        }
    };
    let metadata: String = ["variant", "light-variant", "dark-variant"]
        .iter()
        .filter_map(|name| {
            base.metadata(name).map(|value| {
                format!(
                    "<property name=\"{name}\">{}</property>",
                    glib::markup_escape_text(&value)
                )
            })
        })
        .collect();
    // GtkSourceView generates widget CSS from the child scheme's own text
    // style. Copy both colors explicitly so prose keeps the base palette.
    let text_style = base.style("text")?;
    let text_foreground = text_style
        .foreground()
        .map(|value| format!(" foreground=\"{}\"", glib::markup_escape_text(&value)))
        .unwrap_or_default();
    let text_background = text_style
        .background()
        .map(|value| format!(" background=\"{}\"", glib::markup_escape_text(&value)))
        .unwrap_or_default();
    let directory = tempfile::tempdir().ok()?;
    let xml = format!(
        r#"<?xml version="1.0"?>
<style-scheme id="typsmthng-custom-equations" name="Custom equations" version="1.0" parent-scheme="{}">
  <metadata>{}</metadata>
  <style name="text"{}{}/>
  <style name="typst:math"{}/>
</style-scheme>"#,
        base.id(),
        metadata,
        text_foreground,
        text_background,
        background_attribute,
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

    fn rendered_texture(window: &gtk::Window, editor: &sourceview5::View) -> gdk::Texture {
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
    }

    fn capture_if_requested(texture: &gdk::Texture, name: &str) {
        let Some(directory) = std::env::var_os("TYPSMTHNG_SNAPSHOT_DIR") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        texture.save_to_png(directory.join(name)).unwrap();
    }

    fn background_pixel(
        texture: &gdk::Texture,
        editor: &sourceview5::View,
        offset: i32,
    ) -> [u8; 4] {
        let rectangle = editor.iter_location(&editor.buffer().iter_at_offset(offset));
        let (x, y) = editor.buffer_to_window_coords(
            gtk::TextWindowType::Widget,
            rectangle.x() + rectangle.width() / 2,
            rectangle.y() + rectangle.height() / 2,
        );
        let mut pixels = vec![0; (texture.width() * texture.height() * 4) as usize];
        let stride = texture.width() as usize * 4;
        texture.download(&mut pixels, stride);
        pixels[y as usize * stride + x as usize * 4..][..4]
            .try_into()
            .unwrap()
    }

    fn prose_pixels(texture: &gdk::Texture, editor: &sourceview5::View) -> Vec<u8> {
        let end = editor.iter_location(&editor.buffer().iter_at_offset(4));
        let (width, _) =
            editor.buffer_to_window_coords(gtk::TextWindowType::Widget, end.x() + end.width(), 0);
        let stride = texture.width() as usize * 4;
        let mut pixels = vec![0; stride * texture.height() as usize];
        texture.download(&mut pixels, stride);
        pixels
            .chunks_exact(stride)
            .take(end.height() as usize)
            .flat_map(|row| row[..width as usize * 4].iter().copied())
            .collect()
    }

    #[test]
    #[ignore = "requires a display; run one exact filter under Xvfb"]
    fn native_equation_background_settings_preserve_token_styles() {
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
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
            let baseline = rendered_texture(&highlighted_window, &highlighted_editor);
            let prose_background = background_pixel(&baseline, &highlighted_editor, 4);
            capture_if_requested(&baseline, &format!("{id}-equations-default.png"));
            let background = base.style("text").unwrap().background().unwrap();
            for (enabled, custom, expected) in [
                (false, "", background.as_str()),
                (true, "transparent", background.as_str()),
                (true, "#f0cafe", "#f0cafe"),
            ] {
                let settings = UiSettings {
                    equation_highlighting: enabled,
                    equation_highlight_color: custom.into(),
                    ..UiSettings::default()
                };
                let scheme = customized_scheme(&base, &settings)
                    .unwrap_or_else(|| panic!("{id} {custom:?} enabled={enabled}"));
                if !enabled || custom == "transparent" {
                    assert!(scheme
                        .style("typst:math")
                        .and_then(|style| style.background())
                        .is_none());
                } else {
                    let style = scheme.style("typst:math").unwrap();
                    assert_eq!(
                        gdk::RGBA::parse(style.background().unwrap()).unwrap(),
                        gdk::RGBA::parse(expected).unwrap()
                    );
                }
                assert_eq!(
                    scheme.style("typst:math-symbol").unwrap().foreground(),
                    base.style("typst:math-symbol").unwrap().foreground()
                );
                buffer.set_style_scheme(Some(&scheme));
                buffer.ensure_highlight(&buffer.start_iter(), &buffer.end_iter());
                let tags = buffer.iter_at_offset(10).tags();
                if enabled && custom != "transparent" {
                    assert!(tags.iter().any(|tag| tag.is_background_set()
                        && tag.background_rgba() == Some(gdk::RGBA::parse(expected).unwrap())));
                } else {
                    assert!(tags.iter().all(|tag| !tag.is_background_set()));
                }
                assert!(tags.iter().any(|tag| tag.is_foreground_set()));
                let texture = rendered_texture(&highlighted_window, &highlighted_editor);
                assert_eq!(
                    background_pixel(&texture, &highlighted_editor, 4),
                    prose_background,
                    "customizing equations must preserve baseline prose background in {id}"
                );
                assert!(
                    prose_pixels(&texture, &highlighted_editor)
                        == prose_pixels(&baseline, &highlighted_editor),
                    "customizing equations must preserve prose glyph colors and background in {id}"
                );
                if !enabled || custom == "transparent" {
                    assert_eq!(
                        background_pixel(&texture, &highlighted_editor, 7),
                        background_pixel(&texture, &highlighted_editor, 4),
                        "disabled or transparent equations must match prose background in {id}"
                    );
                }
                capture_if_requested(
                    &texture,
                    &format!(
                        "{id}-equations-{}.png",
                        if enabled { "custom" } else { "disabled" }
                    ),
                );
            }
            let half_alpha = UiSettings {
                equation_highlight_color: "rgba(255, 255, 255, 0.5)".into(),
                ..UiSettings::default()
            };
            buffer.set_style_scheme(customized_scheme(&base, &half_alpha).as_ref());
            buffer.ensure_highlight(&buffer.start_iter(), &buffer.end_iter());
            let blended = rendered_texture(&highlighted_window, &highlighted_editor);
            let math_background = background_pixel(&blended, &highlighted_editor, 7);
            for channel in 0..3 {
                assert!((i16::from(math_background[channel]) - (i16::from(prose_background[channel]) + 255) / 2).abs() <= 1,
                    "half-alpha equation backgrounds should blend into baseline prose background in {id}");
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
