//! Editor-only requests and UTF-8/snippet handling shared with the native UI.
use super::{BackendError, Project, Result};
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;
use typst::syntax::{LinkedNode, Side, Source};

#[derive(Debug, Clone)]
pub enum EditorQuery {
    Complete,
    Hover,
    Format { anchor: usize },
}

#[derive(Debug, Clone)]
pub struct EditorRequest {
    pub path: String,
    pub text: String,
    /// Typst uses UTF-8 bytes; GTK uses Unicode characters.
    pub cursor: usize,
    pub query: EditorQuery,
}

#[derive(Debug)]
pub enum EditorResponse {
    Completions {
        from: usize,
        items: Vec<typst_ide::Completion>,
    },
    Hover(Option<String>),
    Formatted {
        text: String,
        cursor: usize,
        anchor: usize,
    },
}

pub fn format_source(source: Source, cursor: usize, anchor: usize) -> Result<EditorResponse> {
    if !source.text().is_char_boundary(cursor) || !source.text().is_char_boundary(anchor) {
        return Err(BackendError::Process(
            "Invalid selection byte offset".into(),
        ));
    }
    let formatter = typstyle_core::Typstyle::new(typstyle_core::Config {
        reorder_import_items: false,
        ..Default::default()
    });
    let text = formatter
        .format_source(source.clone())
        .render()
        .map_err(|error| BackendError::Process(error.to_string()))?;
    let formatted = Source::detached(text.clone());
    let mapped_cursor = if cursor == anchor && cursor == source.text().len() {
        text.len()
    } else {
        formatted_offset(
            &source,
            &formatted,
            cursor,
            if cursor >= anchor {
                Side::Before
            } else {
                Side::After
            },
        )
    };
    Ok(EditorResponse::Formatted {
        cursor: mapped_cursor,
        anchor: if cursor == anchor {
            mapped_cursor
        } else {
            formatted_offset(
                &source,
                &formatted,
                anchor,
                if anchor <= cursor {
                    Side::After
                } else {
                    Side::Before
                },
            )
        },
        text,
    })
}

/// Keep the caret inside the same token occurrence when whitespace changes.
/// For whitespace/deleted punctuation, retain the bounded line and character column.
fn formatted_offset(source: &Source, formatted: &Source, offset: usize, side: Side) -> usize {
    fn matches(node: &LinkedNode<'_>, target: &LinkedNode<'_>, positions: &mut Vec<usize>) {
        if node.kind() == target.kind() && node.leaf_text() == target.leaf_text() {
            positions.push(node.offset());
        }
        for child in node.children() {
            matches(&child, target, positions);
        }
    }
    if let Some(leaf) = LinkedNode::new(source.root()).leaf_at(offset, side) {
        if !leaf.kind().is_trivia() {
            let mut old = Vec::new();
            let mut new = Vec::new();
            matches(&LinkedNode::new(source.root()), &leaf, &mut old);
            matches(&LinkedNode::new(formatted.root()), &leaf, &mut new);
            if let Some(start) = old
                .iter()
                .position(|start| *start == leaf.offset())
                .and_then(|index| new.get(index))
            {
                return start + offset.saturating_sub(leaf.offset()).min(leaf.len());
            }
        }
    }
    let prefix = source.text().get(..offset).unwrap_or_default();
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count();
    let start = formatted
        .lines()
        .line_to_byte(line)
        .unwrap_or(formatted.text().len());
    let text = formatted.text()[start..]
        .split('\n')
        .next()
        .unwrap_or_default();
    start + char_to_byte(text, column.min(text.chars().count())).unwrap_or(text.len())
}

pub fn char_to_byte(text: &str, offset: usize) -> Option<usize> {
    if offset == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(offset).map(|(byte, _)| byte)
}

pub fn byte_to_char(text: &str, offset: usize) -> Option<usize> {
    text.get(..offset).map(|prefix| prefix.chars().count())
}

/// A cheap trigger only. Typst's semantic completer checks the actual syntax and
/// supplies safe, project-relative paths on its background worker.
pub fn may_complete_path(line_before_cursor: &str) -> bool {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(
                r#"(?:^|[^\p{L}\p{N}_-])(?:(?:image|read|csv|json|yaml|xml|bibliography|cbor|plugin|pdf\.attach)\s*\(\s*(?:source\s*:\s*)?|(?:import|include)\s+)(?:"[^"\n]*)?$"#,
            )
            .expect("valid path-completion trigger")
        })
        .is_match(line_before_cursor)
}

