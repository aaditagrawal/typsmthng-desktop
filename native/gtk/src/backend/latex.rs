use regex::{Captures, Regex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionWarning {
    pub construct: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversionMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub document_class: Option<String>,
    pub packages: Vec<String>,
    pub graphics_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionResult {
    pub typst: String,
    pub warnings: Vec<ConversionWarning>,
    pub metadata: ConversionMetadata,
}

pub fn convert_latex_to_typst(source: &str) -> ConversionResult {
    let source = strip_comments(source);
    let metadata = metadata(&source);
    let mut warnings = Vec::new();
    let document = Regex::new(r"(?s)\\begin\{document\}(.*?)\\end\{document\}").unwrap();
    let mut body = document
        .captures(&source)
        .map(|capture| capture[1].to_owned())
        .unwrap_or_else(|| source.clone());
    body = convert_lists(body);
    body = convert_math_environments(body);
    body = convert_sections(body);
    body = convert_commands(body);
    body = convert_math(body);
    body = body
        .replace("\\&", "&")
        .replace("\\%", "%")
        .replace("\\_", "_")
        .replace("\\#", "#")
        .replace("\\$", "$")
        .replace('~', "\u{a0}");
    collect_warnings(&body, &mut warnings);
    body = Regex::new(
        r"(?m)^[ \t]*\\(?:documentclass|usepackage|title|author|date|graphicspath)(?:\[[^]]*\])?\{[^}]*\}[ \t]*\n?",
    )
    .unwrap()
    .replace_all(&body, "")
    .into_owned();
    body = Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(body.trim(), "\n\n")
        .into_owned();
    let mut header = Vec::new();
    if let Some(title) = &metadata.title {
        header.push(format!(
            "#set document(title: \"{}\")",
            escape_string(title)
        ));
    }
    if let Some(author) = &metadata.author {
        header.push(format!(
            "#set document(author: \"{}\")",
            escape_string(author)
        ));
    }
    if let Some(date) = metadata.date.as_deref().and_then(typst_date) {
        header.push(format!("#set document(date: {date})"));
    }
    let typst = if header.is_empty() {
        format!("{body}\n")
    } else {
        format!("{}\n\n{body}\n", header.join("\n"))
    };
    ConversionResult {
        typst,
        warnings,
        metadata,
    }
}

fn strip_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let mut slash_count = 0;
            for (offset, character) in line.char_indices() {
                if character == '%' && slash_count % 2 == 0 {
                    return &line[..offset];
                }
                slash_count = if character == '\\' {
                    slash_count + 1
                } else {
                    0
                };
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn metadata(source: &str) -> ConversionMetadata {
    let capture = |name: &str| {
        Regex::new(&format!(r"(?s)\\{}\{{([^}}]*)\}}", name))
            .ok()?
            .captures(source)
            .map(|capture| clean_text(&capture[1]))
    };
    let packages = Regex::new(r"\\usepackage(?:\[[^]]*\])?\{([^}]*)\}")
        .unwrap()
        .captures_iter(source)
        .flat_map(|capture| {
            capture[1]
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let graphics_paths = Regex::new(r"\\graphicspath\{((?:\{[^}]*\})+)\}")
        .unwrap()
        .captures(source)
        .map(|capture| {
            Regex::new(r"\{([^}]*)\}")
                .unwrap()
                .captures_iter(&capture[1])
                .map(|path| path[1].replace('\\', "/"))
                .collect()
        })
        .unwrap_or_default();
    ConversionMetadata {
        title: capture("title"),
        author: capture("author"),
        date: capture("date"),
        document_class: capture("documentclass"),
        packages,
        graphics_paths,
    }
}

fn convert_sections(mut text: String) -> String {
    for (name, marks) in [
        ("part", "="),
        ("chapter", "="),
        ("section", "="),
        ("subsection", "=="),
        ("subsubsection", "==="),
        ("paragraph", "===="),
        ("subparagraph", "====="),
    ] {
        let pattern = Regex::new(&format!(r"\\{}\*?\{{([^}}]*)\}}", name)).unwrap();
        text = pattern
            .replace_all(&text, |capture: &Captures| {
                format!("\n{marks} {}\n", clean_text(&capture[1]))
            })
            .into_owned();
    }
    text
}

fn convert_lists(mut text: String) -> String {
    for (environment, marker) in [("itemize", "-"), ("enumerate", "+")] {
        let pattern = Regex::new(&format!(
            r"(?s)\\begin\{{{environment}\}}(.*?)\\end\{{{environment}\}}"
        ))
        .unwrap();
        while pattern.is_match(&text) {
            text = pattern
                .replace_all(&text, |capture: &Captures| {
                    capture[1]
                        .split("\\item")
                        .skip(1)
                        .map(|item| format!("{marker} {}", item.trim()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .into_owned();
        }
    }
    text
}

fn convert_math_environments(mut text: String) -> String {
    for environment in [
        "equation",
        "equation*",
        "align",
        "align*",
        "gather",
        "gather*",
        "multline",
        "multline*",
    ] {
        let escaped = regex::escape(environment);
        let pattern = Regex::new(&format!(
            r"(?s)\\begin\{{{escaped}\}}(.*?)\\end\{{{escaped}\}}"
        ))
        .unwrap();
        text = pattern
            .replace_all(&text, |capture: &Captures| {
                format!("\n$ {} $\n", math_tokens(&capture[1]))
            })
            .into_owned();
    }
    text
}

fn convert_commands(mut text: String) -> String {
    text = Regex::new(r"\\(?:input|include)\{([^}]*)\}")
        .unwrap()
        .replace_all(&text, |capture: &Captures| {
            let mut path = capture[1].trim().replace('\\', "/");
            if path.to_ascii_lowercase().ends_with(".tex") {
                path.truncate(path.len() - 4);
            }
            if !path.to_ascii_lowercase().ends_with(".typ") {
                path.push_str(".typ");
            }
            format!("#include \"{}\"", escape_string(&path))
        })
        .into_owned();
    for _ in 0..8 {
        let old = text.clone();
        text = replace_one(&text, "textbf", |value| format!("*{value}*"));
        text = replace_one(&text, "textit", |value| format!("_{value}_"));
        text = replace_one(&text, "emph", |value| format!("_{value}_"));
        text = replace_one(&text, "texttt", |value| format!("#raw[{value}]"));
        text = replace_one(&text, "textsc", |value| format!("#smallcaps[{value}]"));
        text = replace_one(&text, "footnote", |value| format!("#footnote[{value}]"));
        text = replace_one(&text, "label", |value| format!(" <{value}>"));
        text = replace_one(&text, "ref", |value| format!("@{value}"));
        text = replace_one(&text, "eqref", |value| format!("@{value}"));
        text = replace_one(&text, "cite", |value| {
            value
                .split(',')
                .map(|key| format!("@{}", key.trim()))
                .collect::<Vec<_>>()
                .join(" ")
        });
        text = replace_one(&text, "url", |value| {
            format!("#link(\"{}\")", escape_string(value))
        });
        text = replace_two(&text, "href", |url, label| {
            format!("#link(\"{}\")[{label}]", escape_string(url))
        });
        text = convert_images(&text);
        if text == old {
            break;
        }
    }
    for (from, to) in [
        ("\\maketitle", "#align(center)[#text(size: 1.5em, weight: \"bold\")[#document.title]\n#document.author]"),
        ("\\tableofcontents", "#outline()"),
        ("\\newpage", "#pagebreak()"),
        ("\\clearpage", "#pagebreak()"),
        ("\\bigskip", "#v(1em)"),
        ("\\medskip", "#v(0.5em)"),
        ("\\smallskip", "#v(0.25em)"),
        ("\\hfill", "#h(1fr)"),
        ("\\today", "#datetime.today().display()"),
        ("\\LaTeX", "LaTeX"),
        ("\\TeX", "TeX"),
        ("\\noindent", ""),
        ("\\centering", ""),
    ] {
        text = text.replace(from, to);
    }
    text
}

fn convert_images(text: &str) -> String {
    Regex::new(r"\\includegraphics(?:\[([^]]*)\])?\{([^}]*)\}")
        .unwrap()
        .replace_all(text, |capture: &Captures| {
            let options = capture
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let width = Regex::new(r"width\s*=\s*([0-9.]+)\\textwidth")
                .unwrap()
                .captures(options)
                .map(|value| {
                    format!(
                        ", width: {}%",
                        value[1].parse::<f64>().unwrap_or(1.0) * 100.0
                    )
                })
                .unwrap_or_default();
            format!(
                "#image(\"{}\"{width})",
                escape_string(&capture[2].replace('\\', "/"))
            )
        })
        .into_owned()
}

fn convert_math(mut text: String) -> String {
    text = Regex::new(r"(?s)\\\[(.*?)\\\]")
        .unwrap()
        .replace_all(&text, |capture: &Captures| {
            format!("$ {} $", math_tokens(&capture[1]))
        })
        .into_owned();
    Regex::new(r"(?s)(^|[^\\])\$([^$\n]+)\$")
        .unwrap()
        .replace_all(&text, |capture: &Captures| {
            format!("{}$ {} $", &capture[1], math_tokens(&capture[2]))
        })
        .into_owned()
}

fn math_tokens(input: &str) -> String {
    let mut output = input.to_owned();
    for (from, to) in [
        ("\\frac", "frac"),
        ("\\sqrt", "sqrt"),
        ("\\sum", "sum"),
        ("\\prod", "product"),
        ("\\int", "integral"),
        ("\\infty", "infinity"),
        ("\\leq", "<="),
        ("\\geq", ">="),
        ("\\neq", "!="),
        ("\\cdot", " dot "),
        ("\\times", " times "),
        ("\\rightarrow", " arrow.r "),
        ("\\leftarrow", " arrow.l "),
        ("\\to", " arrow.r "),
    ] {
        output = output.replace(from, to);
    }
    for name in [
        "alpha", "beta", "gamma", "delta", "epsilon", "theta", "lambda", "mu", "pi", "rho",
        "sigma", "phi", "psi", "omega", "Gamma", "Delta", "Theta", "Lambda", "Sigma", "Phi", "Psi",
        "Omega",
    ] {
        output = output.replace(&format!("\\{name}"), name);
    }
    output
        .replace('&', "")
        .replace("\\\\", " \\ ")
        .trim()
        .into()
}

fn replace_one(text: &str, name: &str, render: impl Fn(&str) -> String) -> String {
    replace_balanced(text, name, 1, |args| render(&args[0]))
}

fn replace_two(text: &str, name: &str, render: impl Fn(&str, &str) -> String) -> String {
    replace_balanced(text, name, 2, |args| render(&args[0], &args[1]))
}

fn replace_balanced(
    text: &str,
    name: &str,
    count: usize,
    render: impl Fn(&[String]) -> String,
) -> String {
    let marker = format!("\\{name}");
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find(&marker) {
        let start = cursor + offset;
        output.push_str(&text[cursor..start]);
        let mut end = start + marker.len();
        let mut args = Vec::new();
        for _ in 0..count {
            while text[end..].chars().next().is_some_and(char::is_whitespace) {
                end += text[end..].chars().next().unwrap().len_utf8();
            }
            let Some((argument, next)) = balanced_argument(text, end) else {
                args.clear();
                break;
            };
            args.push(argument);
            end = next;
        }
        if args.len() == count {
            output.push_str(&render(&args));
            cursor = end;
        } else {
            output.push_str(&marker);
            cursor = start + marker.len();
        }
    }
    output.push_str(&text[cursor..]);
    output
}

fn balanced_argument(text: &str, start: usize) -> Option<(String, usize)> {
    if !text.get(start..)?.starts_with('{') {
        return None;
    }
    let mut depth = 0;
    for (offset, character) in text[start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[start + 1..start + offset].into(), start + offset + 1));
                }
            }
            _ => {}
        }
    }
    None
}

fn collect_warnings(text: &str, warnings: &mut Vec<ConversionWarning>) {
    let mut constructs = Regex::new(r"\\([A-Za-z@]+)")
        .unwrap()
        .captures_iter(text)
        .map(|capture| capture[1].to_owned())
        .collect::<Vec<_>>();
    constructs.sort();
    constructs.dedup();
    for construct in constructs {
        warnings.push(ConversionWarning {
            message: format!("\\{construct} needs manual conversion."),
            construct: format!("command: \\{construct}"),
        });
    }
    let environment = Regex::new(r"\\begin\{([^}]+)\}").unwrap();
    for capture in environment.captures_iter(text) {
        warnings.push(ConversionWarning {
            message: format!("The {} environment needs manual conversion.", &capture[1]),
            construct: format!("environment: {}", &capture[1]),
        });
    }
}

fn clean_text(value: &str) -> String {
    value.replace("\\&", "&").replace("\\%", "%").trim().into()
}

fn escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn typst_date(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || matches!(value, "\\today" | "today") {
        return Some("auto".into());
    }
    let capture = Regex::new(r"^(\d{4})-(\d{1,2})-(\d{1,2})$")
        .unwrap()
        .captures(value)?;
    Some(format!(
        "datetime(year: {}, month: {}, day: {})",
        &capture[1], &capture[2], &capture[3]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_document_features() {
        let source = r#"\documentclass{article}
\usepackage{graphicx,amsmath}
\title{A Paper}
\author{Ada}
\date{2026-09-05}
\begin{document}
\maketitle
\section{Results}
This is \textbf{bold} with \href{https://example.com}{a link} and $x \leq y$.
\begin{itemize}
\item First
\item Second
\end{itemize}
\includegraphics[width=0.5\textwidth]{figures/result.png}
\end{document}"#;
        let result = convert_latex_to_typst(source);
        assert!(result.typst.contains("#set document(title: \"A Paper\")"));
        assert!(result.typst.contains("= Results"));
        assert!(result.typst.contains("*bold*"));
        assert!(result.typst.contains("- First"));
        assert!(result.typst.contains("width: 50%"));
        assert!(result.typst.contains("$ x <= y $"));
        assert_eq!(result.metadata.packages, ["graphicx", "amsmath"]);
    }

    #[test]
    fn handles_nested_commands() {
        let result = convert_latex_to_typst(r"\textbf{outer \emph{inner}}");
        assert!(result.typst.contains("*outer _inner_*"));
    }

    #[test]
    fn warns_about_unknown_commands() {
        let result = convert_latex_to_typst(r"Hello \custom{world}");
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.construct == "command: \\custom"));
    }
}
