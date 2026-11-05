#![cfg(target_os = "linux")]
mod platform_paths;

use std::ffi::OsString;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

const SAMPLES: usize = 31;
const WARMUPS: usize = 5;

fn compare(name: &str, iterations: usize, mut baseline: impl FnMut(), mut candidate: impl FnMut()) {
    if std::env::var("DEPENDENCY_BENCH_FILTER").is_ok_and(|filter| !name.contains(&filter)) {
        return;
    }
    let measure = |operation: &mut dyn FnMut()| {
        let started = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        started.elapsed().as_nanos() as f64 / iterations as f64
    };
    for _ in 0..WARMUPS {
        measure(&mut baseline);
        measure(&mut candidate);
    }
    let mut before = Vec::new();
    let mut after = Vec::new();
    for sample in 0..SAMPLES {
        if sample % 2 == 0 {
            before.push(measure(&mut baseline));
            after.push(measure(&mut candidate));
        } else {
            after.push(measure(&mut candidate));
            before.push(measure(&mut baseline));
        }
    }
    before.sort_by(f64::total_cmp);
    after.sort_by(f64::total_cmp);
    println!("{name}: iterations={iterations}, warmups={WARMUPS}, alternating samples={SAMPLES}; ns/op median [p10,p90] baseline {:.2} [{:.2},{:.2}], candidate {:.2} [{:.2},{:.2}], ratio {:.3}", before[15], before[3], before[27], after[15], after[3], after[27], after[15] / before[15]);
}

// These adapters preserve the consumed expressions at the recorded baseline,
// including StateStore's historical acceptance of relative XDG_DATA_HOME.
fn baseline_state() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let app_data = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| base.home_dir().join(".local/share"));
    Some(app_data.join("dev.typsmthng.desktop").join("stable"))
}
fn candidate_state() -> Option<PathBuf> {
    platform_paths::data_dir().map(|app_data| app_data.join("dev.typsmthng.desktop").join("stable"))
}
fn baseline_fonts() -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|base| base.cache_dir().join("dev.typsmthng.desktop/google-fonts"))
}
fn candidate_fonts() -> Option<PathBuf> {
    platform_paths::google_font_cache_dir()
}
fn baseline_updates() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "typsmthng", "typsmthng")
        .map(|project| project.cache_dir().join("updates"))
}
fn candidate_updates() -> Option<PathBuf> {
    platform_paths::update_cache_dir()
}

fn scenarios(mut visit: impl FnMut(&str)) {
    use std::os::unix::ffi::OsStringExt;
    let long = format!("/home/{}", "long-user/".repeat(100));
    let non_utf8 = OsString::from_vec(b"/home/user-\xff".to_vec());
    let cases: [(&str, Option<OsString>, Option<OsString>, Option<OsString>); 8] = [
        ("default", Some("/home/alice".into()), None, None),
        (
            "xdg",
            Some("/home/alice".into()),
            Some("/cache".into()),
            Some("/data".into()),
        ),
        (
            "relative",
            Some("relative-home".into()),
            Some("relative-cache".into()),
            Some("relative-data".into()),
        ),
        ("empty", Some("".into()), Some("".into()), Some("".into())),
        ("passwd-fallback", None, None, None),
        ("long-home", Some(long.into()), None, None),
        (
            "unicode",
            Some("/home/गृह".into()),
            Some("/cache/é".into()),
            Some("/data/家".into()),
        ),
        ("non-utf8", Some(non_utf8), None, None),
    ];
    for (name, home, cache, data) in cases {
        for (key, value) in [
            ("HOME", home),
            ("XDG_CACHE_HOME", cache),
            ("XDG_DATA_HOME", data),
        ] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        visit(name);
    }
}

fn main() {
    scenarios(|name| {
        assert_eq!(baseline_state(), candidate_state());
        assert_eq!(baseline_fonts(), candidate_fonts());
        assert_eq!(baseline_updates(), candidate_updates());
        let iterations = if matches!(name, "empty" | "passwd-fallback") {
            100
        } else {
            2_000
        };
        compare(
            &format!("paths/{name}/state"),
            iterations,
            || {
                black_box(baseline_state());
            },
            || {
                black_box(candidate_state());
            },
        );
        compare(
            &format!("paths/{name}/fonts"),
            iterations,
            || {
                black_box(baseline_fonts());
            },
            || {
                black_box(candidate_fonts());
            },
        );
        compare(
            &format!("paths/{name}/updates"),
            iterations,
            || {
                black_box(baseline_updates());
            },
            || {
                black_box(candidate_updates());
            },
        );
    });
}

#[test]
fn directory_contract_matches_baseline() {
    scenarios(|_| {
        assert_eq!(baseline_state(), candidate_state());
        assert_eq!(baseline_fonts(), candidate_fonts());
        assert_eq!(baseline_updates(), candidate_updates());
    });
}
