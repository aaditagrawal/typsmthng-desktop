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

/// Which workspace panes are visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Source,
    Split,
    Preview,
}

impl ViewMode {
    pub const ALL: [ViewMode; 3] = [ViewMode::Source, ViewMode::Split, ViewMode::Preview];

    pub fn id(self) -> &'static str {
        match self {
            ViewMode::Source => "source",
            ViewMode::Split => "split",
            ViewMode::Preview => "preview",
        }
    }

    pub fn from_id(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|mode| mode.id() == id)
            .unwrap_or(ViewMode::Split)
    }

    pub fn shows_source(self) -> bool {
        self != ViewMode::Preview
    }

    pub fn shows_preview(self) -> bool {
        self != ViewMode::Source
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSettings {
    pub theme: Theme,
    pub font_size: u32,
    pub auto_compile: bool,
    pub auto_save: bool,
    pub auto_save_delay_ms: u32,
    pub save_on_focus_loss: bool,
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
    pub view_mode: ViewMode,
    pub minimap: bool,
    pub centered_scrolling: bool,
    /// Empty uses the desktop's monospace font.
    pub editor_font_family: String,
    /// Line height as a percentage of the font's own line height.
    pub editor_line_height: u32,
    pub editor_ligatures: bool,
    /// Empty uses the desktop interface font.
    pub ui_font_family: String,
    /// Points; 0 uses the desktop interface font size.
    pub ui_font_size: u32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_size: DEFAULT_EDITOR_FONT_SIZE,
            auto_compile: true,
            compile_delay_ms: 100,
            auto_save: true,
            auto_save_delay_ms: 100,
            save_on_focus_loss: false,
            line_wrapping: true,
            line_numbers: true,
            vim_mode: false,
            page_size: "auto".into(),
            presentation_notes_layout: "auto".into(),
            presentation_notes_font_size: 17,
            system_fonts: true,
            google_fonts: true,
            translucent: false,
            view_mode: ViewMode::Split,
            minimap: true,
            centered_scrolling: false,
            editor_font_family: String::new(),
            editor_line_height: 100,
            editor_ligatures: true,
            ui_font_family: String::new(),
            ui_font_size: 0,
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

pub const DEFAULT_EDITOR_FONT_SIZE: u32 = 15;
pub const EDITOR_FONT_SIZES: std::ops::RangeInclusive<u32> = 8..=40;

/// Apply `steps` zoom notches to an editor font size in points.
pub fn zoom_editor_font(size: u32, steps: i32) -> u32 {
    size.saturating_add_signed(steps)
        .clamp(*EDITOR_FONT_SIZES.start(), *EDITOR_FONT_SIZES.end())
}

pub const EDITOR_LINE_HEIGHTS: std::ops::RangeInclusive<u32> = 100..=200;
/// Interface sizes offered in settings; 0 ("Auto") follows the desktop.
pub const UI_FONT_SIZES_OFFERED: [u32; 9] = [9, 10, 11, 12, 13, 14, 15, 16, 18];

/// CSS for the editor's font family, size and ligature features.
pub fn editor_css(settings: &UiSettings) -> String {
    let family = match settings.editor_font_family.trim() {
        "" => String::new(),
        family => format!(
            "font-family: \"{}\", monospace; ",
            family.replace(['"', '\\'], "")
        ),
    };
    let features = if settings.editor_ligatures {
        ""
    } else {
        "font-feature-settings: \"liga\" 0, \"calt\" 0, \"dlig\" 0; "
    };
    format!(
        ".typst-editor {{ {family}font-size: {}pt; {features}}}",
        settings.font_size
    )
}

/// Extra pixels (above, below) each line for the configured line height.
pub fn editor_line_padding(font_size: u32, line_height: u32) -> (i32, i32) {
    let font_pixels = f64::from(font_size) * 96.0 / 72.0;
    let extra = (font_pixels * f64::from(line_height.saturating_sub(100)) / 100.0).round() as i32;
    (extra / 2, extra - extra / 2)
}

/// The `gtk-font-name` for the interface, or `None` to keep the desktop's.
/// `system` is the desktop value, e.g. `"Cantarell 11"`.
pub fn ui_font_name(settings: &UiSettings, system: &str) -> Option<String> {
    let family = settings.ui_font_family.trim();
    if family.is_empty() && settings.ui_font_size == 0 {
        return None;
    }
    // Pango font names end in the size; everything before it is the family.
    let (system_family, system_size) = match system.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f64>().is_ok() => (family, size),
        _ => (system, "11"),
    };
    let family = if family.is_empty() {
        system_family
    } else {
        family
    };
    let size = match settings.ui_font_size {
        0 => system_size.to_string(),
        size => size.to_string(),
    };
    Some(format!("{family} {size}"))
}

/// Explicit page sizes offered in settings, in dropdown order after "Auto".
pub const PAGE_SIZES: [(&str, &str); 8] = [
    ("a3", "A3"),
    ("a4", "A4"),
    ("a5", "A5"),
    ("a6", "A6"),
    ("us-letter", "US Letter"),
    ("us-legal", "US Legal"),
    ("iso-b5", "ISO B5"),
    ("presentation-16-9", "Presentation 16:9"),
];

/// Map a GTK/PWG paper name (`gtk_paper_size_get_default`, which follows
/// `LC_PAPER` on glibc and the user's region elsewhere) to a Typst paper.
pub fn locale_page_size(paper_name: &str) -> &'static str {
    match paper_name {
        "na_letter" => "us-letter",
        "na_legal" => "us-legal",
        "iso_a3" => "a3",
        "iso_a5" => "a5",
        "iso_a6" => "a6",
        "iso_b5" => "iso-b5",
        _ => "a4",
    }
}

