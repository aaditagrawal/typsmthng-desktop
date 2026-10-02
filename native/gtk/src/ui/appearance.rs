use gtk::prelude::*;

use super::model::Theme;

#[derive(Clone)]
pub struct ThemeButton {
    pub button: gtk::Button,
    icon: gtk::Image,
    system: gtk::DrawingArea,
}

impl ThemeButton {
    pub fn new() -> Self {
        let button = gtk::Button::new();
        button.add_css_class("flat");
        let overlay = gtk::Overlay::new();
        let icon = gtk::Image::from_icon_name("weather-clear-symbolic");
        icon.set_pixel_size(16);
        overlay.set_child(Some(&icon));
        // Laptop is not included in every platform's symbolic icon theme.
        let system = gtk::DrawingArea::new();
        system.set_content_width(10);
        system.set_content_height(10);
        system.set_draw_func(|area, context, _, _| {
            let color = area.color();
            context.set_source_rgba(
                f64::from(color.red()),
                f64::from(color.green()),
                f64::from(color.blue()),
                f64::from(color.alpha()),
            );
            context.set_line_width(1.0);
            context.rectangle(2.0, 2.0, 6.0, 5.0);
            context.move_to(2.0, 7.0);
            context.line_to(0.5, 8.5);
            context.line_to(9.5, 8.5);
            context.line_to(8.0, 7.0);
            context.stroke().expect("draw system appearance badge");
        });
        system.set_halign(gtk::Align::End);
        system.set_valign(gtk::Align::End);
        system.add_css_class("theme-system-badge");
        overlay.add_overlay(&system);
        button.set_child(Some(&overlay));
        let result = Self {
            button,
            icon,
            system,
        };
        result.update(Theme::System);
        result
    }

    pub fn update(&self, theme: Theme) {
        let dark = match theme {
            Theme::System => adw::StyleManager::default().is_dark(),
            Theme::Light => false,
            Theme::Dark => true,
        };
        self.icon.set_icon_name(Some(if dark {
            "weather-clear-night-symbolic"
        } else {
            "weather-clear-symbolic"
        }));
        self.system.set_visible(theme == Theme::System);
        let mode = match theme {
            Theme::System if dark => "System appearance (dark)",
            Theme::System => "System appearance (light)",
            Theme::Light => "Light appearance",
            Theme::Dark => "Dark appearance",
        };
        self.button.set_tooltip_text(Some(&format!(
            "{mode}. Cycle system, light, and dark (Ctrl+J)"
        )));
    }
}
