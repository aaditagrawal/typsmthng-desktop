//! Checks the bundled Typst grammar against GtkSourceView itself.
//!
//! Run under a display with warnings made fatal, so GtkSourceView grammar or
//! scheme errors abort the test:
//!
//! ```sh
//! dbus-run-session -- xvfb-run -a env G_DEBUG=fatal-warnings \
//!   cargo test --test typst_highlighting -- --ignored --test-threads=1
//! ```

use std::path::{Path, PathBuf};

use sourceview5::prelude::*;

fn data_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(name)
}

fn style_ids() -> Vec<String> {
    let lang = std::fs::read_to_string(data_dir("language-specs").join("typst.lang")).unwrap();
    regex::Regex::new(r#"<style id="([a-z-]+)""#)
        .unwrap()
        .captures_iter(&lang)
        .map(|captures| captures[1].to_string())
        .collect()
}

/// Encodes a style id as a unique foreground colour.
fn probe_color(index: usize) -> String {
    format!("#0000{:02X}", index + 1)
}

struct Highlighted {
    buffer: sourceview5::Buffer,
    source: String,
    ids: Vec<String>,
}

impl Highlighted {
    fn new(source: &str) -> Self {
        let ids = style_ids();

        let languages = sourceview5::LanguageManager::new();
        languages.append_search_path(data_dir("language-specs").to_str().unwrap());
        let language = languages.language("typst").expect("typst.lang loads");

        // A scheme that gives every Typst style its own colour, so the
        // effective style at an offset can be read back from the tags.
        let schemes_dir = tempfile::tempdir().unwrap();
        let styles: String = ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                format!(
                    "  <style name=\"typst:{id}\" foreground=\"{}\"/>\n",
                    probe_color(index)
                )
            })
            .collect();
        std::fs::write(
            schemes_dir.path().join("typst-probe.xml"),
            format!(
                "<?xml version=\"1.0\"?>\n<style-scheme id=\"typst-probe\" name=\"Probe\" version=\"1.0\">\n{styles}</style-scheme>\n"
            ),
        )
        .unwrap();
        let schemes = sourceview5::StyleSchemeManager::new();
        schemes.append_search_path(schemes_dir.path().to_str().unwrap());
        let scheme = schemes.scheme("typst-probe").expect("probe scheme loads");

        let buffer = sourceview5::Buffer::with_language(&language);
        buffer.set_style_scheme(Some(&scheme));
        buffer.set_highlight_syntax(true);
        buffer.set_text(source);
        let (start, end) = buffer.bounds();
        buffer.ensure_highlight(&start, &end);
        Self {
            buffer,
            source: source.to_string(),
            ids,
        }
    }

    /// The innermost Typst style at `needle[index]` (first occurrence).
    fn style_at(&self, needle: &str, index: usize) -> Option<&str> {
        let byte = self
            .source
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in sample"));
        let offset = self.source[..byte + index].chars().count() as i32;
        let iter = self.buffer.iter_at_offset(offset);
        let tag = iter
            .tags()
            .into_iter()
            .rev()
            .find(|tag| tag.is_foreground_set())?;
        let color = tag.foreground_rgba()?;
        let index = (color.blue() * 255.0).round() as usize;
        self.ids.get(index.checked_sub(1)?).map(String::as_str)
    }

    fn assert_styles(&self, cases: &[(&str, usize, Option<&str>)]) {
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|&(needle, index, expected)| {
                let actual = self.style_at(needle, index);
                (actual != expected).then(|| {
                    format!(
                        "{needle:?}[{index}] ({:?}): expected {expected:?}, got {actual:?}",
                        &needle[index..]
                    )
                })
            })
            .collect();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }
}

