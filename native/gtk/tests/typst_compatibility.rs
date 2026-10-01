use std::fs;
use std::path::Path;
use std::process::Command;

use typsmthng_gtk::backend::preview::PreviewCompiler;
use typsmthng_gtk::backend::{CompileOptions, Project, TypstTool};

const CHILD_TEST: &str = "TYPSMTHNG_COMPATIBILITY_TEST";
const FIXTURE_ROOT: &str = "TYPSMTHNG_COMPATIBILITY_ROOT";

/// Environment cases run in their own processes. No test mutates the global
/// environment while another test or Typst worker could be reading it.
fn isolated(name: &str, prepare: impl FnOnce(&Path, &mut Command), check: impl FnOnce(&Path)) {
    if std::env::var(CHILD_TEST).as_deref() == Ok(name) {
        let root = std::env::var_os(FIXTURE_ROOT).expect("isolated test fixture root");
        check(Path::new(&root));
        return;
    }

    let tool = TypstTool::detect().expect("install Typst 0.15.1 or set TYPSMTHNG_TYPST");
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env(CHILD_TEST, name)
        .env(FIXTURE_ROOT, directory.path())
        .env("TYPSMTHNG_TYPST", tool.executable());
    for variable in [
        "TYPST_FONT_PATHS",
        "TYPST_IGNORE_SYSTEM_FONTS",
        "TYPST_IGNORE_EMBEDDED_FONTS",
        "TYPST_PACKAGE_PATH",
        "TYPST_PACKAGE_CACHE_PATH",
        "SOURCE_DATE_EPOCH",
        "TYPST_ROOT",
        "TYPST_FEATURES",
    ] {
        command.env_remove(variable);
    }
    prepare(directory.path(), &mut command);
    let output = command.output().expect("run isolated compatibility test");
    assert!(
        output.status.success(),
        "{name} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed;"),
        "{name} did not execute exactly one isolated test"
    );
}

fn assert_matches_cli(project: &Project, options: &CompileOptions) {
    let compiler = PreviewCompiler::default();
    let preview = compiler.compile(project, "main.typ", options).unwrap();
    let tool = TypstTool::detect().expect("Typst 0.15.1 required for compatibility tests");
    let svg = tool
        .compile_svg_with_options(project, "main.typ", options)
        .unwrap();
    let pdf = tool
        .compile_pdf_with_options(project, "main.typ", options)
        .unwrap();
    assert!(preview.success(), "preview: {}", preview.stderr);
    assert!(svg.success(), "CLI SVG: {}", svg.stderr);
    assert!(pdf.success(), "CLI PDF: {}", pdf.stderr);
    // These fixtures need no fallback fonts or other warnings. A successful
    // compilation alone would miss the original font-path mismatch.
    assert!(preview.diagnostics.is_empty(), "{}", preview.stderr);
    assert!(svg.diagnostics.is_empty(), "{}", svg.stderr);
    assert!(pdf.diagnostics.is_empty(), "{}", pdf.stderr);
    let preview = preview.artifact.unwrap();
    let svg = svg.artifact.unwrap();
    assert_eq!(preview.pages.len(), svg.len());
    for (preview, cli) in preview.pages.iter().zip(&svg) {
        assert_eq!(preview.svg, cli.svg, "page {} SVG differs", cli.page);
        for (actual, expected) in [
            (preview.width_points, cli.width_points),
            (preview.height_points, cli.height_points),
        ] {
            // CLI SVG headers round point dimensions to two decimal places.
            assert!((actual.unwrap() - expected.unwrap()).abs() < 0.01);
        }
    }
    assert!(pdf.artifact.unwrap().starts_with(b"%PDF"));
    let again = compiler.compile(project, "main.typ", options).unwrap();
    assert_eq!(preview.pages, again.artifact.unwrap().pages);
}

fn deterministic_options() -> CompileOptions {
    CompileOptions {
        ignore_system_fonts: true,
        creation_timestamp: Some(1_704_067_200),
        inherit_environment: false,
        ..Default::default()
    }
}

