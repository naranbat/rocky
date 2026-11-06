//! The rows of a task file task: the file, its `with` inputs, and a preview of the file.

use serde_yaml::Value;

use crate::condition;
use crate::doc;
use crate::edit::Edit;
use crate::form::{Action, Field, Row};
use crate::node::{self, Path};
use crate::vars;

/// Lines of the task file to preview.
const PREVIEW_LINES: usize = 12;

/// The task at `path` is a mapping with `file`, or the `name: path` shorthand string.
pub fn rows(ed: &mut Edit, root: &Value, path: &Path) -> Vec<Row> {
    let Some(task) = node::get(root, path) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    let file = match task {
        Value::String(file) => {
            rows.push(Row::edit("file", root, path.clone(), Field::Text));
            // `with` and `when` need the mapping form; the shorthand string has neither.
            rows.push(Row::add(
                "with",
                Action::Set(
                    path.clone(),
                    Some(doc::map_of([
                        ("file", Value::from(file.as_str())),
                        ("with", doc::map_of([("NAME", Value::from(""))])),
                    ])),
                ),
            ));
            rows.push(Row::add(
                "when",
                Action::Set(
                    path.clone(),
                    Some(doc::map_of([
                        ("file", Value::from(file.as_str())),
                        ("when", condition::new_leaf()),
                    ])),
                ),
            ));
            file.clone()
        }
        task => {
            rows.push(Row::edit("file", root, path.key("file"), Field::Text));
            rows.push(Row::hint("with"));
            rows.extend(vars::rows(root, &path.key("with"), "with"));
            task.get("file").map(doc::scalar_text).unwrap_or_default()
        }
    };
    rows.push(Row::hint("relative to this file"));
    if !file.is_empty() {
        rows.push(Row::heading(&format!("FILE PREVIEW · {file}"), ""));
        let preview: Vec<String> = ed
            .preview(&file)
            .lines()
            .take(PREVIEW_LINES)
            .map(str::to_string)
            .collect();
        rows.extend(preview.iter().map(|line| Row::hint(line)));
        rows.push(Row::hint(
            "Only file, with and when go here; edit the rest in the file.",
        ));
    }
    rows
}
