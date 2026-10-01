//! Speaker notes loaded and saved beside a project's compiled entrypoint.
use super::{FileContent, InlineNote, Project, Result};

/// Returned vectors use zero-based indexes; queried and sidecar page numbers start at one.
pub fn load(
    context: Option<(&Project, &str)>,
    page_count: usize,
    inline: &[InlineNote],
) -> (Vec<String>, Vec<String>) {
    let mut inline_notes = vec![String::new(); page_count];
    let mut sidecar_notes = vec![String::new(); page_count];
    for note in inline {
        if let Some(target) = note
            .page
            .checked_sub(1)
            .and_then(|index| inline_notes.get_mut(index))
        {
            if !target.is_empty() {
                target.push_str("\n\n");
            }
            target.push_str(&note.text);
        }
    }
    let Some((project, current)) = context else {
        return (inline_notes, sidecar_notes);
    };
    for (number, note) in parse_note_sections(&read_text(project, &sidecar_path(current))) {
        if let Some(index) = number.checked_sub(1).filter(|index| *index < page_count) {
            sidecar_notes[index] = note;
        }
    }
    for (index, line) in read_text(project, current)
        .lines()
        .filter_map(|line| line.trim().strip_prefix("// note:").map(str::trim))
        .enumerate()
    {
        if index < page_count && inline_notes[index].is_empty() {
            inline_notes[index] = line.to_string();
        }
    }
    (inline_notes, sidecar_notes)
}

/// Save one zero-based slide. Blank text removes its section.
pub fn save(project: &Project, current: &str, slide: usize, text: &str) -> Result<()> {
    let sidecar = sidecar_path(current);
    let existing = read_text(project, &sidecar);
    let mut sections = parse_note_sections(&existing);
    if text.trim().is_empty() {
        sections.remove(&(slide + 1));
    } else {
        sections.insert(slide + 1, text.trim().to_string());
    }
    if sections.is_empty() && existing.is_empty() {
        return Ok(());
    }
    project
        .write_text_atomic(
            sidecar,
            &serialize_note_sections(&sections, &project.name()),
        )
        .map(|_| ())
}

fn sidecar_path(current: &str) -> String {
    format!("{}.notes.md", current.trim_end_matches(".typ"))
}
fn read_text(project: &Project, path: &str) -> String {
    project
        .read_file(path)
        .ok()
        .and_then(|file| match file.content {
            FileContent::Text(text) => Some(text),
            FileContent::Binary(_) => None,
        })
        .unwrap_or_default()
}

fn parse_note_sections(markdown: &str) -> std::collections::BTreeMap<usize, String> {
    let mut sections = std::collections::BTreeMap::new();
    let mut slide = None;
    let mut lines = Vec::new();
    let flush = |slide: Option<usize>,
                 lines: &mut Vec<&str>,
                 sections: &mut std::collections::BTreeMap<usize, String>| {
        if let Some(slide) = slide {
            let note = lines.join("\n").trim().to_string();
            if !note.is_empty() {
                sections.insert(slide, note);
            }
        }
        lines.clear();
    };
    for line in markdown.lines() {
        if let Some(number) = sidecar_slide_heading(line) {
            flush(slide, &mut lines, &mut sections);
            slide = Some(number);
        } else if slide.is_some() {
            lines.push(line);
        }
    }
    flush(slide, &mut lines, &mut sections);
    sections
}

fn serialize_note_sections(
    sections: &std::collections::BTreeMap<usize, String>,
    title: &str,
) -> String {
    let mut markdown = format!("# Speaker notes — {title}\n\n");
    for (number, note) in sections {
        markdown.push_str(&format!("## Slide {number}\n\n{}\n\n", note.trim()));
    }
    markdown.truncate(markdown.trim_end().len());
    markdown.push('\n');
    markdown
}

