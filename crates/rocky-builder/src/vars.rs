//! `NAME = value` rows for a mapping of variables (`values`, `env`, `with`).

use serde_yaml::{Mapping, Value};

use crate::doc;
use crate::form::{Action, Field, Row};
use crate::node::{self, Path};
use crate::theme;

/// A row per entry of the mapping at `path` (Enter edits the value, `r` renames, `x`
/// removes), and a `+ NAME` row. Removing the last entry removes the mapping.
pub fn rows(root: &Value, path: &Path, label: &str) -> Vec<Row> {
    let map = node::get(root, path).and_then(Value::as_mapping);
    let names = map.map(doc::keys).unwrap_or_default();
    let mut rows: Vec<Row> = names
        .iter()
        .map(|name| {
            let entry = path.key(name);
            let remove = if names.len() == 1 {
                path.clone()
            } else {
                entry.clone()
            };
            Row::edit(name, root, entry.clone(), Field::Scalar)
                .label_style(theme::accent())
                .removable(remove)
                .renamable(entry.clone())
                .movable(entry)
        })
        .collect();
    let fresh = doc::fresh_key(map.unwrap_or(&Mapping::new()), "NAME");
    rows.push(Row::add(
        &format!("{label} NAME"),
        Action::Add(path.key(&fresh), Value::from(""), None),
    ));
    rows
}
