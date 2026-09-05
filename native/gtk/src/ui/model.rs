//! Display-independent UI state and input mapping.
//!
//! Keeping these rules outside GTK makes presentation navigation and startup
//! handling testable on builders that do not have a display server.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSettings {
    pub theme: Theme,
    pub font_size: u32,
    pub auto_compile: bool,
    pub compile_delay_ms: u32,
    pub line_wrapping: bool,
    pub line_numbers: bool,
    pub vim_mode: bool,
    pub page_size: String,
    pub presentation_notes_layout: String,
    pub presentation_notes_font_size: u32,
    pub system_fonts: bool,
    pub google_fonts: bool,
    pub translucent: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_size: 15,
            auto_compile: true,
            compile_delay_ms: 100,
            line_wrapping: true,
            line_numbers: true,
            vim_mode: false,
            page_size: "auto".into(),
            presentation_notes_layout: "auto".into(),
            presentation_notes_font_size: 17,
            system_fonts: true,
            google_fonts: true,
            translucent: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Files,
    Contents,
    Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationTool {
    Pointer,
    Laser,
    Pen,
    Highlighter,
    Eraser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blackout {
    None,
    Black,
    White,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedPoint {
    pub x: f64,
    pub y: f64,
}

impl NormalizedPoint {
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationStroke {
    pub tool: PresentationTool,
    pub color: (f64, f64, f64, f64),
    pub points: Vec<NormalizedPoint>,
}

#[derive(Debug, Clone)]
pub struct PresentationState {
    pub slide: usize,
    pub slide_count: usize,
    pub tool: PresentationTool,
    pub blackout: Blackout,
    pub strokes: Vec<Vec<AnnotationStroke>>,
    pub started_at: Instant,
    pub elapsed_before_pause: Duration,
    pub timer_running: bool,
    pub notes_visible: bool,
    pub laser: Option<NormalizedPoint>,
}

impl PresentationState {
    pub fn new(slide_count: usize) -> Self {
        Self {
            slide: 0,
            slide_count,
            tool: PresentationTool::Pointer,
            blackout: Blackout::None,
            strokes: vec![Vec::new(); slide_count],
            started_at: Instant::now(),
            elapsed_before_pause: Duration::ZERO,
            timer_running: true,
            notes_visible: false,
            laser: None,
        }
    }

    pub fn goto(&mut self, slide: usize) {
        self.slide = slide.min(self.slide_count.saturating_sub(1));
        self.blackout = Blackout::None;
        self.laser = None;
    }

    pub fn next(&mut self) {
        if self.blackout == Blackout::None {
            self.goto(self.slide.saturating_add(1));
        } else {
            self.blackout = Blackout::None;
        }
    }

    pub fn previous(&mut self) {
        if self.blackout == Blackout::None {
            self.goto(self.slide.saturating_sub(1));
        } else {
            self.blackout = Blackout::None;
        }
    }

    pub fn elapsed(&self) -> Duration {
        if self.timer_running {
            self.elapsed_before_pause + self.started_at.elapsed()
        } else {
            self.elapsed_before_pause
        }
    }

    pub fn toggle_timer(&mut self) {
        if self.timer_running {
            self.elapsed_before_pause += self.started_at.elapsed();
        } else {
            self.started_at = Instant::now();
        }
        self.timer_running = !self.timer_running;
    }

    pub fn reset_timer(&mut self) {
        self.elapsed_before_pause = Duration::ZERO;
        self.started_at = Instant::now();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationCommand {
    Next,
    Previous,
    First,
    Last,
    ToggleBlack,
    ToggleWhite,
    ToggleGrid,
    ToggleNotes,
    ToggleLaser,
    TogglePen,
    ToggleHighlighter,
    ToggleEraser,
    ClearAnnotations,
    ToggleTimer,
    ResetTimer,
    ToggleFullscreen,
    Exit,
}

pub fn presentation_command_for_key(key: &str) -> Option<PresentationCommand> {
    match key {
        "Right" | "Down" | "Page_Down" | "space" | "Return" | "j" | "J" | "n" | "N" => {
            Some(PresentationCommand::Next)
        }
        "Left" | "Up" | "Page_Up" | "BackSpace" | "k" | "K" | "p" | "P" => {
            Some(PresentationCommand::Previous)
        }
        "Home" => Some(PresentationCommand::First),
        "End" => Some(PresentationCommand::Last),
        "b" | "B" | "period" => Some(PresentationCommand::ToggleBlack),
        "w" | "W" | "comma" => Some(PresentationCommand::ToggleWhite),
        "g" | "G" | "o" | "O" => Some(PresentationCommand::ToggleGrid),
        "s" | "S" => Some(PresentationCommand::ToggleNotes),
        "l" | "L" => Some(PresentationCommand::ToggleLaser),
        "d" | "D" => Some(PresentationCommand::TogglePen),
        "h" | "H" => Some(PresentationCommand::ToggleHighlighter),
        "e" | "E" => Some(PresentationCommand::ToggleEraser),
        "c" | "C" | "Delete" => Some(PresentationCommand::ClearAnnotations),
        "t" | "T" => Some(PresentationCommand::ToggleTimer),
        "r" | "R" => Some(PresentationCommand::ResetTimer),
        "f" | "F" | "F11" => Some(PresentationCommand::ToggleFullscreen),
        "Escape" | "q" | "Q" => Some(PresentationCommand::Exit),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SlideNumberBuffer(String);

impl SlideNumberBuffer {
    pub fn push_digit(&mut self, digit: char) {
        if digit.is_ascii_digit() && self.0.len() < 4 {
            self.0.push(digit);
        }
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Presentation slide numbers are one-based for humans.
    pub fn commit(&mut self, slide_count: usize) -> Option<usize> {
        let typed = std::mem::take(&mut self.0).parse::<usize>().ok()?;
        Some(typed.saturating_sub(1).min(slide_count.saturating_sub(1)))
    }

    pub fn display(&self) -> &str {
        &self.0
    }
}

/// A positional `.typ` argument opens its containing vault and selects the file;
/// any other positional path is treated as a vault directory.
pub fn resolve_startup_path(path: impl AsRef<Path>) -> (PathBuf, Option<PathBuf>) {
    let path = path.as_ref();
    if path.extension().and_then(|part| part.to_str()) == Some("typ") {
        let immediate = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let root = immediate
            .ancestors()
            .find(|candidate| {
                candidate.join("main.typ").is_file() || candidate.join(".typsmthng").exists()
            })
            .map(Path::to_path_buf)
            .unwrap_or(immediate);
        let selected = path
            .strip_prefix(&root)
            .ok()
            .map(Path::to_path_buf)
            .or_else(|| path.file_name().map(PathBuf::from));
        (root, selected)
    } else {
        (path.to_path_buf(), None)
    }
}

pub fn format_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_typ_file_selects_file_inside_parent_vault() {
        assert_eq!(
            resolve_startup_path("/tmp/deck/main.typ"),
            (PathBuf::from("/tmp/deck"), Some(PathBuf::from("main.typ")))
        );
    }

    #[test]
    fn startup_directory_has_no_selected_file() {
        assert_eq!(
            resolve_startup_path("/tmp/deck"),
            (PathBuf::from("/tmp/deck"), None)
        );
    }

    #[test]
    fn nested_typ_file_resolves_to_nearest_project_marker() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("main.typ"), "Main").unwrap();
        let chapter = directory.path().join("chapters/one.typ");
        std::fs::create_dir_all(chapter.parent().unwrap()).unwrap();
        std::fs::write(&chapter, "Chapter").unwrap();
        assert_eq!(
            resolve_startup_path(&chapter),
            (
                directory.path().to_path_buf(),
                Some(PathBuf::from("chapters/one.typ"))
            )
        );
    }

    #[test]
    fn presentation_navigation_clamps_to_deck() {
        let mut state = PresentationState::new(2);
        state.next();
        state.next();
        assert_eq!(state.slide, 1);
        state.previous();
        state.previous();
        assert_eq!(state.slide, 0);
    }

    #[test]
    fn key_map_supports_remote_friendly_navigation() {
        assert_eq!(
            presentation_command_for_key("space"),
            Some(PresentationCommand::Next)
        );
        assert_eq!(
            presentation_command_for_key("Page_Up"),
            Some(PresentationCommand::Previous)
        );
        assert_eq!(
            presentation_command_for_key("Escape"),
            Some(PresentationCommand::Exit)
        );
    }

    #[test]
    fn slide_number_buffer_is_one_based_and_clamped() {
        let mut buffer = SlideNumberBuffer::default();
        buffer.push_digit('9');
        buffer.push_digit('7');
        assert_eq!(buffer.commit(12), Some(11));
        assert_eq!(buffer.display(), "");
    }
}