fn sidecar_slide_heading(line: &str) -> Option<usize> {
    let heading = line.trim().strip_prefix("##")?.trim();
    let number = if heading
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("slide"))
    {
        heading.get(5..)?.trim()
    } else {
        heading
    };
    number.parse::<usize>().ok().filter(|number| *number > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_preserve_inline_fallback_and_sidecar_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let project = Project::create(directory.path(), "Demo").unwrap();
        project
            .write_text_atomic(
                "deck.typ",
                "// note: First fallback\n// note: Second fallback\n",
            )
            .unwrap();
        project
            .write_text_atomic(
                "deck.notes.md",
                "## Slide 1\nSidecar\n## Slide 99\nIgnored\n",
            )
            .unwrap();
        let inline = [
            InlineNote {
                page: 1,
                text: "First".into(),
            },
            InlineNote {
                page: 1,
                text: "Second".into(),
            },
            InlineNote {
                page: 0,
                text: "Invalid".into(),
            },
        ];
        assert_eq!(
            load(Some((&project, "deck.typ")), 2, &inline),
            (
                vec!["First\n\nSecond".into(), "Second fallback".into()],
                vec!["Sidecar".into(), "".into()]
            )
        );
        assert_eq!(
            load(None, 2, &inline),
            (
                vec!["First\n\nSecond".into(), "".into()],
                vec!["".into(), "".into()]
            )
        );
        assert_eq!(load(None, 0, &inline), (vec![], vec![]));
        project.delete_permanently("deck.notes.md").unwrap();
        project
            .create_binary_file("deck.notes.md", &[0xff])
            .unwrap();
        assert_eq!(load(Some((&project, "deck.typ")), 2, &inline).1, ["", ""]);
    }

    #[test]
    fn notes_save_preserves_other_sections_and_project_path_safety() {
        let directory = tempfile::tempdir().unwrap();
        let project = Project::create(directory.path(), "Demo").unwrap();
        save(&project, "chapters/deck.typ", 0, " ").unwrap();
        assert!(!project.root().join("chapters/deck.notes.md").exists());
        project
            .write_text_atomic(
                "chapters/deck.notes.md",
                "## 1\nFirst\n## slide 2\nSecond\n",
            )
            .unwrap();
        save(&project, "chapters/deck.typ", 0, " Updated ").unwrap();
        assert_eq!(
            load(Some((&project, "chapters/deck.typ")), 2, &[]).1,
            ["Updated", "Second"]
        );
        save(&project, "chapters/deck.typ", 0, "").unwrap();
        assert_eq!(
            load(Some((&project, "chapters/deck.typ")), 2, &[]).1,
            ["", "Second"]
        );
        assert!(save(&project, "../outside.typ", 0, "Note").is_err());
        assert!(!directory.path().join("outside.notes.md").exists());
    }
    #[test]
    fn sidecar_headings_accept_legacy_and_current_forms() {
        assert_eq!(sidecar_slide_heading("## Slide 2"), Some(2));
        assert_eq!(sidecar_slide_heading("## slide 3"), Some(3));
        assert_eq!(sidecar_slide_heading(" ## 4 "), Some(4));
        assert_eq!(sidecar_slide_heading("# Slide 5"), None);
        assert_eq!(sidecar_slide_heading("## Slide 0"), None);
    }

    #[test]
    fn legacy_sidecar_sections_survive_parsing() {
        let sections = parse_note_sections(
            "# Speaker notes\n\n## 1\n\nFirst note\n\n## slide 2\n\nSecond note\n\n## Slide 3\n\nThird note\n",
        );
        assert_eq!(sections.get(&1).map(String::as_str), Some("First note"));
        assert_eq!(sections.get(&2).map(String::as_str), Some("Second note"));
        assert_eq!(sections.get(&3).map(String::as_str), Some("Third note"));
        let serialized = serialize_note_sections(&sections, "Demo");
        assert_eq!(
            serialized,
            "# Speaker notes — Demo\n\n## Slide 1\n\nFirst note\n\n## Slide 2\n\nSecond note\n\n## Slide 3\n\nThird note\n"
        );
        assert_eq!(parse_note_sections(&serialized), sections);
    }
}
