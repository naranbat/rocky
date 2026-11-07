//! The home screen's pane for a recent project: its jobs, whether it is valid, its last run.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph, Wrap};

use crate::chrome;
use crate::home::tilde;
use crate::recent::{Entry, Status};
use crate::theme::{self, fg};

/// The project's name: its folder's, or its project file's.
pub fn name(entry: &Entry) -> String {
    let path = &entry.path;
    let name = if path.extension().is_some_and(|ext| ext == "yaml") {
        path.file_stem()
    } else {
        path.file_name()
    };
    name.unwrap_or_default().to_string_lossy().into_owned()
}

/// The status in a few words and its color: `✓ valid`, `✗ invalid`.
pub fn status(status: &Status) -> (&'static str, ratatui::style::Color) {
    match status {
        Status::Valid => ("✓ valid", theme::OK),
        Status::Invalid(_) => ("✗ invalid", theme::ERROR),
        Status::Running => ("● running", theme::ACCENT),
        Status::Missing => ("✗ missing", theme::ERROR),
    }
}

pub fn draw(frame: &mut Frame, area: Rect, entry: &Entry) {
    let summary = &entry.summary;
    let [title, path, _, cards, _, body, button] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::styled(name(entry), theme::bold(theme::TEXT))),
        title,
    );
    frame.render_widget(
        Paragraph::new(Line::styled(tilde(&entry.path), fg(theme::MUTED))),
        path,
    );

    let (status_text, status_color) = status(&summary.status);
    let values = [
        ("Jobs", summary.names.len().to_string(), theme::TEXT),
        ("Scheduled", summary.scheduled.to_string(), theme::TEXT),
        ("Validate", status_text.to_string(), status_color),
    ];
    let areas = Layout::horizontal([Constraint::Fill(1); 3])
        .spacing(1)
        .split(cards);
    for ((label, value, color), area) in values.into_iter().zip(areas.iter()) {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(label, fg(theme::MUTED)),
                Line::styled(value, theme::bold(color)),
            ])
            .block(chrome::lines(Borders::ALL).padding(ratatui::widgets::Padding::horizontal(1))),
            *area,
        );
    }

    let mut lines = vec![
        Line::styled("Jobs", fg(theme::MUTED)),
        Line::styled(summary.names.join(", "), fg(theme::TEXT)),
        Line::default(),
    ];
    if let Status::Invalid(error) = &summary.status {
        lines.push(Line::styled("Error", fg(theme::MUTED)));
        lines.push(Line::styled(error.clone(), fg(theme::ERROR)));
        lines.push(Line::default());
    }
    lines.push(Line::styled("Last run", fg(theme::MUTED)));
    lines.push(match &summary.last_run {
        Some(run) if run.contains('✗') => Line::styled(run.clone(), fg(theme::ERROR)),
        Some(run) => Line::styled(run.clone(), fg(theme::OK)),
        None => Line::styled("—", fg(theme::MUTED)),
    });
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);

    let open = chrome::button(frame, button, "⏎ Open", true);
    let rest = Rect {
        x: button.x + open + 1,
        width: button.width.saturating_sub(open + 1),
        ..button
    };
    let validate = chrome::button(frame, rest, "v Validate", false);
    let hint = Rect {
        x: rest.x + validate,
        y: rest.y + 1,
        width: rest.width.saturating_sub(validate),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  d remove from recent",
            fg(theme::MUTED),
        ))),
        hint,
    );
}
