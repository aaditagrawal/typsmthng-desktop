#![cfg_attr(
    all(target_os = "windows", not(debug_assertions), not(feature = "console")),
    windows_subsystem = "windows"
)]

mod ui;

use std::path::PathBuf;
use std::process::Command;

fn main() -> glib::ExitCode {
    configure_packaged_runtime();
    let mut options = ui::LaunchOptions::default();
    for argument in std::env::args_os().skip(1) {
        if argument == "--smoke-test" {
            options.smoke_test = true;
        } else if argument == "--interaction-smoke-test" {
            options.interaction_smoke_test = true;
        } else if argument == "--presentation-smoke-test" {
            options.presentation_smoke_test = true;
        } else if argument != "--" && options.startup_path.is_none() {
            options.startup_path = Some(PathBuf::from(argument));
        }
    }
    ui::launch(options)
}

fn configure_packaged_runtime() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(binary_dir) = executable.parent() else {
        return;
    };
    let resource_root = if cfg!(target_os = "macos") {
        binary_dir.join("../Resources")
    } else {
        binary_dir.to_path_buf()
    };
    let share = if cfg!(target_os = "macos") {
        resource_root.join("share")
    } else {
        binary_dir.join("share")
    };
    let share = if share.is_dir() {
        share
    } else {
        binary_dir.join("../share")
    };
    if share.is_dir() {
        prepend_environment_path("XDG_DATA_DIRS", &share);
        let schemas = share.join("glib-2.0/schemas");
        if schemas.is_dir() {
            std::env::set_var("GSETTINGS_SCHEMA_DIR", schemas);
        }
    }

    let lib = if cfg!(target_os = "macos") {
        resource_root.join("lib")
    } else if binary_dir.join("lib").is_dir() {
        binary_dir.join("lib")
    } else {
        binary_dir.join("../lib")
    };
    let gio_modules = lib.join("gio/modules");
    if gio_modules.is_dir() {
        std::env::set_var("GIO_MODULE_DIR", gio_modules);
    }
    let query_name = if cfg!(windows) {
        "gdk-pixbuf-query-loaders.exe"
    } else {
        "gdk-pixbuf-query-loaders"
    };
    let query = binary_dir.join(query_name);
    let Some(loaders) = find_directory_named(&lib, "loaders", 4) else {
        return;
    };
    if !query.is_file() {
        return;
    }
    let Ok(output) = Command::new(&query)
        .env("GDK_PIXBUF_MODULEDIR", &loaders)
        .output()
    else {
        return;
    };
    if !output.status.success() || output.stdout.is_empty() {
        return;
    }
    let cache = std::env::temp_dir().join(format!(
        "typsmthng-gdk-pixbuf-loaders-{}.cache",
        std::process::id()
    ));
    if std::fs::write(&cache, output.stdout).is_ok() {
        std::env::set_var("GDK_PIXBUF_MODULEDIR", loaders);
        std::env::set_var("GDK_PIXBUF_MODULE_FILE", cache);
    }
}

fn prepend_environment_path(name: &str, path: &std::path::Path) {
    let mut paths = vec![path.to_path_buf()];
    if let Some(existing) = std::env::var_os(name) {
        paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var(name, joined);
    }
}

fn find_directory_named(root: &std::path::Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 || !root.is_dir() {
        return None;
    }
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|value| value == name) {
                return Some(path);
            }
            if let Some(found) = find_directory_named(&path, name, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}
