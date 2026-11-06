//! The start screen, shown until a project is open: create a new project, open one by
//! path, open the example, or pick a recent project.

use std::path::{Path, PathBuf};

use jiff::{Timestamp, Zoned};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::home::{expand, tilde};
use crate::input::{self, Input};
use crate::new_project::{NewProject, Outcome};
use crate::recent::{Entry, Recent, Status};
use crate::scheduler;
use crate::theme;

/// The project opened by Open example, from this source tree.
const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/httpbin");

/// What the start screen asked for.
pub enum Pick {
    Open(PathBuf),
    /// A project was created at the path, but not opened.
    Created(PathBuf),
    /// Forget a project from the recent list.
    Forget(PathBuf),
    /// Read every project's summary again.
    Refresh,
    Quit,
}

/// What the keys go to.
#[derive(Default)]
enum Mode {
    #[default]
    List,
    Filter,
    /// Typing the path of a project to open.
    Open(Input),
    New(NewProject),
}

/// The start screen's state between frames.
#[derive(Default)]
pub struct Start {
    filter: Input,
    cursor: usize,
    mode: Mode,
    /// Where `rocky` is, looked up once.
    rocky: Option<Option<PathBuf>>,
}

impl Start {
    /// The recent projects that match the filter.
    fn shown<'a>(&self, recent: &'a Recent) -> Vec<&'a Entry> {
        let needle = self.filter.text.trim().to_lowercase();
        recent
            .entries
            .iter()
            .filter(|entry| {
                needle.is_empty()
                    || name(&entry.path).to_lowercase().contains(&needle)
                    || tilde(&entry.path).to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub fn handle(&mut self, key: KeyEvent, recent: &Recent) -> Option<Pick> {
        match &mut self.mode {
            Mode::New(form) => {
                match form.handle(key)? {
                    Outcome::Cancel => self.mode = Mode::List,
                    Outcome::Created { path, open } => {
                        self.mode = Mode::List;
                        return Some(if open {
                            Pick::Open(path)
                        } else {
                            Pick::Created(path)
                        });
                    }
                }
                return None;
            }
            Mode::Open(field) => {
                match field.handle(key) {
                    input::Outcome::Done => {
                        let path = expand(&field.text);
                        self.mode = Mode::List;
                        return Some(Pick::Open(path));
                    }
                    input::Outcome::Cancel => self.mode = Mode::List,
                    input::Outcome::Edited => {}
                }
                return None;
            }
            Mode::Filter => {
                match key.code {
                    KeyCode::Enter | KeyCode::Down => self.mode = Mode::List,
                    KeyCode::Esc => {
                        self.filter = Input::default();
                        self.mode = Mode::List;
                    }
                    _ => {
                        self.filter.handle(key);
                    }
                }
                self.cursor = 0;
                return None;
            }
            Mode::List => {}
        }
        let shown = self.shown(recent);
        let selected = shown.get(self.cursor).map(|entry| entry.path.clone());
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(shown.len().saturating_sub(1))
            }
            KeyCode::Enter => return selected.map(Pick::Open),
            KeyCode::Char('x') | KeyCode::Delete => return selected.map(Pick::Forget),
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Esc => self.filter = Input::default(),
            KeyCode::Char('n') => {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                self.mode = Mode::New(NewProject::new(&home));
            }
            KeyCode::Char('o') => {
                let cwd = std::env::current_dir().unwrap_or_default();
                self.mode = Mode::Open(Input::new(&format!("{}/", tilde(&cwd))));
            }
            KeyCode::Char('e') if Path::new(EXAMPLE).is_dir() => {
                return Some(Pick::Open(PathBuf::from(EXAMPLE)));
            }
            KeyCode::Char('r') => return Some(Pick::Refresh),
            KeyCode::Char('q') => return Some(Pick::Quit),
            _ => {}
        }
        None
    }

    /// Draws the start screen. `error` is the last failure, shown in the status line.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, recent: &Recent, error: Option<&str>) {
        let [main, status] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        let [side, list] =
            Layout::horizontal([Constraint::Length(42), Constraint::Fill(1)]).areas(main);
        let rocky = self.rocky.get_or_insert_with(scheduler::find_rocky).clone();
        frame.render_widget(
            Paragraph::new(sidebar(rocky.as_deref())).block(
                Block::new()
                    .borders(Borders::RIGHT)
                    .border_style(theme::dim()),
            ),
            side.inner(ratatui::layout::Margin::new(1, 1)),
        );
        self.draw_list(
            frame,
            list.inner(ratatui::layout::Margin::new(2, 1)),
            recent,
        );
        let status_line = match error {
            Some(error) => Line::styled(format!(" {error}"), theme::error()),
            None => Line::styled(
                " rocky validate runs when a project opens · the recent list is stored on this machine",
                theme::dim(),
            ),
        };
        frame.render_widget(Paragraph::new(status_line), status);
        match &self.mode {
            Mode::New(form) => form.draw(frame, main),
            Mode::Open(field) => {
                let width = main.width.min(80);
                let area = Rect::new(main.x + (main.width - width) / 2, main.y + 2, width, 3);
                frame.render_widget(Clear, area);
                let block = Block::bordered()
                    .title(Span::styled(" Open a project folder ", theme::bold()))
                    .title_bottom(Line::styled(" enter open · esc cancel ", theme::dim()))
                    .border_style(theme::accent());
                let inner = block.inner(area);
                frame.render_widget(block, area);
                field.draw(frame, inner);
            }
            Mode::List | Mode::Filter => {}
        }
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect, recent: &Recent) {
        let [head, rows] =
            Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);
        let filter = match (&self.mode, self.filter.text.is_empty()) {
            (Mode::Filter, _) => None,
            (_, true) => Some(Span::styled("/ filter by name or path", theme::dim())),
            (_, false) => Some(Span::styled(
                format!("/ {}", self.filter.text),
                theme::accent(),
            )),
        };
        let mut title = vec![
            Span::styled("RECENT PROJECTS ", theme::bold()),
            Span::styled(format!("{}   ", recent.entries.len()), theme::dim()),
        ];
        title.extend(filter);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(title),
                Line::raw(""),
                Line::styled(
                    format!("  {:<36}{:<26}{:<20}{}", "NAME", "JOBS", "STATUS", "OPENED"),
                    theme::dim(),
                ),
            ]),
            head,
        );
        if let Mode::Filter = self.mode {
            let x = 24 + u16::try_from(recent.entries.len().to_string().len()).unwrap_or(0);
            let field = Rect::new(head.x + x, head.y, head.width.saturating_sub(x), 1);
            self.filter.draw(frame, field);
        }
        if recent.entries.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::styled(
                    "No recent projects yet: create one (n) or open a project folder (o).",
                    theme::dim(),
                )),
                rows,
            );
            return;
        }
        let shown = self.shown(recent);
        self.cursor = self.cursor.min(shown.len().saturating_sub(1));
        let now = Zoned::now();
        let mut lines = Vec::new();
        for (index, entry) in shown.iter().enumerate() {
            lines.extend(row(entry, &now, index == self.cursor));
        }
        if shown.is_empty() {
            lines.push(Line::styled(
                "No recent project matches the filter.",
                theme::dim(),
            ));
        }
        if let Some(Status::Invalid(error)) =
            shown.get(self.cursor).map(|entry| &entry.summary.status)
        {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                format!("{error} — opens with the problem highlighted"),
                theme::error(),
            ));
        }
        let scroll = (self.cursor * 3 + 3).saturating_sub(usize::from(rows.height));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(0), 0)),
            rows,
        );
    }
}

