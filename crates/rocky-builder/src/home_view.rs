//! The home screen: start a new project, open one, or pick a recent one. The pane on the
//! right follows the selected row.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph};

use crate::chrome;
use crate::home::tilde;
use crate::new_pane::NewPane;
use crate::open_pane::OpenPane;
use crate::recent::Recent;
use crate::recent_pane;
use crate::theme::{self, fg};

/// How many recent projects the screen shows.
const SHOWN: usize = 5;

pub enum Action {
    None,
    Open(PathBuf),
    Quit,
}

pub struct Home {
    cwd: PathBuf,
    /// 0 New project, 1 Open project, then the recent projects.
    selected: usize,
    /// Keys go to the New or Open pane.
    in_pane: bool,
    new: NewPane,
    open: OpenPane,
    pub message: Option<String>,
}

impl Home {
    pub fn new(cwd: &Path) -> Home {
        Home {
            cwd: cwd.to_path_buf(),
            selected: 0,
            in_pane: false,
            new: NewPane::new(cwd),
            open: OpenPane::new(cwd),
            message: None,
        }
    }

    pub fn key(&mut self, key: KeyEvent, recent: &mut Recent) -> Action {
        self.message = None;
        let shown = recent.entries.len().min(SHOWN);
        if self.in_pane {
            if key.code == KeyCode::Esc {
                self.in_pane = false;
                return Action::None;
            }
            let picked = match self.selected {
                0 => self.new.key(key),
                _ => self.open.key(key),
            };
            return match picked {
                Ok(Some(path)) => Action::Open(path),
                Ok(None) => Action::None,
                Err(error) => {
                    self.message = Some(error);
                    Action::None
                }
            };
        }
        let entry = self.selected.checked_sub(2);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(shown + 1)
            }
            KeyCode::Char(c @ '1'..='5') => {
                let index = c as usize - '1' as usize;
                if index < shown {
                    self.selected = index + 2;
                }
            }
            KeyCode::Char('n') => {
                self.selected = 0;
                self.in_pane = true;
            }
            KeyCode::Char('o') => {
                self.selected = 1;
                self.in_pane = true;
            }
            KeyCode::Enter => match entry {
                Some(index) => return Action::Open(recent.entries[index].path.clone()),
                None => self.in_pane = true,
            },
            KeyCode::Char('v') if entry.is_some() => recent.refresh(),
            KeyCode::Char('d') => {
                if let Some(index) = entry {
                    let path = recent.entries[index].path.clone();
                    if let Err(error) = recent.remove(&path) {
                        self.message = Some(error);
                    }
                    self.selected = self.selected.min(recent.entries.len().min(SHOWN) + 1);
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
            _ => {}
        }
        Action::None
    }

    pub fn draw(&self, frame: &mut Frame, recent: &Recent) {
        let [top, body, footer] = chrome::screen(frame);
        chrome::top(
            frame,
            top,
            vec![Span::styled(tilde(&self.cwd), fg(theme::MUTED))],
            vec![Span::styled(
                format!("rocky {}", env!("CARGO_PKG_VERSION")),
                fg(theme::MUTED),
            )],
        );
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(38), Constraint::Min(0)]).areas(body);
        frame.render_widget(chrome::lines(Borders::RIGHT), left);
        self.draw_list(
            frame,
            Rect {
                width: left.width.saturating_sub(1),
                ..left
            },
            recent,
        );

        let pane = chrome::pad(right, 3, 1);
        match self.selected {
            0 => self.new.draw(frame, pane, self.in_pane),
            1 => self.open.draw(frame, pane, self.in_pane),
            n => recent_pane::draw(frame, pane, &recent.entries[n - 2]),
        }

        let keys: &[(&str, &str)] = if self.in_pane {
            &[("↵", "open"), ("⇥", "complete"), ("esc", "back")]
        } else {
            &[
                ("↑↓", "select"),
                ("1-5", "recent"),
                ("↵", "open"),
                ("n", "new"),
                ("o", "open path"),
                ("q", "quit"),
            ]
        };
        let message = self
            .message
            .iter()
            .map(|message| Span::styled(message.clone(), fg(theme::ERROR)))
            .collect();
        chrome::bar(frame, footer, theme::keys(keys, &["↵"]), message);
    }

    fn draw_list(&self, frame: &mut Frame, area: Rect, recent: &Recent) {
        let width = area.width.saturating_sub(4);
        let start = |key: &str, label: &str, command: &str, n: usize| {
            let selected = self.selected == n;
            let chip = if selected {
                theme::ACCENT
            } else {
                theme::MUTED
            };
            chrome::row(
                vec![chrome::spread(
                    vec![
                        Span::styled(format!(" {key} "), Style::new().fg(theme::BG).bg(chip)),
                        Span::styled(format!("  {label}"), theme::bold(theme::TEXT)),
                    ],
                    vec![Span::styled(command.to_string(), fg(theme::MUTED))],
                    width,
                )],
                selected,
                area.width,
            )
        };
        let mut lines = vec![
            Line::default(),
            Line::styled("  ◆ rocky", theme::bold(theme::ACCENT)),
            Line::styled("  Run jobs on a schedule, from YAML.", fg(theme::MUTED)),
            Line::default(),
            Line::from(vec![Span::raw("  "), chrome::heading("start")]),
        ];
        lines.extend(start("n", "New project", "rocky new project", 0));
        lines.extend(start("o", "Open project…", "rocky.yaml or -f FILE", 1));
        lines.push(Line::default());
        lines.push(chrome::spread(
            vec![Span::raw("  "), chrome::heading("recent")],
            vec![chrome::heading("last opened")],
            width + 2,
        ));
        let now = Timestamp::now();
        for (n, entry) in recent.entries.iter().take(SHOWN).enumerate() {
            let selected = self.selected == n + 2;
            let (status, color) = recent_pane::status(&entry.summary.status);
            let path = theme::fit(&tilde(&entry.path), width.saturating_sub(3) as usize);
            lines.extend(chrome::row(
                vec![
                    chrome::spread(
                        vec![
                            Span::styled(format!("{}  ", n + 1), fg(theme::MUTED)),
                            Span::styled(recent_pane::name(entry), theme::bold(theme::TEXT)),
                        ],
                        vec![
                            Span::styled(status, fg(color)),
                            Span::styled(
                                format!("{:>12}", ago(entry.opened, now)),
                                fg(theme::MUTED),
                            ),
                        ],
                        width,
                    ),
                    Line::styled(format!("   {path}"), fg(theme::MUTED)),
                ],
                selected,
                area.width,
            ));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// When `then` was, from `now`: `2 min ago`, `yesterday`, `Oct 21`.
fn ago(then: Timestamp, now: Timestamp) -> String {
    let seconds = now.duration_since(then).as_secs();
    let (minutes, hours, days) = (seconds / 60, seconds / 3600, seconds / 86400);
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{minutes} min ago"),
        3600..86400 => format!("{hours} h ago"),
        86400..172800 => "yesterday".to_string(),
        172800..604800 => format!("{days} days ago"),
        604800..1209600 => "last week".to_string(),
        _ => then
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%b %d")
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_long_ago() {
        let now: Timestamp = "2026-11-07T12:00:00Z".parse().unwrap();
        let before = |seconds: i64| now - jiff::SignedDuration::from_secs(seconds);
        assert_eq!(ago(before(5), now), "just now");
        assert_eq!(ago(before(120), now), "2 min ago");
        assert_eq!(ago(before(90_000), now), "yesterday");
        assert_eq!(ago(before(3 * 86_400), now), "3 days ago");
        assert_eq!(ago(before(10 * 86_400), now), "last week");
    }
}
