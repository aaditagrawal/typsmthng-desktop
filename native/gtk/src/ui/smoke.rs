//! Optional native-widget snapshots for repeatable smoke-test visual review.
use gtk::prelude::*;

pub fn capture_windows(_application: &gtk::Application) {
    let Some(directory) = std::env::var_os("TYPSMTHNG_SNAPSHOT_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).expect("create snapshot directory");
    let mut captured = 0;
    for (index, widget) in gtk::Window::list_toplevels().iter().enumerate() {
        let Some(window) = widget.downcast_ref::<gtk::Window>() else {
            continue;
        };
        if !window.is_visible() {
            continue;
        }
        let transient = window.transient_for().is_some();
        if transient && (!window.is_mapped() || window.width() <= 0 || window.height() <= 0) {
            continue;
        }
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        let Some(node) = snapshot.to_node() else {
            if transient {
                eprintln!("TYPESMTHNG_SNAPSHOT_SKIPPED non-drawable transient window {index}");
                continue;
            }
            panic!("requested snapshot for window {index} has no drawable content");
        };
        let renderer = window.renderer().expect("window renderer");
        let texture = renderer.render_texture(&node, None);
        let path = directory.join(format!("window-{index}.png"));
        texture.save_to_png(&path).expect("save window snapshot");
        captured += 1;
        println!("TYPESMTHNG_SNAPSHOT {}", path.display());
    }
    assert!(
        captured > 0,
        "requested snapshots but no drawable window was captured"
    );
}
