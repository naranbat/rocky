//! A file's lines with line numbers: one part of it alone, or the whole file with that part
//! highlighted, and the cursor while editing.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::highlight;
use crate::theme::{self, fg};

pub struct Code<'a> {
    pub lines: &'a [String],
    /// The part in focus.
    pub focus: Range<usize>,
    /// Show only the part in focus; otherwise the whole file, the rest faint.
    pub only_focus: bool,
    /// The cursor (line, character), while editing.
    pub cursor: Option<(usize, usize)>,
}

impl Code<'_> {
    /// The lines shown: the part (stretched to the cursor) or every line.
    fn shown(&self) -> Range<usize> {
        if !self.only_focus {
            return 0..self.lines.len();
        }
        let end = match self.cursor {
            Some((row, _)) => self.focus.end.max(row + 1),
            None => self.focus.end,
        };
        self.focus.start..end.min(self.lines.len())
    }
}

pub fn draw(frame: &mut Frame, area: Rect, code: &Code) {
    let shown = code.shown();
    let height = area.height as usize;
    let anchor = code.cursor.map_or(code.focus.start, |(row, _)| row);
    let at = anchor.saturating_sub(shown.start);
    let top = if at + 4 <= height {
        0
    } else {
        at.saturating_sub(height / 3)
    };
    let lines: Vec<Line> = shown
        .skip(top)
        .take(height)
        .map(|row| line(code, row))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn line(code: &Code, row: usize) -> Line<'static> {
    let in_focus = code.focus.contains(&row);
    let marked = in_focus && !code.only_focus;
    let cursor = code.cursor.filter(|(at, _)| *at == row).map(|(_, col)| col);
    let bar = if marked {
        Span::styled("▌", fg(theme::ACCENT))
    } else {
        Span::raw(" ")
    };
    let number_color = if in_focus { theme::MUTED } else { theme::DIM };
    let mut spans = vec![
        bar,
        Span::styled(format!("{:>4}   ", row + 1), fg(number_color)),
    ];
    let text = highlight::line(&code.lines[row], !in_focus && !code.only_focus);
    match cursor {
        Some(col) => spans.extend(with_cursor(text, col)),
        None => spans.extend(text),
    }
    let line = Line::from(spans);
    if marked || cursor.is_some() {
        line.style(Style::new().bg(theme::SELECTED))
    } else {
        line
    }
}

/// `spans` with the character at `col` drawn as the cursor.
fn with_cursor(spans: Vec<Span<'static>>, col: usize) -> Vec<Span<'static>> {
    let cursor = Style::new().fg(theme::BG).bg(theme::ACCENT);
    let mut out = Vec::new();
    let mut at = 0;
    let mut placed = false;
    for span in spans {
        let chars: Vec<char> = span.content.chars().collect();
        if !placed && col < at + chars.len() {
            let i = col - at;
            out.push(Span::styled(
                chars[..i].iter().collect::<String>(),
                span.style,
            ));
            out.push(Span::styled(chars[i].to_string(), cursor));
            out.push(Span::styled(
                chars[i + 1..].iter().collect::<String>(),
                span.style,
            ));
            placed = true;
        } else {
            out.push(span);
        }
        at += chars.len();
    }
    if !placed {
        out.push(Span::styled(" ", cursor));
    }
    out
}
