//! Shared zoom input: Ctrl+scroll steps and a transient zoom-level badge.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

/// Touchpads report pixel deltas; this many pixels count as one wheel notch.
const SURFACE_PIXELS_PER_STEP: f64 = 40.0;

/// Call `on_step(steps, x, y)` for Ctrl+scroll over `widget`; positive steps
/// zoom in. Plain scrolling passes through to the scrolled window.
pub fn connect_ctrl_scroll(
    widget: &impl IsA<gtk::Widget>,
    on_step: impl Fn(i32, f64, f64) + 'static,
) {
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    // Capture before the ScrolledWindow consumes the event as a scroll.
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let pointer = Rc::new(Cell::new((0.0, 0.0)));
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion({
        let pointer = pointer.clone();
        move |_, x, y| pointer.set((x, y))
    });
    widget.add_controller(motion);
    let accumulated = Cell::new(0.0);
    scroll.connect_scroll(move |controller, _, dy| {
        if !controller
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            accumulated.set(0.0);
            return glib::Propagation::Proceed;
        }
        let delta = if controller.unit() == gtk::gdk::ScrollUnit::Surface {
            dy / SURFACE_PIXELS_PER_STEP
        } else {
            dy
        };
        let total = accumulated.get() - delta;
        let steps = total.trunc();
        accumulated.set(total - steps);
        if steps != 0.0 {
            let (x, y) = pointer.get();
            on_step(steps as i32, x, y);
        }
        glib::Propagation::Stop
    });
    widget.add_controller(scroll);
}

/// A small pill that briefly reports the zoom level over a pane.
#[derive(Clone)]
pub struct ZoomBadge {
    revealer: gtk::Revealer,
    label: gtk::Label,
    hide: Rc<RefCell<Option<glib::SourceId>>>,
}

impl ZoomBadge {
    pub fn new() -> Self {
        let label = gtk::Label::new(None);
        label.add_css_class("zoom-badge");
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::Crossfade);
        revealer.set_transition_duration(160);
        revealer.set_child(Some(&label));
        revealer.set_halign(gtk::Align::End);
        revealer.set_valign(gtk::Align::End);
        revealer.set_margin_end(18);
        revealer.set_margin_bottom(14);
        revealer.set_can_target(false);
        Self {
            revealer,
            label,
            hide: Rc::new(RefCell::new(None)),
        }
    }

    pub fn widget(&self) -> &gtk::Revealer {
        &self.revealer
    }

    pub fn show(&self, text: &str) {
        self.label.set_text(text);
        self.revealer.set_reveal_child(true);
        if let Some(timer) = self.hide.borrow_mut().take() {
            timer.remove();
        }
        let revealer = self.revealer.downgrade();
        let hide = self.hide.clone();
        let timer = glib::timeout_add_local_once(Duration::from_millis(900), move || {
            hide.borrow_mut().take();
            if let Some(revealer) = revealer.upgrade() {
                revealer.set_reveal_child(false);
            }
        });
        self.hide.replace(Some(timer));
    }
}

/// Logical pixels per typographic point at 100% (CSS reference pixel, 96 DPI).
pub const PIXELS_PER_POINT: f64 = 96.0 / 72.0;
pub const MIN_PREVIEW_SCALE: f64 = 0.1;
pub const MAX_PREVIEW_SCALE: f64 = 5.0;
/// Button and menu stops, as in document viewers such as Evince and Okular.
pub const PREVIEW_SCALE_PRESETS: [f64; 15] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0,
];
/// Ctrl+scroll zooms continuously by this factor per wheel notch.
pub const PREVIEW_SCROLL_FACTOR: f64 = 1.1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreviewZoom {
    /// Every page fills the viewport width.
    FitWidth,
    /// Zoom the full page so its text column fits the viewport width.
    FitText,
    /// Fixed scale where 1.0 shows pages at their physical size.
    Scale(f64),
}

impl PreviewZoom {
    pub fn clamped(scale: f64) -> Self {
        PreviewZoom::Scale(scale.clamp(MIN_PREVIEW_SCALE, MAX_PREVIEW_SCALE))
    }

