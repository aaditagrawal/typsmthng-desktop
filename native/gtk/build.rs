use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let mut entries = Vec::new();
    for directory in ["releases", "changelog"] {
        println!("cargo:rerun-if-changed={}", root.join(directory).display());
    }
    for entry in fs::read_dir(root.join("releases")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "md") {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            if let Some(version) = stem.strip_prefix('v') {
                entries.push((version.to_string(), path));
            }
        }
    }
    for directory in fs::read_dir(root.join("changelog")).unwrap() {
        let directory = directory.unwrap();
        if !directory.file_type().unwrap().is_dir() {
            continue;
        }
        let version = directory.file_name().into_string().unwrap();
        for entry in fs::read_dir(directory.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "md") {
                let text = fs::read_to_string(&path).unwrap();
                assert!(
                    !text.trim().is_empty()
                        && text
                            .lines()
                            .filter(|line| !line.trim().is_empty())
                            .all(|line| { line.starts_with("- ") && line.trim().len() > 4 }),
                    "{} must contain changelog bullet lines",
                    path.display()
                );
                entries.push((version.clone(), path));
            }
        }
    }
    entries.sort_by_key(|(version, path)| {
        (
            version.clone(),
            !path.starts_with(root.join("releases")),
            path.clone(),
        )
    });
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    assert!(
        entries.iter().any(|(entry, _)| entry == &version),
        "Add changelog notes for v{version}"
    );
    let mut generated = String::from("const RAW_RELEASE_NOTES: &[(&str, &str)] = &[\n");
    for (version, path) in entries {
        generated.push_str(&format!(
            "({version:?}, include_str!({:?})),\n",
            path.canonicalize().unwrap()
        ));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("release_notes.rs"),
        generated,
    )
    .unwrap();
}
