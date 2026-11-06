//! The rows of a command task: `cmd`, its pipe, and the optional fields it sets.

use ratatui::text::Line;
use serde_yaml::Value;

use crate::condition;
use crate::doc;
use crate::form::{Action, Choice, Field, Row};
use crate::node::{self, Path};
use crate::theme;
use crate::vars;

/// Optional fields offered by `+ option`, with the value each starts with.
fn optional() -> [(&'static str, Value); 9] {
    [
        ("stdin", Value::from("")),
        ("env", doc::map_of([("NAME", Value::from(""))])),
        ("cwd", Value::from(".")),
        (
            "pipe",
            Value::Sequence(vec![Value::Sequence(vec![Value::from("cat")])]),
        ),
        ("stdout_file", Value::from("out.json")),
        ("capture_stdout", Value::from(false)),
        ("timeout", Value::from("30s")),
        (
            "retry",
            doc::map_of([("attempts", Value::from(3)), ("delay", Value::from("2s"))]),
        ),
        ("allow_failure", Value::from(true)),
    ]
}

pub fn rows(root: &Value, path: &Path) -> Vec<Row> {
    let Some(task) = node::get(root, path) else {
        return Vec::new();
    };
    let mut rows = args(root, &path.key("cmd"), "cmd");
    if let Some(stages) = task.get("pipe").and_then(Value::as_sequence) {
        for index in 0..stages.len() {
            let stage = path.key("pipe").index(index);
            rows.push(
                Row::edit("pipe |", root, stage.clone(), Field::Flow)
                    .removable(stage.clone())
                    .movable(stage),
            );
        }
        rows.push(Row::add(
            "pipe stage",
            Action::Add(
                path.key("pipe").index(stages.len()),
                Value::Sequence(vec![Value::from("cat")]),
                Some(Field::Flow),
            ),
        ));
    }
    for (key, field, hint) in [
        ("timeout", Field::Optional, ""),
        ("stdin", Field::Flow, "text, or YAML sent as JSON"),
        ("cwd", Field::Optional, "relative to the project file"),
        (
            "stdout_file",
            Field::Optional,
            "write stdout here; result has no stdout",
        ),
    ] {
        if task.get(key).is_some() {
            rows.push(Row::edit(key, root, path.key(key), field).removable(path.key(key)));
            if !hint.is_empty() {
                rows.push(Row::hint(hint));
            }
        }
    }
    for key in ["allow_failure", "capture_stdout"] {
        if let Some(on) = task.get(key) {
            let on = on.as_bool().unwrap_or(false);
            rows.push(
                Row::field(
                    key,
                    Line::styled(if on { "[x] on" } else { "[ ] off" }, theme::text()),
                    Action::Set(path.key(key), Some(Value::from(!on))),
                )
                .removable(path.key(key)),
            );
        }
    }
    if task.get("env").is_some() {
        rows.push(Row::hint("env"));
        rows.extend(vars::rows(root, &path.key("env"), "env"));
    }
    if task.get("retry").is_some() {
        rows.extend(retry(root, &path.key("retry")));
    }
    let missing: Vec<Choice> = optional()
        .into_iter()
        .filter(|(key, _)| task.get(*key).is_none())
        .map(|(key, value)| Choice::new(key, "", Action::Set(path.key(key), Some(value))))
        .collect();
    if !missing.is_empty() {
        let keys: Vec<&str> = missing.iter().map(|choice| choice.label.as_str()).collect();
        rows.push(Row::add(
            &keys.join(" · "),
            Action::Pick("Add option".to_string(), missing),
        ));
    }
    rows
}

/// A command as rows: the program, each argument, and `+ arg`. An argument left empty is
/// removed.
pub fn args(root: &Value, path: &Path, label: &str) -> Vec<Row> {
    let count = node::get(root, path)
        .and_then(Value::as_sequence)
        .map_or(0, Vec::len);
    let mut rows: Vec<Row> = (0..count)
        .map(|index| {
            let arg = path.index(index);
            let label = if index == 0 { label } else { "" };
            let field = if index == 0 {
                Field::Text
            } else {
                Field::Optional
            };
            Row::edit(label, root, arg.clone(), field)
                .removable(arg.clone())
                .movable(arg)
        })
        .collect();
    if let Some(program) = rows.first_mut() {
        let text = node::get(root, &path.index(0))
            .map(doc::scalar_text)
            .unwrap_or_default();
        program.value = Line::styled(text, theme::CMD);
    }
    rows.push(if count == 0 {
        Row::add(
            "program",
            Action::Add(
                path.clone(),
                Value::Sequence(vec![Value::from("echo")]),
                Some(Field::Flow),
            ),
        )
    } else {
        Row::add(
            "arg",
            Action::Add(path.index(count), Value::from(""), Some(Field::Optional)),
        )
    });
    rows
}

/// The rows of the optional `retry` at `path`: attempts, delay, and an optional `until`.
pub fn retry(root: &Value, path: &Path) -> Vec<Row> {
    if node::get(root, path).is_none() {
        return vec![Row::add(
            "retry",
            Action::Set(
                path.clone(),
                Some(doc::map_of([
                    ("attempts", Value::from(3)),
                    ("delay", Value::from("2s")),
                ])),
            ),
        )];
    }
    let mut rows = vec![
        Row::edit("retry attempts", root, path.key("attempts"), Field::Number)
            .removable(path.clone()),
        Row::edit("retry delay", root, path.key("delay"), Field::Optional).removable(path.clone()),
    ];
    rows.extend(condition::rows(
        root,
        &path.key("until"),
        "retry until",
        "retried until this holds",
        true,
    ));
    rows
}