    /// The scale actually displayed for a page `page_width` points wide
    /// whose visible text column is `column` (see [`fit_text_width`]).
    pub fn scale(self, available_width: f64, page_width: f64, column: Option<(f64, f64)>) -> f64 {
        let visible_width = match (self, column) {
            (PreviewZoom::Scale(scale), _) => return scale,
            (PreviewZoom::FitText, Some(column)) => fit_text_width(page_width, column),
            _ => page_width,
        };
        available_width / (visible_width * PIXELS_PER_POINT)
    }
}

/// The measured text width in points. Invalid or missing measurements fall
/// back to the full page rather than requesting an unbounded zoom.
pub fn fit_text_width(page_width: f64, (left, right): (f64, f64)) -> f64 {
    if !left.is_finite() || !right.is_finite() || right <= left {
        return page_width;
    }
    (right.min(page_width) - left.max(0.0)).clamp(1.0, page_width)
}

/// The next preset strictly beyond `scale` in `direction` (±1).
pub fn step_preview_scale(scale: f64, direction: i32) -> f64 {
    // Tolerate rounding so 1.0 → 1.1 rather than 1.0 → 1.0.
    let epsilon = 0.005;
    let next = if direction > 0 {
        PREVIEW_SCALE_PRESETS
            .iter()
            .copied()
            .find(|preset| *preset > scale + epsilon)
    } else {
        PREVIEW_SCALE_PRESETS
            .iter()
            .rev()
            .copied()
            .find(|preset| *preset < scale - epsilon)
    };
    next.unwrap_or(scale)
        .clamp(MIN_PREVIEW_SCALE, MAX_PREVIEW_SCALE)
}

pub fn format_scale(scale: f64) -> String {
    format!("{}%", (scale * 100.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_steps_leave_fitted_scales_in_the_right_direction() {
        assert_eq!(step_preview_scale(1.0, 1), 1.1);
        assert_eq!(step_preview_scale(1.0, -1), 0.9);
        // A fitted 87% steps to the neighbouring presets, never backwards.
        assert_eq!(step_preview_scale(0.87, 1), 0.9);
        assert_eq!(step_preview_scale(0.87, -1), 0.75);
        assert_eq!(step_preview_scale(4.0, 1), 4.0);
        assert_eq!(step_preview_scale(0.25, -1), 0.25);
    }

    #[test]
    fn fit_text_uses_the_actual_column_width() {
        let a4 = 595.28;
        assert!((fit_text_width(a4, (70.87, 524.41)) - 453.54).abs() < 0.01);
        assert_eq!(fit_text_width(a4, (100.0, 400.0)), 300.0);
        assert_eq!(fit_text_width(a4, (-10.0, a4 + 10.0)), a4);
        assert_eq!(fit_text_width(a4, (400.0, 100.0)), a4);
        assert_eq!(fit_text_width(a4, (f64::NAN, 400.0)), a4);
        let fitted = PreviewZoom::FitText.scale(300.0 * PIXELS_PER_POINT, a4, Some((100.0, 400.0)));
        assert!((fitted - 1.0).abs() < 1e-9);
        assert_eq!(
            PreviewZoom::FitText.scale(a4 * PIXELS_PER_POINT, a4, None),
            PreviewZoom::FitWidth.scale(a4 * PIXELS_PER_POINT, a4, None)
        );
    }

    #[test]
    fn fit_width_reports_the_scale_it_displays() {
        let a4 = 595.28;
        let fitted = PreviewZoom::FitWidth.scale(a4 * PIXELS_PER_POINT, a4, None);
        assert!((fitted - 1.0).abs() < 1e-9);
        assert_eq!(PreviewZoom::Scale(1.5).scale(10.0, a4, None), 1.5);
        assert_eq!(
            PreviewZoom::clamped(9.0),
            PreviewZoom::Scale(MAX_PREVIEW_SCALE)
        );
        assert_eq!(format_scale(0.874), "87%");
    }
}