fn check_typst_modes() {
    let sample = r#"= Title with *bold* <intro>
== Part 1(a)
Plain *strong* and _emph_ with snake_case and 2*3 text.
See @intro, https://typst.app/docs. and \#not-code, a \
line break -- en --- em ~ nbsp ...
- item
+ enum
12. numbered
/ Term: description
`inline raw` then
```rust
let x = "no code here";
```
// line comment
/* outer /* nested */ still comment */ after
#let k = 3pt + 1.5em * 50% // trailing
#set text(size: 11pt, fill: none, hyphenate: auto)
#show heading: it => [*#it.body*]
#show: doc.with(course: "ECE \"69500\"")
#if k > 2 [yes] else [no]
Value #k and #text(red)[inline *content*]. Done.
Call #f(x).y then text.
Inline $x^2 + alpha_1 != 3 dot.op y$ and more.
$ hat(a) &= sqrt((m omega)/(2 hbar)) \
  a arrow.r.double b "text" #k $
$ mat(delim: "[", 1, 2; 3, 4) $
#box[#let inner = 1] after box and #box[a [b] *c*] tail.
"#;
    let highlighted = Highlighted::new(sample);
    highlighted.assert_styles(&[
        // Headings: marker and text are separate styles.
        ("= Title", 0, Some("heading-marker")),
        ("= Title", 2, Some("heading")),
        ("*bold*", 1, Some("strong")),
        ("<intro>", 1, Some("label")),
        ("== Part", 1, Some("heading-marker")),
        ("== Part", 3, Some("heading")),
        ("Part 1(a)", 7, Some("heading")),
        // Inline markup.
        ("Plain", 0, None),
        ("*strong*", 2, Some("strong")),
        ("_emph_", 2, Some("emph")),
        ("snake_case", 5, None),
        ("2*3 text", 1, None),
        ("@intro", 1, Some("ref")),
        ("https://typst", 9, Some("link")),
        ("docs. and", 4, None),
        ("\\#not", 1, Some("escape")),
        ("not-code", 0, None),
        ("a \\\n", 2, Some("linebreak")),
        ("-- en", 0, Some("shorthand")),
        ("--- em", 2, Some("shorthand")),
        ("~ nbsp", 0, Some("shorthand")),
        ("- item", 0, Some("list-marker")),
        ("+ enum", 0, Some("list-marker")),
        ("12. numbered", 2, Some("list-marker")),
        ("/ Term", 0, Some("list-marker")),
        ("Term:", 1, Some("term")),
        ("description", 0, None),
        ("`inline raw`", 3, Some("raw")),
        ("```rust", 0, Some("raw-delimiter")),
        ("rust\n", 0, Some("raw-lang")),
        ("let x = \"no", 0, Some("raw-block")),
        ("// line", 3, Some("comment")),
        ("still comment", 0, Some("comment")),
        ("*/ after", 3, None),
        // Code after `#`.
        ("#let k", 0, Some("keyword")),
        ("#let k", 1, Some("keyword")),
        ("3pt", 1, Some("number")),
        ("1.5em", 3, Some("number")),
        ("50%", 2, Some("number")),
        ("* 50%", 0, Some("operator")),
        ("// trailing", 2, Some("comment")),
        ("#set", 1, Some("keyword")),
        ("text(size", 0, Some("function")),
        ("size:", 0, Some("named-argument")),
        ("size:", 4, Some("punctuation")),
        ("11pt", 0, Some("number")),
        ("none,", 0, Some("constant")),
        ("auto)", 0, Some("constant")),
        ("heading: it", 0, Some("function")),
        ("=> [", 0, Some("operator")),
        ("#it.body*", 0, Some("interpolated")),
        ("#it.body*", 4, Some("interpolated")),
        ("body*]", 4, Some("strong")),
        ("doc.with", 4, Some("function")),
        ("course:", 0, Some("named-argument")),
        ("\"ECE", 0, Some("string")),
        ("\\\"69500", 0, Some("string-escape")),
        ("#if", 1, Some("keyword")),
        ("[yes]", 1, None),
        ("else [no]", 0, Some("keyword")),
        // Embedded expressions return to markup when they end.
        ("#k and", 1, Some("interpolated")),
        ("#k and", 3, None),
        ("#text(red)", 0, Some("function")),
        ("#text(red)", 1, Some("function")),
        ("*content*", 1, Some("strong")),
        (". Done", 2, None),
        ("#f(x).y", 6, Some("interpolated")),
        (" then text", 1, None),
        // Math.
        ("$x^2", 0, Some("math-delimiter")),
        ("$x^2", 1, Some("math-variable")),
        ("$x^2", 2, Some("math-attach")),
        ("$x^2", 3, Some("math-number")),
        ("+ alpha", 0, Some("math-operator")),
        ("alpha_1", 0, Some("math-symbol")),
        ("alpha_1", 5, Some("math-attach")),
        ("!= 3", 0, Some("math-operator")),
        ("dot.op", 0, Some("math-symbol")),
        ("dot.op", 4, Some("math-modifier")),
        ("y$ and", 1, Some("math-delimiter")),
        ("y$ and", 3, None),
        ("hat(a)", 0, Some("math-function")),
        ("hat(a)", 3, Some("math-group")),
        ("&=", 0, Some("math-alignment")),
        ("&=", 1, Some("math-operator")),
        ("sqrt((m", 0, Some("math-function")),
        ("hbar))", 0, Some("math-symbol")),
        ("hbar)) \\", 7, Some("math-alignment")),
        ("arrow.r.double", 0, Some("math-symbol")),
        ("arrow.r.double", 7, Some("math-modifier")),
        ("\"text\"", 1, Some("math-string")),
        ("#k $", 1, Some("interpolated")),
        ("delim:", 0, Some("math-named-argument")),
        ("\"[\"", 1, Some("math-string")),
        ("2; 3", 1, Some("punctuation")),
        // Content blocks nest and close correctly.
        ("#let inner", 1, Some("keyword")),
        ("1] after", 1, Some("punctuation")),
        ("after box", 0, None),
        ("[b] *c*", 1, None),
        ("*c*]", 1, Some("strong")),
        ("*c*]", 3, Some("punctuation")),
        (" tail.", 1, None),
    ]);
}