#[test]
fn document_features_match_preview_svg_and_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join("chapters")).unwrap();
    for (name, contents) in [
        ("chapters/body.typ", "#include \"../part.typ\"\n#read(\"/data.txt\")"),
        ("part.typ", "Imported text"),
        ("data.txt", "Root data"),
        ("data.json", "{\"title\":\"JSON\"}"),
        ("data.csv", "A,B\n1,2"),
        ("data.yaml", "title: YAML"),
        ("data.toml", "title = \"TOML\""),
        ("icon.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><rect width=\"20\" height=\"20\" fill=\"red\"/></svg>"),
        ("refs.bib", "@article{doe, author={Doe, Jane}, title={An example}, journal={Journal}, year={2024}}"),
    ] {
        fs::write(root.join(name), contents).unwrap();
    }
    fs::write(root.join("main.typ"), "").unwrap();
    let project = Project::open(root).unwrap();
    for (name, source) in [
        ("markup", "= Heading\n*Bold* _emphasis_ `raw` \\ line\n- Item\n+ Numbered\n/ Term: Definition\n#sym.alpha"),
        ("scripting", "#let greet(name) = [Hello #name]\n#show strong: it => text(fill: red, it.body)\n#greet(\"World\") *bold*\n#for n in range(4) [#n ]\n#assert.eq((1, 2).map(x => x * 2), (2, 4))"),
        ("math", "$ integral_0^infinity e^(-x^2) dif x = sqrt(pi)/2 $\n$ mat(1, 2; 3, 4) quad cases(x & \"positive\", -x & \"negative\") $"),
        ("layout", "#set page(width: 200pt, height: 160pt)\n#columns(2)[One #colbreak() Two]\n#pagebreak()\n#table(columns: 2, [A], [B], [C], [D])"),
        ("imports", "#include \"chapters/body.typ\""),
        ("data", "#json(\"data.json\").title\n#csv(\"data.csv\").flatten().join(\",\")\n#yaml(\"data.yaml\").title\n#toml(\"data.toml\").title"),
        ("images", "#image(\"icon.svg\", width: 30pt, alt: \"Square\")\n#rect(width: 40pt, height: 30pt, fill: gradient.linear(red, blue))"),
        ("bibliography", "Citation @doe\n#bibliography(\"refs.bib\")"),
        ("introspection", "#set heading(numbering: \"1.\")\n#outline()\n= Introduction <intro>\nSee @intro\n#context [Page #counter(page).display()]\n#metadata((page: 1, text: \"note\")) <typsmthng-note>"),
        ("unicode", "Café Ελληνικά\n#text(lang: \"ar\", dir: rtl)[مرحبا]"),
        ("date", "#assert.eq(datetime.today(offset: 0).year(), 2024)\n#datetime.today(offset: 0).display()"),
    ] {
        eprintln!("Checking {name}");
        fs::write(root.join("main.typ"), source).unwrap();
        assert_matches_cli(&project, &deterministic_options());
    }
    assert_matches_cli(
        &project,
        &CompileOptions {
            page_preamble: Some("#set page(paper: \"a5\")".into()),
            ..deterministic_options()
        },
    );
}

fn write_package(root: &Path) {
    let package = root.join("local/compatibility-fixture/0.1.0");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("typst.toml"),
        "[package]\nname=\"compatibility-fixture\"\nversion=\"0.1.0\"\nentrypoint=\"lib.typ\"\n",
    )
    .unwrap();
    fs::write(package.join("lib.typ"), "#let greeting = [Local package]").unwrap();
}

fn write_unique_font(destination: &Path) -> String {
    let (font, info) = typst_kit::fonts::embedded().next().unwrap();
    let family = format!("Test{}", "F".repeat(info.family.chars().count() - 4));
    let original = info
        .family
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect::<Vec<_>>();
    let replacement = family
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect::<Vec<_>>();
    assert_eq!(original.len(), replacement.len());
    let mut bytes = font.data().as_slice().to_vec();
    // Change the UTF-16 family names without changing name-table offsets.
    // An unmodified embedded font would let a broken preview pass by using
    // its embedded copy instead of actually loading the environment path.
    for offset in 0..=bytes.len() - original.len() {
        if bytes[offset..].starts_with(&original) {
            bytes[offset..offset + original.len()].copy_from_slice(&replacement);
        }
    }
    let fixture_font =
        typst::text::Font::new(typst::foundations::Bytes::new(bytes.clone()), font.index())
            .expect("renamed fixture font remains valid");
    assert_eq!(fixture_font.info().family.as_str(), family);
    fs::write(destination, bytes).unwrap();
    family
}

