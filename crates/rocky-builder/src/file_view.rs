//! The Project, Values and Env file tabs: the project file, its `values` block, and the env
//! file, as text to edit in place.

use std::ops::Range;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph};

use crate::chrome;
use crate::code::{self, Code};
use crate::editor::{Action, Editor, Tab, Target};
use crate::outline;
use crate::theme::{self, fg};

/// The file the tab shows.
fn target(editor: &Editor) -> Option<Target> {
    match editor.tab {
        Tab::Env => editor.files.env.as_ref().map(|_| Target::Env),
        _ => Some(Target::Settings),
    }
}

/// The lines the tab shows: the `values` block in the Values tab, else the whole file.
fn focus(editor: &Editor, lines: &[String]) -> Option<Range<usize>> {
    match editor.tab {
        Tab::Values => outline::block(lines, "values"),
        _ => Some(0..lines.len()),
    }
}

pub fn key(editor: &mut Editor, key: KeyEvent) -> Action {
    let Some(target) = target(editor) else {
        if key.code == KeyCode::Char('q') {
            return editor.quit();
        }
        editor.switch_tab(key);
        return Action::None;
    };
    let editing = editor.editing;
    let tab_focus = editor
        .buffer(target)
        .and_then(|buffer| focus(editor, buffer.edit.lines()));
    let Some(range) = tab_focus else {
        // The Values tab without a `values` block.
        if key.code == KeyCode::Char('q') {
            return editor.quit();
        }
        editor.switch_tab(key);
        return Action::None;
    };
    let Some(edit) = editor.buffer_mut(target).map(|buffer| &mut buffer.edit) else {
        return Action::None;
    };
    if editing {
        if key.code == KeyCode::Esc {
            editor.editing = false;
        } else {
            let rows = range.start..range.end.max(edit.row + 1);
            edit.key(key, rows);
        }
        return Action::None;
    }
    match key.code {
        KeyCode::Char('q') => return editor.quit(),
        KeyCode::Char('i') | KeyCode::Enter => {
            edit.checkpoint();
            if !range.contains(&edit.row) {
                let row = outline::start_line(edit.lines(), range);
                edit.place(row, usize::MAX);
            }
            editor.editing = true;
        }
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
            edit.key(key, range);
        }
        KeyCode::Char('u') => editor.undo(target),
        KeyCode::Char('e') => editor.open_external(target),
        _ => {
            editor.switch_tab(key);
        }
    }
    Action::None
}

pub fn keys(editor: &Editor) -> Vec<Span<'static>> {
    if editor.editing {
        return theme::keys(&[("esc", "stop editing"), ("^S", "save")], &["^S"]);
    }
    theme::keys(
        &[
            ("↑↓", "move"),
            ("i", "edit"),
            ("^S", "save"),
            ("u", "undo"),
            ("e", "$EDITOR"),
            ("1-4", "tabs"),
            ("q", "quit"),
        ],
        &["^S"],
    )
}

pub fn draw(editor: &Editor, frame: &mut Frame, area: Rect) {
    let area = chrome::pad(area, 2, 1);
    let note = |frame: &mut Frame, text: &str| {
        frame.render_widget(
            Paragraph::new(Line::styled(text.to_string(), fg(theme::MUTED))),
            area,
        )
    };
    let Some(buffer) = target(editor).and_then(|target| editor.buffer(target)) else {
        note(
            frame,
            "The project file names no `env_file`. Add one in 2 Project.",
        );
        return;
    };
    let lines = buffer.edit.lines();
    let Some(range) = focus(editor, lines) else {
        note(
            frame,
            "The project file has no `values:` block. Add one in 2 Project.",
        );
        return;
    };
    let [head, _, code_area, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(area);
    let mut info = vec![
        Span::styled(editor.shown_path(&buffer.path), theme::bold(theme::TEXT)),
        Span::styled(format!("  {} lines", lines.len()), fg(theme::MUTED)),
    ];
    if buffer.is_dirty() {
        info.push(Span::styled("  ● modified", fg(theme::ACCENT)));
    }
    frame.render_widget(Paragraph::new(Line::from(info)), head);
    let cursor = (buffer.edit.row, buffer.edit.col);
    code::draw(
        frame,
        code_area,
        &Code {
            lines,
            focus: range,
            only_focus: true,
            cursor: editor.editing.then_some(cursor),
        },
    );
    let (text, color) = match &editor.problem {
        Some(problem) => (format!("✗ {}", problem.message), theme::ERROR),
        None => (
            format!("✓ project loads · {} jobs", editor.files.jobs.len()),
            theme::OK,
        ),
    };
    frame.render_widget(chrome::lines(Borders::TOP), status);
    frame.render_widget(
        Paragraph::new(Line::styled(
            theme::fit(&text, status.width as usize),
            fg(color),
        )),
        Rect {
            y: status.y + 1,
            height: 1,
            ..status
        },
    );
}