/// Rocky's name and what it does, the ways to start, and whether `rocky` was found.
fn sidebar(rocky: Option<&Path>) -> Vec<Line<'static>> {
    let key = |key: &str, title: &str, detail: &str| {
        vec![
            Line::from(vec![
                Span::styled(format!(" {key} "), theme::accent()),
                Span::styled(title.to_string(), theme::bold()),
            ]),
            Line::styled(format!("   {detail}"), theme::dim()),
            Line::raw(""),
        ]
    };
    let mut lines = vec![
        Line::styled("rocky", theme::bold()),
        Line::styled("Build YAML jobs: commands, conditions,", theme::dim()),
        Line::styled("loops and schedules, then run them.", theme::dim()),
        Line::raw(""),
    ];
    lines.extend(key(
        "n",
        "New project…",
        "Start from a basic or full starter",
    ));
    lines.extend(key(
        "o",
        "Open project…",
        "Type the path of a project folder",
    ));
    if Path::new(EXAMPLE).is_dir() {
        lines.extend(key("e", "Open example", "examples/httpbin"));
    }
    lines.extend(key(
        "↵",
        "Open recent",
        "/ filter · x forget · r refresh · q quit",
    ));
    match rocky {
        Some(path) => {
            lines.push(Line::styled("● rocky CLI found", theme::ok()));
            lines.push(Line::styled(tilde(path), theme::dim()));
        }
        None => {
            lines.push(Line::styled("● rocky CLI not found", theme::error()));
            lines.push(Line::styled(
                "Running and scheduling need it on PATH",
                theme::dim(),
            ));
        }
    }
    lines
}

