//! The project-level rows: values, env file, and defaults.

use serde_yaml::Value;

use crate::doc;
use crate::form::{Action, Field, Row};
use crate::node::Path;
use crate::task_cmd;
use crate::vars;

/// The project's `values`, `env_file` and `defaults`.
pub fn rows(root: &Value) -> Vec<Row> {
    let top = Path::default();
    let mut rows = vec![
        Row::heading("PROJECT", "shared by every job, job files included"),
        Row::heading(
            "VALUES",
            "read as ${NAME} by every job; a job's own value wins",
        ),
    ];
    rows.extend(vars::rows(root, &top.key("values"), "value"));
    rows.push(Row::heading("ENV FILE", "a .env file shared by all jobs"));
    rows.push(
        Row::edit("env_file", root, top.key("env_file"), Field::Optional)
            .removable(top.key("env_file")),
    );
    rows.push(Row::heading(
        "DEFAULTS",
        "every cmd task gets these unless it sets them",
    ));
    rows.extend(defaults(root, &top.key("defaults"), "defaults"));
    rows
}

/// The rows of a `defaults` mapping (timeout, cwd, env, retry), or a row that adds one.
pub fn defaults(root: &Value, path: &Path, label: &str) -> Vec<Row> {
    if crate::node::get(root, path).is_none() {
        return vec![Row::add(
            label,
            Action::Set(
                path.clone(),
                Some(doc::map_of([("timeout", Value::from("30s"))])),
            ),
        )];
    }
    let mut rows = Vec::new();
    for key in ["timeout", "cwd"] {
        rows.push(Row::edit(key, root, path.key(key), Field::Optional).removable(path.key(key)));
    }
    rows.push(Row::hint("env"));
    rows.extend(vars::rows(root, &path.key("env"), "env"));
    rows.extend(task_cmd::retry(root, &path.key("retry")));
    rows.push(Row::field(
        "",
        ratatui::text::Line::styled(format!("× remove {label}"), crate::theme::dim()),
        Action::Set(path.clone(), None),
    ));
    rows
}

/// The sidebar's one-line summary of the project's defaults.
pub fn defaults_summary(project: &Value) -> String {
    let Some(defaults) = project.get("defaults").and_then(Value::as_mapping) else {
        return "none".to_string();
    };
    match defaults.get("timeout") {
        Some(timeout) => format!("timeout {}", doc::scalar_text(timeout)),
        None => format!("{} set", defaults.len()),
    }
}
