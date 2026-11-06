//! Colors and text styles: the terminal's own background, with a yellow accent.

use ratatui::style::{Color, Modifier, Style};

pub const TEXT: Color = Color::Rgb(0xe6, 0xe6, 0xe6);
pub const DIM: Color = Color::Rgb(0x7a, 0x7a, 0x7a);
pub const ACCENT: Color = Color::Rgb(0xe8, 0xc8, 0x4f);
pub const OK: Color = Color::Rgb(0x4c, 0xc2, 0x86);
pub const ERROR: Color = Color::Rgb(0xe5, 0x5a, 0x52);
pub const NUMBER: Color = Color::Rgb(0xe8, 0x84, 0x4a);
/// A scheduled job's icon in the sidebar.
pub const SCHEDULE: Color = Color::Rgb(0x63, 0x8f, 0xf0);
/// Behind the top bar and the status bar.
pub const BAR: Color = Color::Rgb(0x00, 0x00, 0x00);
/// Behind the focused row.
pub const SELECTED: Color = Color::Rgb(0x31, 0x31, 0x31);

pub const CMD: Color = Color::Rgb(0x4c, 0xc2, 0x86);
pub const FOR_EACH: Color = Color::Rgb(0x63, 0x8f, 0xf0);
pub const WHILE: Color = Color::Rgb(0x45, 0xbd, 0xcc);
pub const TRIGGER: Color = Color::Rgb(0xe8, 0x84, 0x4a);

pub fn text() -> Style {
    Style::new().fg(TEXT)
}

pub fn dim() -> Style {
    Style::new().fg(DIM)
}

pub fn accent() -> Style {
    Style::new().fg(ACCENT)
}

pub fn ok() -> Style {
    Style::new().fg(OK)
}

pub fn error() -> Style {
    Style::new().fg(ERROR)
}

pub fn bold() -> Style {
    Style::new().fg(TEXT).add_modifier(Modifier::BOLD)
}

/// A section heading: `TASKS · 3`.
pub fn heading() -> Style {
    Style::new().fg(DIM).add_modifier(Modifier::BOLD)
}
