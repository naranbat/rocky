//! Conditions (`when`, `while`, `retry.until`): `this · operator · value`, and `all` /
//! `any` / `not` groups of them.

use serde_yaml::Value;

use crate::doc;
use crate::form::{Action, Field, Row};
use crate::node::{self, Path};

/// A new single condition, as added by `+ when`.
pub fn new_leaf() -> Value {
    doc::map_of([
        ("this", Value::from("tasks.NAME.ok")),
        ("eq", Value::from(true)),
    ])
}

/// The rows of the optional condition at `path`, or a row that adds one.
pub fn rows(root: &Value, path: &Path, label: &str, note: &str) -> Vec<Row> {
    match node::get(root, path) {
        Some(_) => vec![
            Row::edit(label, root, path.clone(), Field::Flow).removable(path.clone()),
            Row::hint(note),
        ],
        None => vec![Row::add(
            label,
            Action::Add(path.clone(), new_leaf(), Some(Field::Flow)),
        )],
    }
}