/// Save a clipboard PNG beside the edited source in an images directory. Each
/// attempted name uses create_new, so neither another paste nor an existing
/// asset can be overwritten. Project access rejects escaping paths/symlinks.
pub fn save_pasted_image(project: &Project, source: &str, png: &[u8]) -> Result<String> {
    let (source, _) = super::paths::safe_existing_path(project.root(), source)?;
    if !source.ends_with(".typ") || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(BackendError::Process(
            "Clipboard image is not a PNG for a Typst source".into(),
        ));
    }
    let parent = Path::new(&source).parent().unwrap_or_else(|| Path::new(""));
    for index in 1..10_000 {
        let name = if index == 1 {
            "pasted-image.png".to_owned()
        } else {
            format!("pasted-image-{index}.png")
        };
        let relative = format!("images/{name}");
        match project.create_binary_file(parent.join(&relative), png) {
            Ok(_) => return Ok(relative),
            Err(BackendError::AlreadyExists(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(BackendError::Process(
        "Too many pasted images in this folder".into(),
    ))
}

/// Markup needs a hash to enter code; an existing function argument does not.
pub fn pasted_image_source(text: &str, cursor: usize, relative: &str) -> String {
    let source = Source::detached(text);
    let root = LinkedNode::new(source.root());
    let leaf = root.leaf_at(cursor, Side::Before);
    if leaf.as_ref().is_some_and(|leaf| {
        leaf.kind() == typst::syntax::SyntaxKind::Str
            && cursor > leaf.offset()
            && cursor < leaf.range().end
    }) {
        return relative.to_owned();
    }
    let insert = format!("image(\"{relative}\")");
    // Completion's mode_after intentionally stays in code immediately after a
    // #function() so users can add a field access. A new pasted expression at
    // that same position is markup, so inspect the proposed insertion itself.
    let mut candidate = text.to_owned();
    if !candidate.is_char_boundary(cursor) {
        return format!("#{insert}");
    }
    candidate.insert_str(cursor, &insert);
    let candidate = Source::detached(candidate);
    let code = LinkedNode::new(candidate.root())
        .leaf_at(cursor + 1, Side::After)
        .is_some_and(|leaf| {
            leaf.kind() == typst::syntax::SyntaxKind::Ident
                && leaf.mode_after() == Some(typst::syntax::SyntaxMode::Code)
        });
    format!("{}{insert}", if code { "" } else { "#" })
}

/// One atomic math-delimiter edit, in UTF-8 byte offsets.
#[derive(Debug, PartialEq, Eq)]
pub struct MathEdit {
    pub range: Range<usize>,
    pub replacement: String,
    pub cursor: usize,
}

/// Use Typst's parser to distinguish real equation delimiters from literal
/// dollars in escapes, raw text, comments and strings.
pub fn math_edit(text: &str, cursor: usize, key: char) -> Option<MathEdit> {
    use typst::syntax::SyntaxKind;
    fn equation_at(source: &Source, offset: usize) -> Option<Range<usize>> {
        let root = LinkedNode::new(source.root());
        let leaf = root.leaf_at(offset, Side::After)?;
        if leaf.kind() != SyntaxKind::Dollar || leaf.offset() != offset {
            return None;
        }
        let parent = leaf.parent()?;
        (parent.kind() == SyntaxKind::Equation).then(|| parent.range())
    }
    if !text.is_char_boundary(cursor) {
        return None;
    }
    let following = text.get(cursor..)?;
    let empty_pair = cursor > 0 && text[..cursor].ends_with('$') && following.starts_with('$');
    if key != '$' && (!matches!(key, ' ' | '\u{8}') || !empty_pair) {
        return None;
    }
    let closing = (key == '$')
        .then(|| {
            following
                .char_indices()
                .find(|(_, character)| !character.is_whitespace())
        })
        .flatten()
        .filter(|(_, character)| *character == '$')
        .map(|(offset, _)| cursor + offset);
    let source = (closing.is_some() || empty_pair).then(|| Source::detached(text));
    if let Some(closing) = closing {
        if let Some(equation) = equation_at(source.as_ref()?, closing) {
            if equation.start < cursor && equation.end == closing + 1 {
                return Some(MathEdit {
                    range: cursor..cursor,
                    replacement: String::new(),
                    cursor: closing + 1,
                });
            }
        }
    }
    if matches!(key, ' ' | '\u{8}')
        && empty_pair
        && equation_at(source.as_ref()?, cursor - 1) == Some(cursor - 1..cursor + 1)
    {
        return Some(MathEdit {
            range: if key == ' ' {
                cursor..cursor
            } else {
                cursor - 1..cursor + 1
            },
            replacement: if key == ' ' {
                "  ".into()
            } else {
                String::new()
            },
            cursor: if key == ' ' { cursor + 1 } else { cursor - 1 },
        });
    }
    if key != '$' {
        return None;
    }
    let mut candidate = text.to_string();
    candidate.insert_str(cursor, "$$");
    let candidate = Source::detached(candidate);
    if equation_at(&candidate, cursor).is_some_and(|range| range.start == cursor) {
        Some(MathEdit {
            range: cursor..cursor,
            replacement: "$$".into(),
            cursor: cursor + 1,
        })
    } else {
        None
    }
}

/// Typst IDE snippets use `${name}` placeholders, including empty `${}`.
/// Insert their plain text and select the first placeholder in one undo step.
pub fn flatten_snippet(snippet: &str) -> (String, Option<Range<usize>>) {
    let mut text = String::new();
    let mut first = None;
    let mut remaining = snippet;
    while let Some(start) = remaining.find("${") {
        text.push_str(&remaining[..start]);
        let tail = &remaining[start + 2..];
        let Some(end) = tail.find('}') else {
            text.push_str(&remaining[start..]);
            return (text, first);
        };
        let offset = text.chars().count();
        let placeholder = &tail[..end];
        let value = match placeholder.split_once(':') {
            Some((number, default))
                if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) =>
            {
                default
            }
            _ if !placeholder.is_empty() && placeholder.chars().all(|c| c.is_ascii_digit()) => "",
            _ => placeholder,
        };
        text.push_str(value);
        first.get_or_insert(offset..text.chars().count());
        remaining = &tail[end + 1..];
    }
    text.push_str(remaining);
    (text, first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_completion_triggers_only_in_path_arguments() {
        for text in [
            "#image(",
            "#image(\"",
            "#image(\"images/plot",
            "#image(source: \"images/",
            "#figure(image(\"../",
            "#image(\n  \"",
            "#import \"chapters/",
            "#include ",
            "#include \"../",
            "#bibliography(\"refs",
        ] {
            assert!(may_complete_path(text), "{text}");
        }
        for text in [
            "ordinary prose",
            "#text(\"hello",
            "#fooimage(\"",
            "#image(\"done.png\", width: ",
            "#import \"done.typ\": ",
        ] {
            assert!(!may_complete_path(text), "{text}");
        }
    }

    #[test]
    fn clipboard_assets_preserve_existing_files_and_use_source_relative_paths() {
        let directory = tempfile::tempdir().unwrap();
        let project = Project::create(directory.path(), "Clipboard").unwrap();
        project
            .create_text_file("chapters/draft.typ", "Draft")
            .unwrap();
        let png = hex::decode("89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000b49444154789c636000020000050001a5f645400000000049454e44ae426082").unwrap();
        let first = save_pasted_image(&project, "chapters/draft.typ", &png).unwrap();
        let second = save_pasted_image(&project, "chapters/draft.typ", &png).unwrap();
        assert_eq!(first, "images/pasted-image.png");
        assert_eq!(second, "images/pasted-image-2.png");
        for relative in [&first, &second] {
            assert_eq!(
                std::fs::read(project.root().join("chapters").join(relative)).unwrap(),
                png
            );
        }
        assert_eq!(
            project.read_file("chapters/draft.typ").unwrap().content,
            super::super::FileContent::Text("Draft".into())
        );
        assert!(save_pasted_image(&project, "../outside.typ", &png).is_err());
        assert!(save_pasted_image(&project, "main.typ", b"not an image").is_err());
        assert_eq!(
            pasted_image_source("hello ", 6, &first),
            "#image(\"images/pasted-image.png\")"
        );
        assert_eq!(
            pasted_image_source("#figure()", 8, &first),
            "image(\"images/pasted-image.png\")"
        );
        assert_eq!(
            pasted_image_source("#image()", 8, &first),
            "#image(\"images/pasted-image.png\")"
        );
        assert_eq!(
            pasted_image_source("#image(\"\")", 8, &first),
            "images/pasted-image.png"
        );
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_assets_reject_an_images_directory_outside_the_project() {
        let directory = tempfile::tempdir().unwrap();
        let project = Project::create(directory.path(), "Clipboard").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), project.root().join("images")).unwrap();
        assert!(save_pasted_image(&project, "main.typ", b"\x89PNG\r\n\x1a\n").is_err());
        assert_eq!(outside.path().read_dir().unwrap().count(), 0);
    }
    #[test]
    fn math_delimiters_respect_typst_context() {
        for text in ["", "é🙂 ", "#text[inside ", "$x$ after "] {
            let edit = math_edit(text, text.len(), '$').expect(text);
            assert_eq!(edit.replacement, "$$");
            assert_eq!(edit.cursor, text.len() + 1);
        }
        for text in [
            r"\",
            "`raw ",
            "```typ\nraw ",
            "// comment ",
            "/* comment ",
            "#let s = \"string ",
            "$x + ",
        ] {
            assert_eq!(math_edit(text, text.len(), '$'), None, "{text}");
        }
        let edit = math_edit("é $$", 4, ' ').unwrap();
        assert_eq!(edit.replacement, "  ");
        assert_eq!(edit.cursor, 5);
        assert_eq!(math_edit("$x$", 2, '$').unwrap().cursor, 3);
        let edit = math_edit("$$", 1, '\u{8}').unwrap();
        assert_eq!(edit.range, 0..2);
        for text in [r"\$$", "`$$`", "// $$", "#let s = \"$$\""] {
            let cursor = text.find("$$").unwrap() + 1;
            assert_eq!(math_edit(text, cursor, ' '), None, "{text}");
            assert_eq!(math_edit(text, cursor, '\u{8}'), None, "{text}");
        }
    }

    #[test]
    fn closing_math_skips_block_padding_only_inside_the_current_equation() {
        for text in ["$ x^2 $", "é🙂 $ x^2 \t\n $"] {
            let cursor = text.find("x^2").unwrap() + 3;
            let edit = math_edit(text, cursor, '$').unwrap();
            assert_eq!(edit.range, cursor..cursor);
            assert!(edit.replacement.is_empty());
            assert_eq!(edit.cursor, text.rfind('$').unwrap() + 1);
        }
        for text in ["Prose $next$", "$previous$ $next$"] {
            let cursor = text.find(" $next$").unwrap();
            let edit = math_edit(text, cursor, '$').unwrap();
            assert_eq!(edit.replacement, "$$");
            assert_eq!(edit.cursor, cursor + 1);
        }
    }

    #[test]
    fn offsets_and_snippets_preserve_unicode() {
        assert_eq!(char_to_byte("é🙂x", 2), Some(6));
        assert_eq!(byte_to_char("é🙂x", 6), Some(2));
        assert_eq!(byte_to_char("é🙂x", 3), None);
        assert_eq!(char_to_byte("é🙂x", 4), None);
        assert_eq!(
            flatten_snippet("é(${λ}, ${})"),
            ("é(λ, )".into(), Some(2..3))
        );
        assert_eq!(
            flatten_snippet("image(\"${}\")"),
            ("image(\"\")".into(), Some(7..7))
        );
        assert_eq!(flatten_snippet("${x}_${2:2}"), ("x_2".into(), Some(0..1)));
    }

    #[test]
    fn formatter_maps_repeated_unicode_tokens_and_rejects_syntax_errors() {
        let source = "#let café=(1,2)\n#let pair=(café,café)\n#café\n";
        let start = source.rfind("café").unwrap();
        let end = start + "café".len();
        let EditorResponse::Formatted {
            text,
            cursor,
            anchor,
        } = format_source(Source::detached(source), end, start).unwrap()
        else {
            panic!()
        };
        assert_eq!(&text[anchor..cursor], "café");
        assert_eq!(anchor, text.rfind("café").unwrap());
        let EditorResponse::Formatted { text: again, .. } =
            format_source(Source::detached(text.clone()), cursor, anchor).unwrap()
        else {
            panic!()
        };
        assert_eq!(text, again);
        assert!(format_source(Source::detached("#let café=("), 0, 0).is_err());
        let source = "#set page(width:240pt,height:180pt,margin:20pt)\n#let café=(1,2)\n= A short draft\nHello café. $x^2+y^2=z^2$\n#text(weight:\"bold\")[Result: #café.at(0)]\n";
        let start = source.rfind("café").unwrap();
        let EditorResponse::Formatted {
            text,
            cursor,
            anchor,
        } = format_source(Source::detached(source), start + "café".len(), start).unwrap()
        else {
            panic!()
        };
        assert_eq!(&text[anchor..cursor], "café");
        let source = "#let café=(1,2)\n#café";
        let start = source.rfind("café").unwrap();
        for (cursor, anchor) in [(source.len(), start), (start, source.len())] {
            let EditorResponse::Formatted {
                text,
                cursor,
                anchor,
            } = format_source(Source::detached(source), cursor, anchor).unwrap()
            else {
                panic!()
            };
            assert_eq!(&text[cursor.min(anchor)..cursor.max(anchor)], "café");
        }
    }
}
