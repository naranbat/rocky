//! Colors and styles, from the design in `tui-design/`: a dark background with an amber
//! accent, and key chips in the footers.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub const BG: Color = Color::Rgb(0x0d, 0x10, 0x15);
/// Behind the top bar and the footers.
pub const PANEL: Color = Color::Rgb(0x16, 0x1a, 0x20);
/// Behind the selected row.
pub const SELECTED: Color = Color::Rgb(0x1f, 0x24, 0x2c);
/// Borders and separators.
pub const LINE: Color = Color::Rgb(0x2a, 0x2f, 0x38);
pub const TEXT: Color = Color::Rgb(0xe6, 0xe8, 0xeb);
pub const MUTED: Color = Color::Rgb(0x9a, 0xa0, 0xa8);
pub const DIM: Color = Color::Rgb(0x5c, 0x63, 0x6c);
pub const ACCENT: Color = Color::Rgb(0xf0, 0xb5, 0x4a);
pub const OK: Color = Color::Rgb(0x6f, 0xcf, 0x7f);
pub const ERROR: Color = Color::Rgb(0xf0, 0x6a, 0x5f);
pub const BLUE: Color = Color::Rgb(0x6a, 0xae, 0xf0);
/// YAML keys outside the highlighted part.
pub const DIM_BLUE: Color = Color::Rgb(0x3d, 0x5a, 0x7a);
pub const PURPLE: Color = Color::Rgb(0xc4, 0x9a, 0xf0);
const CHIP: Color = Color::Rgb(0xc8, 0xcc, 0xd2);

pub fn fg(color: Color) -> Style {
    Style::new().fg(color)
}

pub fn bold(color: Color) -> Style {
    fg(color).add_modifier(Modifier::BOLD)
}

/// A footer key and its label: `[↵] open`. A `main` key is drawn in the accent color.
pub fn key(key: &str, label: &str, main: bool) -> Vec<Span<'static>> {
    let chip = if main { ACCENT } else { CHIP };
    vec![
        Span::styled(format!(" {key} "), Style::new().fg(BG).bg(chip)),
        Span::styled(format!(" {label}   "), fg(MUTED)),
    ]
}

/// Footer keys, each `(key, label)`; keys in `main` are drawn in the accent color.
pub fn keys(items: &[(&str, &str)], main: &[&str]) -> Vec<Span<'static>> {
    items
        .iter()
        .flat_map(|(k, label)| key(k, label, main.contains(k)))
        .collect()
}

/// `text` cut to `width` characters, ending in `…` when cut.
pub fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_cuts_long_text() {
        assert_eq!(fit("abc", 3), "abc");
        assert_eq!(fit("abcdef", 4), "abc…");
    }
}
