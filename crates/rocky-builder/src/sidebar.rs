//! The left sidebar: the project's settings and the list of jobs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_yaml::{Mapping, Value};

use crate::doc::{self, Doc};
use crate::job;
use crate::project;
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
    let mut lines = vec![
        Line::styled(" PROJECT", theme::heading()),
        Line::from(vec![
            marker(on_project),
            Span::styled("Project", name_style(on_project)),
        ]),
        Line::styled(format!("  {values} values · env {env_file}"), theme::dim()),
        Line::styled(
            format!("  defaults {}", project::defaults_summary(&doc.root)),
            theme::dim(),
        ),
        Line::raw(""),
    ];
    let names = doc.job_names();
    lines.push(Line::styled(
        format!(" JOBS · {}", names.len()),
        theme::heading(),
    ));
    let mut selected_line = 1;
    for name in &names {
        let Some(job) = doc
            .root
            .get("jobs")
            .and_then(|jobs| jobs.get(name.as_str()))
        else {
            continue;
        };
        let on = *selection == Selection::Job(name.clone());
        if on {
            selected_line = lines.len() + 1;
        }
        let broken = problem.is_some_and(|problem| problem.job.as_ref() == Some(name));
        let running = scheduler.is_some_and(|scheduler| scheduler.is_running_job(name));
        let scheduled = job.get("schedule").is_some();
        let dot = if broken {
            Span::styled("✗ ", theme::error())
        } else if running {
            Span::styled("▶ ", theme::accent())
        } else if scheduled {
            Span::styled("● ", theme::ok())
        } else {
            Span::styled("○ ", theme::dim())
        };
        lines.push(Line::from(vec![
            marker(on),
            dot,
            Span::styled(name.clone(), name_style(on)),
        ]));
        lines.push(Line::styled(
            format!("    {}", job::summary(job)),
            theme::dim(),
        ));
    }
    let height = usize::from(area.height);
    let scroll = (selected_line + 2).saturating_sub(height);
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
