//! What every screen shares: the background, the top bar, the footer and the panes' frames.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme::{self, fg};

/// Paints the background and splits the screen into top bar, body and footer.
pub fn screen(frame: &mut Frame) -> [Rect; 3] {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::new().bg(theme::BG)), area);
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// A one-line bar on the panel color, with `left` and `right` aligned to its sides.
pub fn bar(frame: &mut Frame, area: Rect, left: Vec<Span<'static>>, right: Vec<Span<'static>>) {
    let style = Style::new().bg(theme::PANEL);
    frame.render_widget(Block::new().style(style), area);
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
    let right = Line::from(right);
    let width = (right.width() as u16).min(inner.width);
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(width)]).areas(inner);
    frame.render_widget(Paragraph::new(Line::from(left)), left_area);
    frame.render_widget(Paragraph::new(right), right_area);
}

/// The top bar: `◆ rocky`, then `left`, with `right` on the right.
pub fn top(frame: &mut Frame, area: Rect, left: Vec<Span<'static>>, right: Vec<Span<'static>>) {
    let mut spans = vec![Span::styled("◆ rocky  ", theme::bold(theme::ACCENT))];
    spans.extend(left);
    bar(frame, area, spans, right);
}

/// A frame with a line on the given sides.
pub fn lines(borders: Borders) -> Block<'static> {
    Block::new().borders(borders).border_style(fg(theme::LINE))
}

/// The area inside `area`, with `x` columns and `y` rows of margin.
pub fn pad(area: Rect, x: u16, y: u16) -> Rect {
    Rect {
        x: area.x + x.min(area.width),
        y: area.y + y.min(area.height),
        width: area.width.saturating_sub(2 * x),
        height: area.height.saturating_sub(2 * y),
    }
}

/// A small caps-style section heading: `TASKS · RUN IN THIS ORDER`.
pub fn heading(text: &str) -> Span<'static> {
    Span::styled(text.to_uppercase(), fg(theme::MUTED))
}

/// A line with `left` and `right` aligned to the sides of `width`.
pub fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let used = Line::from(left.clone()).width() + Line::from(right.clone()).width();
    let gap = (width as usize).saturating_sub(used).max(1);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    Line::from(spans)
}

/// `✓ valid` or `✗ invalid`, for the top bar.
pub fn validity(valid: bool) -> Span<'static> {
    if valid {
        Span::styled("✓ valid", fg(theme::OK))
    } else {
        Span::styled("✗ invalid", fg(theme::ERROR))
    }
}

/// A one-row button at the left of `area`: filled with the accent when `main`, with the
/// panel color otherwise. Returns its width.
pub fn button(frame: &mut Frame, area: Rect, text: &str, main: bool) -> u16 {
    let label = format!(" {text} ");
    let width = (Line::from(label.as_str()).width() as u16).min(area.width);
    let style = if main {
        theme::bold(theme::BG).bg(theme::ACCENT)
    } else {
        fg(theme::TEXT).bg(theme::SELECTED)
    };
    frame.render_widget(
        Paragraph::new(Line::styled(label, style)),
        Rect {
            width,
            height: area.height.min(1),
            ..area
        },
    );
    width
}

/// A row of tabs over a line, the selected one bold and underlined in the accent, with
/// `right` at the right. Takes two rows.
pub fn tabs(
    frame: &mut Frame,
    area: Rect,
    labels: &[String],
    selected: usize,
    right: Vec<Span<'static>>,
) {
    let [labels_area, line_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    let mut spans = vec![Span::raw("  ")];
    let mut x = labels_area.x + 2;
    let mut underline = (0, 0);
    for (n, label) in labels.iter().enumerate() {
        let text = format!(" {label} ");
        let width = Line::from(text.as_str()).width() as u16;
        let style = if n == selected {
            underline = (x, width);
            theme::bold(theme::TEXT)
        } else {
            fg(theme::MUTED)
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw("  "));
        x += width + 2;
    }
    let inner = Rect {
        width: labels_area.width.saturating_sub(1),
        ..labels_area
    };
    frame.render_widget(Paragraph::new(spread(spans, right, inner.width)), inner);
    frame.render_widget(chrome_line(line_area.width), line_area);
    let (x, width) = underline;
    if x < line_area.right() {
        let width = width.min(line_area.right() - x);
        frame.render_widget(
            Paragraph::new(Line::styled("━".repeat(width as usize), fg(theme::ACCENT))),
            Rect {
                x,
                width,
                ..line_area
            },
        );
    }
}

fn chrome_line(width: u16) -> Paragraph<'static> {
    Paragraph::new(Line::styled("─".repeat(width as usize), fg(theme::LINE)))
}

/// A sidebar row `width` wide: `content` with a blank line above and below. When
/// `selected`, all of it is highlighted, with the accent bar on the left.
pub fn row(content: Vec<Line<'static>>, selected: bool, width: u16) -> Vec<Line<'static>> {
    let pad = || Line::default();
    std::iter::once(pad())
        .chain(content)
        .chain(std::iter::once(pad()))
        .map(|line| {
            let bar = if selected {
                Span::styled("▌ ", fg(theme::ACCENT))
            } else {
                Span::raw("  ")
            };
            let mut spans = vec![bar];
            spans.extend(line.spans);
            // Spaces to the edge, so the highlight covers the whole row.
            let used = Line::from(spans.clone()).width();
            spans.push(Span::raw(" ".repeat((width as usize).saturating_sub(used))));
            let line = Line::from(spans).style(line.style);
            if selected {
                line.style(Style::new().bg(theme::SELECTED))
            } else {
                line
            }
        })
        .collect()
}
