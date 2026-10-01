//! Editor-only requests and UTF-8/snippet handling shared with the native UI.
use std::ops::Range;

#[derive(Debug, Clone)]
pub enum EditorQuery {
    Complete,
    Hover,
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
}