fn check_homework_fixture() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/homework/main.typ"),
    )
    .unwrap();
    let highlighted = Highlighted::new(&source);
    highlighted.assert_styles(&[
        ("#show: homework", 1, Some("keyword")),
        ("homework.with", 9, Some("function")),
        ("= Problem 1", 0, Some("heading-marker")),
        ("= Problem 1", 2, Some("heading")),
        ("$ [hat(a)", 2, Some("math-group")),
        ("<commutator>", 0, Some("label")),
        ("@commutator", 0, Some("ref")),
        ("sum_(n=0)^oo", 0, Some("math-symbol")),
        ("sum_(n=0)^oo", 10, Some("math-symbol")),
        ("chevron.r", 8, Some("math-modifier")),
        ("```python", 3, Some("raw-lang")),
        ("import numpy", 0, Some("raw-block")),
        ("Numerically", 0, None),
    ]);
}

/// Headings stay headings after several repetitions of the homework body
/// (fenced raw, escapes, nested block comments, `#let`, `RR^#k`, ...).
fn check_repeated_fixture_headings() {
    let fixture = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/homework/main.typ"),
    )
    .unwrap();
    let body = &fixture[fixture.find("= Problem 1").unwrap()..];
    let source: String = (1..=4)
        .map(|round| {
            body.replace("= Problem 1", &format!("= Problem {}", 2 * round - 1))
                .replace("= Problem 2", &format!("= Problem {}", 2 * round))
                .replace("Part 1(", &format!("Part {}(", 2 * round - 1))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let highlighted = Highlighted::new(&source);
    let headings: Vec<&str> = source
        .lines()
        .filter(|line| line.starts_with('='))
        .collect();
    assert_eq!(headings.len(), 16);
    let cases: Vec<(&str, usize, Option<&str>)> = headings
        .iter()
        .flat_map(|line| {
            let text = line.find(' ').unwrap() + 1;
            [
                (*line, 0, Some("heading-marker")),
                (*line, text, Some("heading")),
                (*line, line.len() - 1, Some("heading")),
            ]
        })
        .collect();
    highlighted.assert_styles(&cases);
}

/// Unterminated inline constructs stop at a paragraph break instead of
/// swallowing the rest of the document.
fn check_unclosed_constructs_do_not_leak() {
    let sample = r#"Unclosed `raw here
continues on this line

= After raw
Unclosed $x + y
still math

= After math
Escapes \$ and \* and \` stay text *bold* here.

= After escapes
Math $a < b$ and $x <y> z$ and $a<=b$ then <label> text.

= After less-than
#let s = "unclosed string
#set text(size: 1pt)

= After string
"#;
    let highlighted = Highlighted::new(sample);
    highlighted.assert_styles(&[
        ("raw here", 0, Some("raw")),
        ("continues", 0, Some("raw")),
        ("= After raw", 2, Some("heading")),
        ("still math", 0, Some("math-symbol")),
        ("= After math", 2, Some("heading")),
        ("\\$ and", 0, Some("escape")),
        ("\\* and", 1, Some("escape")),
        ("\\` stay", 1, Some("escape")),
        ("stay text", 0, None),
        ("*bold*", 1, Some("strong")),
        ("= After escapes", 2, Some("heading")),
        ("< b$", 0, Some("math-operator")),
        ("b$ and", 1, Some("math-delimiter")),
        ("<y>", 0, Some("math-operator")),
        ("<y>", 1, Some("math-variable")),
        ("<=b", 0, Some("math-operator")),
        ("<label>", 1, Some("label")),
        ("= After less-than", 2, Some("heading")),
        ("unclosed string", 0, Some("string")),
        ("= After string", 2, Some("heading")),
    ]);
}

fn check_bundled_schemes() {
    let schemes = sourceview5::StyleSchemeManager::new();
    schemes.append_search_path(data_dir("styles").to_str().unwrap());
    for (id, parent) in [
        ("typsmthng-light", "Adwaita"),
        ("typsmthng-dark", "Adwaita-dark"),
    ] {
        let scheme = schemes.scheme(id).unwrap_or_else(|| panic!("{id} loads"));
        // Inherited from the parent scheme.
        assert!(scheme.style("text").is_some(), "{id} inherits {parent}");
        let heading = scheme.style("typst:heading").expect("heading style");
        assert!(heading.is_bold());
        assert_eq!(heading.pango_underline(), gtk::pango::Underline::Single);
        let marker = scheme
            .style("typst:heading-marker")
            .expect("heading marker style");
        assert!(marker.is_underline_set());
        assert_eq!(marker.pango_underline(), gtk::pango::Underline::None);
        for style in [
            "typst:math-delimiter",
            "typst:math-operator",
            "typst:math-symbol",
            "typst:math-function",
            "typst:math-variable",
        ] {
            assert!(scheme.style(style).is_some(), "{id} defines {style}");
        }
    }
}

/// GTK may only be initialised on one thread, so all checks share one test.
#[test]
#[ignore = "requires a display; run under xvfb-run with --test-threads=1"]
fn typst_grammar_and_schemes() {
    gtk::init().expect("a display is required");
    sourceview5::init();
    check_typst_modes();
    check_homework_fixture();
    check_repeated_fixture_headings();
    check_unclosed_constructs_do_not_leak();
    check_bundled_schemes();
}
