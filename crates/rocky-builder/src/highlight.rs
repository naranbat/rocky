//! Colors for one line of YAML: keys, numbers, comments; all faint outside the focused part.

use ratatui::style::Color;
use ratatui::text::Span;

use crate::theme::{self, fg};

/// `line` in colored spans; `faint` for lines outside the focused part.
pub fn line(line: &str, faint: bool) -> Vec<Span<'static>> {
    let (key_color, value_color, number_color) = if faint {
        (theme::DIM_BLUE, theme::DIM, theme::DIM)
    } else {
        (theme::BLUE, theme::TEXT, theme::ACCENT)
    };
    let mut spans = Vec::new();
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    spans.push(Span::raw(indent.to_string()));
    if body.starts_with('#') {
        spans.push(Span::styled(body.to_string(), fg(theme::DIM)));
        return spans;
    }
    let mut rest = body;
    if let Some(after) = rest.strip_prefix("- ") {
        spans.push(Span::styled("- ", fg(theme::DIM)));
        rest = after;
    }
    if let Some(end) = key_end(rest) {
        spans.push(Span::styled(rest[..end].to_string(), fg(key_color)));
        rest = &rest[end..];
    }
    let (value, comment) = match rest.find(" #") {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let number = value
        .trim_start()
        .starts_with(|c: char| c.is_ascii_digit() || c == '-');
    let color: Color = if number { number_color } else { value_color };
    spans.push(Span::styled(value.to_string(), fg(color)));
    if !comment.is_empty() {
        spans.push(Span::styled(comment.to_string(), fg(theme::DIM)));
    }
    spans
}

/// Where a leading `key:` ends, its colon included.
fn key_end(text: &str) -> Option<usize> {
    let colon = text.find(':')?;
    let key = &text[..colon];
    let plain = !key.is_empty() && !key.contains([' ', '{', '[', '"', '\'']);
    let ends = text[colon + 1..].is_empty() || text[colon + 1..].starts_with(' ');
    (plain && ends).then_some(colon + 1)
}

#[cfg(test)]
mod tests {
    fn texts(line: &str) -> Vec<String> {
        super::line(line, false)
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect()
    }

    #[test]
    fn splits_keys_values_and_comments() {
        assert_eq!(
            texts("  hourly: 15  # at :15"),
            ["  ", "hourly:", " 15 ", " # at :15"]
        );
        assert_eq!(texts("    - { this: x }"), ["    ", "- ", "{ this: x }"]);
        assert_eq!(texts("# note"), ["", "# note"]);
        assert_eq!(texts("url: http://x"), ["", "url:", " http://x"]);
    }
}