fn package_case(name: &str, variable: &str) {
    isolated(
        name,
        |root, command| {
            let packages = root.join("package directory");
            write_package(&packages);
            fs::write(
                root.join("main.typ"),
                "#import \"@local/compatibility-fixture:0.1.0\": greeting\n#greeting\n#metadata((page: 1, text: \"Package note\")) <typsmthng-note>",
            )
            .unwrap();
            command.env(variable, packages);
        },
        |root| {
            let project = Project::open(root).unwrap();
            let options = CompileOptions {
                inherit_environment: true,
                ..deterministic_options()
            };
            assert_matches_cli(&project, &options);
            let notes = TypstTool::detect()
                .unwrap()
                .query_notes(&project, "main.typ", &options)
                .unwrap();
            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0].text, "Package note");
        },
    );
}

#[test]
fn environment_package_path_matches_preview_and_exports() {
    package_case(
        "environment_package_path_matches_preview_and_exports",
        "TYPST_PACKAGE_PATH",
    );
}

#[test]
fn environment_package_cache_matches_preview_and_exports() {
    package_case(
        "environment_package_cache_matches_preview_and_exports",
        "TYPST_PACKAGE_CACHE_PATH",
    );
}

#[test]
fn environment_font_paths_and_ignore_flags_match_preview_and_exports() {
    isolated(
        "environment_font_paths_and_ignore_flags_match_preview_and_exports",
        |root, command| {
            let fonts = root.join("font directory");
            let empty_fonts = root.join("empty directory");
            fs::create_dir_all(&fonts).unwrap();
            fs::create_dir_all(&empty_fonts).unwrap();
            let family = write_unique_font(&fonts.join("fixture.otf"));
            fs::write(
                root.join("main.typ"),
                format!("#set text(font: {family:?}, fallback: false)\nHello 123"),
            )
            .unwrap();
            command
                .env(
                    "TYPST_FONT_PATHS",
                    std::env::join_paths([empty_fonts, fonts]).unwrap(),
                )
                .env("TYPST_IGNORE_SYSTEM_FONTS", "true")
                .env("TYPST_IGNORE_EMBEDDED_FONTS", "true");
        },
        |root| {
            let project = Project::open(root).unwrap();
            let options = CompileOptions {
                creation_timestamp: Some(1_704_067_200),
                ..Default::default()
            };
            assert_matches_cli(&project, &options);
            // The app resolves environment fonts before adding Google Fonts'
            // cache directory. Extra paths must preserve the inherited fonts.
            let mut resolved = options.resolved().unwrap();
            resolved.font_paths.push(root.join("extra font cache"));
            assert_matches_cli(&project, &resolved);

            // Typst succeeds with blank output when no fonts are available.
            // Compare that output with the CLI, then restore embedded fonts
            // and verify that text returns, including on a persistent worker.
            resolved.font_paths.clear();
            fs::write(root.join("main.typ"), "Hello without any fonts").unwrap();
            assert_matches_cli(&project, &resolved);
            let compiler = PreviewCompiler::default();
            let without_fonts = compiler
                .compile(&project, "main.typ", &resolved)
                .unwrap()
                .artifact
                .unwrap();
            resolved.ignore_embedded_fonts = false;
            assert_matches_cli(&project, &resolved);
            let with_fonts = compiler
                .compile(&project, "main.typ", &resolved)
                .unwrap()
                .artifact
                .unwrap();
            assert_ne!(without_fonts.pages[0].svg, with_fonts.pages[0].svg);
        },
    );
}

