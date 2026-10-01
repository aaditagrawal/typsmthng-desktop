//! Editor-only requests and UTF-8/snippet handling shared with the native UI.
use super::{BackendError, Result};
use std::ops::Range;
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
