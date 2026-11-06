//! The rows of `for_each` and `while` tasks: their own fields and a nested `do` list.

use ratatui::text::Line;
use serde_yaml::{Mapping, Value};

use crate::edit::Edit;
use crate::form::{Action, Choice, Field, Row};
use crate::node::{self, Path};
use crate::task;
use crate::theme;

pub fn for_each(ed: &mut Edit, root: &Value, path: &Path) -> Vec<Row> {
    let mut rows = vec![
        Row::edit("for each in", root, path.key("for_each"), Field::Var),
        Row::edit("as", root, path.key("as"), Field::Optional).removable(path.key("as")),
        Row::hint("a list, or a JSON array as text · as is optional, default ITEM"),
        on_failure(root, path, "skip to next item"),
    ];
    rows.extend(body(ed, root, path));
    rows
}

pub fn while_loop(ed: &mut Edit, root: &Value, path: &Path) -> Vec<Row> {
    let mut rows = vec![
        // The loop needs its condition, so it can't be removed.
        Row::edit("while", root, path.key("while"), Field::Flow),
        Row::hint("checked before each run"),
        Row::edit("at most", root, path.key("attempts"), Field::Number)
            .removable(path.key("attempts")),
        Row::hint("attempts, then fail; none means no limit"),
        on_failure(root, path, "go on with the next run"),
    ];
    rows.extend(body(ed, root, path));
    rows
}

/// The `on_failure` picker: `stop` (the default) or `next`.
fn on_failure(root: &Value, path: &Path, next: &str) -> Row {
    let key = path.key("on_failure");
    let current = node::get(root, &key).and_then(Value::as_str) == Some("next");
    let choices = vec![
        Choice::new("stop the loop", "", Action::Set(key.clone(), None)),
        Choice::new(next, "", Action::Set(key, Some(Value::from("next")))),
    ];
    Row::field(
        "a do task fails",
        Line::styled(
            if current { next } else { "stop the loop" }.to_string(),
            theme::text(),
        ),
        Action::Pick("When a do task fails".to_string(), choices),
    )
}

/// The `DO · n TASKS` heading and the nested task list.
fn body(ed: &mut Edit, root: &Value, path: &Path) -> Vec<Row> {
    let tasks = path.key("do");
    let count = node::get(root, &tasks)
        .and_then(Value::as_mapping)
        .map_or(0, Mapping::len);
    let plural = if count == 1 { "TASK" } else { "TASKS" };
    let mut rows = vec![Row::heading(
        &format!("DO · {count} {plural}"),
        "Enter opens a task",
    )];
    rows.extend(task::list(ed, root, &tasks, false));
    rows
}