#[test]
fn source_date_epoch_matches_preview_and_exports() {
    isolated(
        "source_date_epoch_matches_preview_and_exports",
        |root, command| {
            fs::write(
                root.join("main.typ"),
                "#assert.eq(datetime.today(offset: 0).year(), 2000)\n#datetime.today(offset: 0).display()",
            )
            .unwrap();
            command.env("SOURCE_DATE_EPOCH", "946684800");
        },
        |root| {
            assert_matches_cli(
                &Project::open(root).unwrap(),
                &CompileOptions {
                    creation_timestamp: None,
                    inherit_environment: true,
                    ..deterministic_options()
                },
            );
        },
    );
}

#[test]
fn explicit_options_override_environment_for_preview_and_exports() {
    isolated(
        "explicit_options_override_environment_for_preview_and_exports",
        |root, command| {
            let packages = root.join("explicit packages");
            write_package(&packages);
            fs::write(
                root.join("main.typ"),
                "#import \"@local/compatibility-fixture:0.1.0\": greeting\n#greeting\n#assert.eq(datetime.today(offset: 0).year(), 1969)",
            )
            .unwrap();
            command
                .env("TYPST_PACKAGE_PATH", root.join("missing packages"))
                .env("SOURCE_DATE_EPOCH", "not a timestamp");
        },
        |root| {
            assert_matches_cli(
                &Project::open(root).unwrap(),
                &CompileOptions {
                    package_path: Some(root.join("explicit packages")),
                    creation_timestamp: Some(-1),
                    inherit_environment: true,
                    ..deterministic_options()
                },
            );
        },
    );
}

#[test]
fn frozen_options_do_not_leak_cli_environment() {
    isolated(
        "frozen_options_do_not_leak_cli_environment",
        |root, command| {
            fs::write(
                root.join("main.typ"),
                "#assert.eq(datetime.today(offset: 0).year(), 2024)\nHello",
            )
            .unwrap();
            command
                .env("SOURCE_DATE_EPOCH", "invalid")
                .env("TYPST_IGNORE_EMBEDDED_FONTS", "invalid")
                .env("TYPST_FONT_PATHS", root.join("missing fonts"));
        },
        |root| assert_matches_cli(&Project::open(root).unwrap(), &deterministic_options()),
    );
}

#[test]
fn invalid_environment_is_reported_by_both_compilation_paths() {
    isolated(
        "invalid_environment_is_reported_by_both_compilation_paths",
        |root, command| {
            fs::write(root.join("main.typ"), "Hello").unwrap();
            command.env("SOURCE_DATE_EPOCH", "not a timestamp");
        },
        |root| {
            let project = Project::open(root).unwrap();
            let options = CompileOptions::default();
            let preview_error = PreviewCompiler::default()
                .compile(&project, "main.typ", &options)
                .err()
                .expect("invalid preview configuration")
                .to_string();
            let cli_error = TypstTool::detect()
                .unwrap()
                .compile_pdf_with_options(&project, "main.typ", &options)
                .unwrap_err()
                .to_string();
            assert_eq!(preview_error, cli_error);
            assert!(preview_error.contains("SOURCE_DATE_EPOCH"));
        },
    );
}

#[test]
fn invalid_documents_have_matching_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("main.typ"), "").unwrap();
    let project = Project::open(directory.path()).unwrap();
    let tool = TypstTool::detect().expect("Typst 0.15.1 required for diagnostic tests");
    let compiler = PreviewCompiler::default();
    for source in ["#missing-symbol", "#include \"missing.typ\""] {
        fs::write(directory.path().join("main.typ"), source).unwrap();
        let preview = compiler
            .compile(&project, "main.typ", &deterministic_options())
            .unwrap();
        let cli = tool
            .compile_svg_with_options(&project, "main.typ", &deterministic_options())
            .unwrap();
        assert!(!preview.success());
        assert!(!cli.success());
        assert_eq!(preview.diagnostics.len(), cli.diagnostics.len());
        for (preview, cli) in preview.diagnostics.iter().zip(&cli.diagnostics) {
            assert_eq!(preview.severity, cli.severity);
            assert_eq!(preview.path, cli.path);
            assert_eq!(preview.line, cli.line);
            assert_eq!(preview.column, cli.column);
            assert_eq!(preview.message, cli.message);
        }
    }
}
