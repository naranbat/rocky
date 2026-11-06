//! How a run shows in the editor: a status line under the job header, a marker on each task
//! row, a summary in the task's own rows, and its output in a pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use serde_json::Value;

use crate::form::{Action, Row};
use crate::results::{End, Shown, TaskRun};
use crate::theme;

/// Longest preview of a result shown inline.
const PREVIEW_CHARS: usize = 60;

/// The run's state for `job`, under the job header.
pub fn job_status(run: Shown, job: &str) -> Option<Line<'static>> {
    if !run.covers(job) {
        return None;
    }
    let results = run.results;
    let (runs, failures) = results.job_counts(job);
    let secs = run.elapsed.as_secs_f32();
    let mut upto = run
        .until
        .filter(|_| run.job == job)
        .map(|task| format!(" · up to {task}"))
        .unwrap_or_default();
    if let Some(by) = run.by {
        upto.push_str(&format!(" · by the {by}"));
    }
    let mut spans = match &results.end {
        None => vec![Span::styled(
            format!("● RUNNING{upto} · {runs} task runs · {secs:.1}s"),
            theme::accent(),
        )],
        Some(End::Done { failed: None }) => vec![
            Span::styled("✓ LAST RUN OK ", theme::ok()),
            Span::styled(
                format!("{runs} task runs in {secs:.2}s{upto}"),
                theme::dim(),
            ),
        ],
        Some(End::Done {
            failed: Some(reason),
        }) => vec![Span::styled(
            format!("✗ LAST RUN FAILED {reason}"),
            theme::error(),
        )],
        Some(End::Error(error)) => {
            vec![Span::styled(format!("✗ RUN ERROR {error}"), theme::error())]
        }
    };
    if failures > 0 && results.end.is_some() {
        spans.push(Span::styled(
            format!(" · {failures} failed runs"),
            theme::dim(),
        ));
    }
    Some(Line::from(spans))
}

/// A short marker for task `name`'s last run, for its row in the task list.
pub fn marker(run: Shown, job: &str, name: &str) -> Option<Span<'static>> {
    let Some(task) = run.results.task(job, name) else {
        return (run.is_running() && run.job == job).then(|| Span::styled("…", theme::dim()));
    };
    let (text, style) = badge(task);
    Some(Span::styled(text, style))
}

/// The badge of a task's runs: what it did, in a word or two, and its color.
fn badge(task: &TaskRun) -> (String, Style) {
    if task.skipped {
        return ("⤼ skipped".to_string(), theme::dim());
    }
    if let Some(target) = &task.queued {
        return (format!("✓ queued {target}"), theme::ok());
    }
    if let Some(items) = task.items {
        return (format!("↻ {items} items"), theme::text());
    }
    if let Some((checks, _)) = task.check {
        return (format!("↻ {checks} checks"), theme::text());
    }
    let Some(last) = &task.last else {
        return (String::new(), theme::dim());
    };
    let ok = last["ok"] == Value::Bool(true);
    let label = match last.get("exit_code") {
        Some(code) => format!("exit {code}"),
        None => "error".to_string(),
    };
    let ms = last["ms"].as_u64().unwrap_or_default();
    let secs = ms as f64 / 1000.0;
    if ok {
        (format!("✓ {label} {secs:.2}s"), theme::ok())
    } else {
        (format!("✗ {label} {secs:.2}s"), theme::error())
    }
}

/// What task `name`'s last run did, under its own rows, and a row that opens its output.
pub fn task_rows(run: Shown, job: &str, name: &str) -> Vec<Row> {
    let Some(task) = run.results.task(job, name) else {
        if run.is_running() && run.job == job {
            return vec![Row::heading("LAST RUN", "waiting…")];
        }
        return Vec::new();
    };
    let (text, style) = badge(task);
    let mut rows = vec![
        Row::heading("LAST RUN", ""),
        Row::note(Line::from(vec![
            Span::styled(text, style),
            Span::styled(format!("  {}", detail(task, name)), theme::dim()),
        ])),
    ];
    if task
        .last
        .as_ref()
        .is_some_and(|event| !event["result"].is_null())
    {
        rows.push(
            Row::field(
                "output",
                Line::styled("view stdout · stderr · result (o)", theme::text()),
                Action::Output(name.to_string()),
            )
            .for_task(name),
        );
    }
    rows
}

/// The rest of a task's summary, after its badge.
fn detail(task: &TaskRun, name: &str) -> String {
    if task.skipped {
        return format!("when was false · tasks.{name} stays unset");
    }
    if let Some(target) = &task.queued {
        return format!("{target} runs after this job");
    }
    if task.items.is_some() {
        return "each item ran do".to_string();
    }
    if let Some((_, holds)) = task.check {
        return if holds {
            "still true"
        } else {
            "condition false, loop done"
        }
        .to_string();
    }
    let Some(last) = &task.last else {
        return String::new();
    };
    let ok = last["ok"] == Value::Bool(true);
    let mut text = (if ok { "ok" } else { "failed" }).to_string();
    if last["allowed"] == Value::Bool(true) {
        text.push_str(" · allowed to fail");
    }
    if task.runs > 1 {
        text.push_str(&format!(
            " · {} of {} runs ok",
            task.runs - task.failures,
            task.runs
        ));
    }
    if let Some(error) = last["error"].as_str() {
        text.push_str(&format!(" · {error}"));
    } else if let Some(stdout) = last["result"].get("stdout") {
        text.push_str(&format!(" · tasks.{name}.stdout = {}", preview(stdout)));
    }
    text
}

/// A value on one short line.
fn preview(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.lines().next().unwrap_or_default().to_string(),
        Value::Null => "(none)".to_string(),
        other => other.to_string(),
    };
    if text.chars().count() > PREVIEW_CHARS {
        format!("{}…", text.chars().take(PREVIEW_CHARS).collect::<String>())
    } else {
        text
    }
}

/// The output pane's state: which task, which tab, and how far it is scrolled.
pub struct Output {
    pub task: String,
    pub tab: usize,
    pub scroll: u16,
}

/// Draws task `output.task`'s last result over `area`: stdout, stderr or the whole result.
pub fn draw_output(frame: &mut Frame, area: Rect, run: Option<Shown>, job: &str, output: &Output) {
    let result = run
        .and_then(|run| run.results.task(job, &output.task))
        .and_then(|task| task.last.as_ref())
        .map(|event| event["result"].clone())
        .unwrap_or(Value::Null);
    let stderr = result["stderr"].as_str().unwrap_or_default();
    let tabs = [
        "stdout".to_string(),
        match stderr.lines().count() {
            0 => "stderr · empty".to_string(),
            1 => "stderr · 1 line".to_string(),
            n => format!("stderr · {n} lines"),
        },
        "result JSON".to_string(),
    ];
    let text = match output.tab {
        0 => match &result["stdout"] {
            Value::String(text) => text.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        },
        1 => stderr.to_string(),
        _ => serde_json::to_string_pretty(&result).unwrap_or_default(),
    };
    let mut title = vec![Span::styled(format!(" {} ", output.task), theme::bold())];
    for (index, tab) in tabs.iter().enumerate() {
        let style = if index == output.tab {
            theme::accent()
        } else {
            theme::dim()
        };
        title.push(Span::styled(format!(" {tab} "), style));
    }
    let block = Block::bordered()
        .title(Line::from(title))
        .title_bottom(Line::styled(
            " tab switch · ↑↓ scroll · esc close ",
            theme::dim(),
        ))
        .border_style(theme::dim());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::new().fg(theme::NUMBER))
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((output.scroll, 0)),
        area,
    );
}
