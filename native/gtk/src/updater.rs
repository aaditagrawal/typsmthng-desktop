#[allow(dead_code)]
#[path = "backend/update_install.rs"]
mod update_install;

fn main() {
    if let Err(error) = update_install::run_helper() {
        eprintln!("Update failed: {error}");
        std::process::exit(1);
    }
}
