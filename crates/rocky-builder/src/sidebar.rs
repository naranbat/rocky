//! The left sidebar: the project's settings and the list of jobs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_yaml::{Mapping, Value};

use crate::doc::{self, Doc};
use crate::job;
use crate::project;
use crate::schedule;
use crate::scheduler::Scheduler;
use crate::theme;
use crate::validate::Problem;

/// What the main area shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Project,
    Job(String),
}

/// Everything the sidebar lists, in order: the project, then each job.
pub fn items(doc: &Doc) -> Vec<Selection> {
    std::iter::once(Selection::Project)
        .chain(doc.job_names().into_iter().map(Selection::Job))
        .collect()
}

/// A job with one starter task, as added by `a`.
pub fn new_job() -> Value {
    doc::map_of([(
        "tasks",
        doc::map_of([(
            "hello",
            doc::map_of([(
                "cmd",
                Value::Sequence(vec![Value::from("echo"), Value::from("hello")]),
            )]),
        )]),
    )])
}

/// The column where entry names and summaries start: after the marker and a status dot.
const TEXT: usize = 4;

/// A sidebar entry: its name line and summary lines, each after the marker.
struct Entry {
    on: bool,
    lines: Vec<Vec<Span<'static>>>,
}

/// Pushes `entries` with a gap line before, between and after them. The selected entry is on
/// the selection background across `width`, padded since a line's background only covers its
/// text, and the gaps beside it hold half a line of that background (`▄` above, `▀` below)
/// as padding. Returns the number of lines through the selected entry's lower gap, if one
/// is selected.
fn section(
    lines: &mut Vec<Line<'static>>,
    entries: Vec<Entry>,
    width: usize,
    marker: &dyn Fn(bool) -> Span<'static>,
) -> Option<usize> {
    let half = |block: &str| Line::styled(block.repeat(width), Style::new().fg(theme::SELECTED));
    let gap = |above: bool, below: bool| match (above, below) {
        (true, _) => half("▀"),
        (_, true) => half("▄"),
        _ => Line::raw(""),
    };
    let mut above = false;
    let mut end = None;
    for entry in entries {
        if above {
            end = Some(lines.len() + 1);
        }
        lines.push(gap(above, entry.on));
        for spans in entry.lines {
            let mut line = Line::from(spans);
            line.spans.insert(0, marker(entry.on));
            if entry.on {
                let pad = width.saturating_sub(line.width());
                line.push_span(Span::raw(" ".repeat(pad)));
                line = line.bg(theme::SELECTED);
            }
            lines.push(line);
        }
        above = entry.on;
    }
    if above {
        end = Some(lines.len() + 1);
    }
    lines.push(gap(above, false));
    end
}

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    doc: &Doc,
    selection: &Selection,
    focused: bool,
    problem: Option<&Problem>,
    scheduler: Option<&Scheduler>,
) {
    let marker = |on: bool| {
        if on && focused {
            Span::styled("▌", theme::accent())
        } else if on {
            Span::styled("▌", theme::dim())
        } else {
            Span::raw(" ")
        }
    };
    let name_style = |on: bool| if on { theme::accent() } else { theme::text() };
    let width = usize::from(area.width.saturating_sub(1));
    // Before a summary line: spaces up to `TEXT`, after the marker.
    let indent = || Span::raw(" ".repeat(TEXT - 1));
    let values = doc
        .root
        .get("values")
        .and_then(Value::as_mapping)
        .map_or(0, Mapping::len);
    let env_file = doc
        .root
        .get("env_file")
        .map(doc::scalar_text)
        .unwrap_or_else(|| "none".into());
    let on_project = *selection == Selection::Project;
    let project_entry = Entry {
        on: on_project,
        lines: vec![
            vec![indent(), Span::styled("Project", name_style(on_project))],
            vec![
                indent(),
                Span::styled(format!("{values} values · env {env_file}"), theme::dim()),
            ],
            vec![
                indent(),
                Span::styled(
                    format!("defaults {}", project::defaults_summary(&doc.root)),
                    theme::dim(),
                ),
            ],
        ],
    };
    let names = doc.job_names();
    let mut jobs = Vec::new();
    for name in &names {
        let Some(job) = doc
            .root
            .get("jobs")
            .and_then(|jobs| jobs.get(name.as_str()))
        else {
            continue;
        };
        let on = *selection == Selection::Job(name.clone());
        let broken = problem.is_some_and(|problem| problem.job.as_ref() == Some(name));
        let running = scheduler.is_some_and(|scheduler| scheduler.is_running_job(name));
        let scheduled = job.get("schedule").is_some();
        // The dot sits in the indent, so the name lines up with the summary below it.
        let dot = if broken {
            Span::styled(" ⦸ ", theme::error())
        } else if running {
            Span::styled(" ▶ ", theme::accent())
        } else if scheduled {
            // A clock, not a green dot: green dots mean running or valid elsewhere.
            Span::styled(" ◷ ", theme::text())
        } else {
            Span::styled(" ○ ", theme::dim())
        };
        jobs.push(Entry {
            on,
            lines: vec![
                vec![dot, Span::styled(name.clone(), name_style(on))],
                // The schedule on the left, the task count on the right, one column short
                // of the border.
                {
                    let schedule = schedule::summary(job.get("schedule"));
                    let count = format!("{} tasks", job::task_count(job));
                    let gap = width
                        .saturating_sub(TEXT + schedule.chars().count() + count.chars().count() + 1)
                        .max(1);
                    vec![
                        indent(),
                        Span::styled(
                            format!("{schedule}{}{count}", " ".repeat(gap)),
                            theme::dim(),
                        ),
                    ]
                },
            ],
        });
    }
    let mut lines = vec![Line::styled(" PROJECT", theme::heading())];
    section(&mut lines, vec![project_entry], width, &marker);
    lines.push(Line::styled(
        format!(" JOBS · {}", names.len()),
        theme::heading(),
    ));
    // Through the selected job and its padding, to scroll it into view.
    let selected_end = section(&mut lines, jobs, width, &marker).unwrap_or(0);
    let height = usize::from(area.height);
    let scroll = selected_end.saturating_sub(height);
    let border = if focused {
        theme::accent()
    } else {
        theme::dim()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::new().borders(Borders::RIGHT).border_style(border))
            .scroll((u16::try_from(scroll).unwrap_or(0), 0)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    /// The selected job's name and summary lines, and only those, are on the selection
    /// background across the sidebar's width.
    #[test]
    fn highlights_the_whole_selected_entry() {
        let dir = tempfile::tempdir().unwrap();
        crate::parts::write(
            dir.path(),
            "",
            "jobs:\n  a: {tasks: {t: {cmd: [echo]}}}\n  b: {schedule: {every: 5m}, tasks: {t: {cmd: [echo]}}}\n",
        )
        .unwrap();
        let doc = Doc::open(dir.path()).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(34, 20)).unwrap();
        let selection = Selection::Job("b".to_string());
        terminal
            .draw(|frame| draw(frame, frame.area(), &doc, &selection, true, None, None))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let highlighted: Vec<String> = (0..20)
            .filter(|&y| buffer[(32, y)].bg == theme::SELECTED)
            .map(|y| (0..33).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect();
        assert_eq!(highlighted.len(), 2, "{highlighted:?}");
        // The name and the summary start in the same column.
        let column = |line: &str, text: &str| {
            line.chars().count() - line[line.find(text).unwrap()..].chars().count()
        };
        assert_eq!(column(&highlighted[0], "b"), TEXT, "{highlighted:?}");
        // A scheduled job's icon is a clock, not a green dot.
        assert!(highlighted[0].contains("◷ b"), "{highlighted:?}");
        assert_eq!(column(&highlighted[1], "every 5m"), TEXT, "{highlighted:?}");
        // The task count ends one column short of the border.
        assert!(highlighted[1].ends_with("1 tasks "), "{highlighted:?}");
        // Half a line of padding above and below, and one gap line between entries.
        let row = |y: u16| (0..33).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        let name = (0..20).find(|&y| row(y).contains(" b")).unwrap();
        assert_eq!(row(name - 1), "▄".repeat(33));
        assert_eq!(row(name + 2), "▀".repeat(33));
        assert_eq!(buffer[(0, name - 1)].fg, theme::SELECTED);
        assert!(row(name - 2).contains("no schedule"), "{}", row(name - 2));
    }
}
