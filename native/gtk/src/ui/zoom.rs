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