/// One recent project, on two lines and a gap.
fn row(entry: &Entry, now: &Zoned, selected: bool) -> Vec<Line<'static>> {
    let missing = matches!(entry.summary.status, Status::Missing);
    let summary = &entry.summary;
    let jobs = if missing {
        "—".to_string()
    } else {
        format!(
            "{} · {} scheduled",
            plural(summary.jobs, "job"),
            summary.scheduled
        )
    };
    let (status, style) = match &summary.status {
        Status::Running => (
            "● running · socket",
            ratatui::style::Style::new().fg(theme::WHILE),
        ),
        Status::Valid => ("● valid", theme::ok()),
        Status::Invalid(_) => ("● invalid", theme::error()),
        Status::Missing => ("○ file not found", theme::dim()),
    };
    let marker = if selected {
        Span::styled("▌ ", theme::accent())
    } else {
        Span::raw("  ")
    };
    let name_style = match (selected, missing) {
        (_, true) => theme::dim(),
        (true, false) => theme::accent(),
        (false, false) => theme::bold(),
    };
    vec![
        Line::from(vec![
            marker.clone(),
            Span::styled(format!("{:<36}", name(&entry.path)), name_style),
            Span::styled(format!("{jobs:<26}"), theme::text()),
            Span::styled(format!("{status:<20}"), style),
            Span::styled(ago(entry.opened, now), theme::dim()),
        ]),
        Line::from(vec![marker, Span::styled(tilde(&entry.path), theme::dim())]),
        Line::raw(""),
    ]
}

/// A project's name: its folder's.
fn name(path: &Path) -> String {
    path.file_name()
        .map(|part| part.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn plural(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        _ => format!("{count} {noun}s"),
    }
}

/// When a project was opened: `just now`, `2 min ago`, `5 h ago`, `yesterday`, `3 days ago`,
/// then the date.
fn ago(opened: Timestamp, now: &Zoned) -> String {
    let then = opened.to_zoned(now.time_zone().clone());
    let secs = now.timestamp().as_second() - opened.as_second();
    let days = (now.date() - then.date()).get_days();
    match (secs, days) {
        (..60, _) => "just now".to_string(),
        (..3600, _) => format!("{} min ago", secs / 60),
        (_, 0) => format!("{} h ago", secs / 3600),
        (_, 1) => "yesterday".to_string(),
        (_, ..7) => format!("{days} days ago"),
        _ if then.year() == now.year() => then.strftime("%b %-d").to_string(),
        _ => then.strftime("%b %-d, %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    /// Draws valid, invalid and missing projects, the filter, and the New project form.
    #[test]
    fn draws_the_list_and_the_form() {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("valid");
        let invalid = dir.path().join("invalid");
        for (project, cmd) in [(&valid, "[echo]"), (&invalid, "[]")] {
            std::fs::create_dir_all(project.join("jobs")).unwrap();
            std::fs::write(
                project.join("jobs/a.yaml"),
                format!("tasks: {{t: {{cmd: {cmd}}}}}\n"),
            )
            .unwrap();
        }
        let mut recent = Recent::none();
        for path in [&valid, &invalid, &dir.path().join("gone")] {
            recent.record(path).unwrap();
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut start = Start::default();
        let screen = |terminal: &mut Terminal<TestBackend>, start: &mut Start| {
            terminal
                .draw(|frame| start.draw(frame, frame.area(), &recent, None))
                .unwrap();
            format!("{:?}", terminal.backend().buffer())
        };
        let all = screen(&mut terminal, &mut start);
        assert!(all.contains("valid") && all.contains("invalid") && all.contains("gone"));
        for key in [KeyCode::Char('/'), KeyCode::Char('i'), KeyCode::Char('n')] {
            assert!(start.handle(KeyEvent::from(key), &recent).is_none());
        }
        let filtered = screen(&mut terminal, &mut start);
        assert!(filtered.contains("invalid") && !filtered.contains("gone"));
        start.handle(KeyEvent::from(KeyCode::Esc), &recent);
        start.handle(KeyEvent::from(KeyCode::Char('n')), &recent);
        assert!(screen(&mut terminal, &mut start).contains("New project"));
    }

    #[test]
    fn names_and_times() {
        assert_eq!(name(Path::new("/w/automation")), "automation");
        let now: Zoned = "2026-11-06T15:00:00[UTC]".parse().unwrap();
        let at = |text: &str| ago(text.parse::<Timestamp>().unwrap(), &now);
        assert_eq!(at("2026-11-06T14:59:30Z"), "just now");
        assert_eq!(at("2026-11-06T14:58:00Z"), "2 min ago");
        assert_eq!(at("2026-11-06T10:00:00Z"), "5 h ago");
        assert_eq!(at("2026-11-05T10:00:00Z"), "yesterday");
        assert_eq!(at("2026-11-03T10:00:00Z"), "3 days ago");
        assert_eq!(at("2026-10-28T10:00:00Z"), "Oct 28");
        assert_eq!(at("2025-10-28T10:00:00Z"), "Oct 28, 2025");
    }
}