/// Resolve the "auto" setting against the locale's paper size.
pub fn effective_page_size<'a>(setting: &'a str, locale_paper: &'a str) -> &'a str {
    if PAGE_SIZES.iter().any(|(id, _)| *id == setting) {
        setting
    } else {
        locale_paper
    }
}

pub fn page_size_label(id: &str) -> &'static str {
    PAGE_SIZES
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map_or("A4", |(_, label)| label)
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
                candidate.join("typst.toml").is_file()
                    || candidate.join("main.typ").is_file()
                    || candidate.join(".typsmthng").exists()
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
    fn font_settings_render_css_padding_and_interface_names() {
        let mut settings = UiSettings::default();
        assert_eq!(editor_css(&settings), ".typst-editor { font-size: 15pt; }");
        settings.editor_font_family = "JetBrains \"Mono".into();
        settings.editor_ligatures = false;
        assert_eq!(
            editor_css(&settings),
            ".typst-editor { font-family: \"JetBrains Mono\", monospace; font-size: 15pt; \
             font-feature-settings: \"liga\" 0, \"calt\" 0, \"dlig\" 0; }"
        );
        assert_eq!(editor_line_padding(15, 100), (0, 0));
        assert_eq!(editor_line_padding(15, 150), (5, 5));
        assert_eq!(editor_line_padding(12, 125), (2, 2));
        assert_eq!(ui_font_name(&settings, "Cantarell 11"), None);
        settings.ui_font_size = 13;
        assert_eq!(
            ui_font_name(&settings, "Noto Sans CJK 10.5"),
            Some("Noto Sans CJK 13".into())
        );
        settings.ui_font_family = "Inter".into();
        assert_eq!(
            ui_font_name(&settings, "Cantarell 11"),
            Some("Inter 13".into())
        );
        settings.ui_font_size = 0;
        assert_eq!(
            ui_font_name(&settings, "Cantarell 11"),
            Some("Inter 11".into())
        );
    }

    #[test]
    fn editor_zoom_steps_by_point_within_bounds() {
        assert_eq!(zoom_editor_font(15, 1), 16);
        assert_eq!(zoom_editor_font(15, -3), 12);
        assert_eq!(zoom_editor_font(9, -5), 8);
        assert_eq!(zoom_editor_font(39, 4), 40);
    }

    #[test]
    fn view_mode_round_trips_and_defaults_to_split() {
        for mode in ViewMode::ALL {
            assert_eq!(ViewMode::from_id(mode.id()), mode);
        }
        assert_eq!(ViewMode::from_id("sideways"), ViewMode::Split);
        assert!(!ViewMode::Preview.shows_source());
        assert!(!ViewMode::Source.shows_preview());
    }

    #[test]
    fn auto_page_size_follows_locale_paper() {
        assert_eq!(locale_page_size("na_letter"), "us-letter");
        assert_eq!(locale_page_size("iso_a4"), "a4");
        assert_eq!(locale_page_size("unknown"), "a4");
        assert_eq!(effective_page_size("auto", "us-letter"), "us-letter");
        assert_eq!(effective_page_size("a5", "us-letter"), "a5");
        assert_eq!(page_size_label("us-letter"), "US Letter");
    }

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
    fn nested_typ_file_resolves_to_typst_manifest_root() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("typst.toml"), "[package]").unwrap();
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
