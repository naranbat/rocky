//! The condition rows of `when`, `while` and `retry.until`: `if this · is operator · value`
//! for a single condition, and `all` / `any` / `not` groups of them, nested.

use ratatui::text::{Line, Span};
use serde_yaml::{Mapping, Value};

use crate::doc;
use crate::form::{Action, Choice, Field, Row};
use crate::node::{self, Path};
use crate::theme;

/// Comparison operators: key, label, and whether it compares numbers.
const OPERATORS: [(&str, &str, bool); 9] = [
    ("gt", "Greater than", true),
    ("gte", "At least", true),
    ("lt", "Less than", true),
    ("lte", "At most", true),
    ("eq", "Equal to", false),
    ("ne", "Not equal to", false),
    ("contains", "Contains", false),
    ("matches", "Matches regex", false),
    ("exists", "Exists", false),
];

/// A new single condition, as added by `+ when`.
pub fn new_leaf() -> Value {
    doc::map_of([
        ("this", Value::from("tasks.NAME.ok")),
        ("eq", Value::from(true)),
    ])
}

/// The rows of the condition at `path`, or a row that adds one. The first row edits the
/// whole condition as one line of YAML; `removable` lets `x` remove it.
pub fn rows(root: &Value, path: &Path, label: &str, note: &str, removable: bool) -> Vec<Row> {
    let Some(condition) = node::get(root, path) else {
        return vec![Row::add(label, Action::Set(path.clone(), Some(new_leaf())))];
    };
    let mut first = Row::field(
        label,
        Line::from(vec![
            Span::styled(summary(condition), theme::accent()),
            Span::styled(format!(" · {note} · enter edits as YAML"), theme::dim()),
        ]),
        Action::Edit(path.clone(), Field::Flow),
    )
    .label_style(theme::accent());
    let remove = removable.then(|| path.clone());
    if let Some(remove) = &remove {
        first = first.removable(remove.clone());
    }
    let mut rows = vec![first];
    node_rows(root, path, 1, remove.as_ref(), &mut rows);
    rows
}

/// What a condition asks, in a few words.
fn summary(condition: &Value) -> String {
    match group(condition) {
        Some(("all", items)) => format!("all of {} hold", items.len()),
        Some((_, items)) => format!("any of {} holds", items.len()),
        None if is_not(condition) => "unless it holds".to_string(),
        None => "one condition".to_string(),
    }
}

/// The kind and items of an `all` / `any` group (a list is an `all`), or `None` for a
/// single condition or a `not`.
fn group(condition: &Value) -> Option<(&'static str, &Vec<Value>)> {
    match condition {
        Value::Sequence(items) => Some(("all", items)),
        Value::Mapping(map) if map.len() == 1 => match (map.get("all"), map.get("any")) {
            (Some(Value::Sequence(items)), _) => Some(("all", items)),
            (_, Some(Value::Sequence(items))) => Some(("any", items)),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `condition` is `{not: …}`.
fn is_not(condition: &Value) -> bool {
    condition
        .as_mapping()
        .is_some_and(|map| map.len() == 1 && map.contains_key("not"))
}

/// The label of a row `depth` levels into a condition.
fn indent(depth: usize, label: &str) -> String {
    format!("{}{label}", "  ".repeat(depth))
}

/// Adds the rows of the condition at `path`. `remove` is what `x` removes from them.
fn node_rows(root: &Value, path: &Path, depth: usize, remove: Option<&Path>, rows: &mut Vec<Row>) {
    let Some(condition) = node::get(root, path) else {
        return;
    };
    let start = rows.len();
    if let Some((kind, items)) = group(condition) {
        group_rows(root, path, condition, kind, items, depth, rows);
    } else if is_not(condition) {
        let inner = condition.get("not").cloned();
        rows.push(Row::field(
            &indent(depth, "NOT"),
            Line::styled("unless this holds · enter removes the NOT", theme::dim()),
            Action::Set(path.clone(), inner),
        ));
        node_rows(root, &path.key("not"), depth + 1, remove, rows);
    } else {
        leaf_rows(root, path, condition, depth, rows);
    }
    if let Some(remove) = remove {
        for row in &mut rows[start..] {
            if row.remove.is_none() && !row.value.to_string().trim_start().starts_with("+ ") {
                row.remove = Some(remove.clone());
            }
        }
    }
}

/// An `all` / `any` group: its kind (Enter picks another), its conditions, and add rows.
fn group_rows(
    root: &Value,
    path: &Path,
    condition: &Value,
    kind: &str,
    items: &[Value],
    depth: usize,
    rows: &mut Vec<Row>,
) {
    let list = match condition {
        Value::Sequence(_) => path.clone(),
        _ => path.key(kind),
    };
    let regroup = |kind: &str| Some(doc::map_of([(kind, Value::Sequence(items.to_vec()))]));
    let choices = vec![
        Choice::new(
            "ALL",
            "every one holds",
            Action::Set(path.clone(), regroup("all")),
        ),
        Choice::new(
            "ANY",
            "at least one holds",
            Action::Set(path.clone(), regroup("any")),
        ),
        Choice::new(
            "NOT",
            "negate this group",
            Action::Set(
                path.clone(),
                Some(doc::map_of([("not", condition.clone())])),
            ),
        ),
    ];
    let hint = if kind == "all" {
        "every one holds"
    } else {
        "at least one holds"
    };
    rows.push(Row::field(
        &indent(depth, &kind.to_uppercase()),
        Line::styled(format!("{hint} · {} conditions", items.len()), theme::dim()),
        Action::Pick("Group".to_string(), choices),
    ));
    for index in 0..items.len() {
        let item = list.index(index);
        let start = rows.len();
        node_rows(root, &item, depth + 1, Some(&item), rows);
        if let Some(row) = rows.get_mut(start) {
            row.shift = Some(item);
        }
    }
    let add = |what: &str, action: Action| {
        Row::field(
            "",
            Line::styled(indent(depth + 1, &format!("+ {what}")), theme::dim()),
            action,
        )
    };
    rows.push(add(
        "condition",
        Action::Set(list.index(items.len()), Some(new_leaf())),
    ));
    rows.push(add(
        "group",
        Action::Set(
            list.index(items.len()),
            Some(doc::map_of([(
                "any",
                Value::Sequence(vec![new_leaf(), new_leaf()]),
            )])),
        ),
    ));
}

/// One `this · operator · value` condition. Several operators make a range (`gt 1, lt 10`).
fn leaf_rows(root: &Value, path: &Path, condition: &Value, depth: usize, rows: &mut Vec<Row>) {
    let empty = Mapping::new();
    let map = condition.as_mapping().unwrap_or(&empty);
    rows.push(Row::edit(
        &indent(depth, "if"),
        root,
        path.key("this"),
        Field::Var,
    ));
    let present: Vec<&str> = OPERATORS
        .iter()
        .map(|(key, _, _)| *key)
        .filter(|key| map.contains_key(*key))
        .collect();
    if present.is_empty() {
        rows.push(Row::field(
            &indent(depth, "is"),
            Line::styled("no operator · enter picks one", theme::error()),
            Action::Pick(
                "Operator".to_string(),
                operator_choices(path, condition, map, None, false),
            ),
        ));
    }
    for key in &present {
        let label = OPERATORS
            .iter()
            .find(|(op, _, _)| op == key)
            .map_or(*key, |(_, label, _)| label);
        rows.push(Row::field(
            &indent(depth, "is"),
            Line::styled(format!("{label} ({key})"), theme::value()),
            Action::Pick(
                "Operator".to_string(),
                operator_choices(path, condition, map, Some(key), present.len() > 1),
            ),
        ));
        let value = path.key(key);
        if *key == "exists" {
            let on = map.get("exists").and_then(Value::as_bool).unwrap_or(true);
            rows.push(Row::field(
                &indent(depth, ""),
                Line::styled(on.to_string(), theme::NUMBER),
                Action::Set(value, Some(Value::from(!on))),
            ));
        } else {
            rows.push(Row::edit(&indent(depth, ""), root, value, Field::Scalar));
        }
    }
}

/// What the operator row of a leaf offers: other operators in place of `key` (or, with
/// `None`, to add), adding a second operator for a range, removing `key`, negating, and
/// wrapping in a group.
fn operator_choices(
    path: &Path,
    condition: &Value,
    map: &Mapping,
    key: Option<&str>,
    removable: bool,
) -> Vec<Choice> {
    let mut choices: Vec<Choice> = OPERATORS
        .iter()
        .filter(|(op, _, _)| Some(*op) == key || !map.contains_key(*op))
        .map(|(op, label, numeric)| {
            let mut new = map.clone();
            match (key, *op) {
                (_, "exists") => {
                    new.retain(|k, _| k.as_str() == Some("this"));
                    new.insert(Value::from("exists"), Value::from(true));
                }
                (Some("exists"), op) | (None, op) => {
                    new.shift_remove("exists");
                    new.insert(Value::from(op), start_value(*numeric));
                }
                (Some(key), op) => {
                    doc::rename(&mut new, key, op);
                    // A number operator needs a number.
                    if *numeric && new.get(op).is_some_and(|value| !value.is_number()) {
                        new.insert(Value::from(op), start_value(true));
                    }
                }
            }
            let detail = if *numeric { "numbers" } else { "any value" };
            Choice::new(
                &format!("{label} ({op})"),
                detail,
                Action::Set(path.clone(), Some(Value::Mapping(new))),
            )
        })
        .collect();
    if let Some((op, _, numeric)) = OPERATORS[..8]
        .iter()
        .find(|(op, _, _)| !map.contains_key(*op))
        .filter(|_| !map.contains_key("exists") && key.is_some())
    {
        choices.push(Choice::new(
            "+ operator",
            "add another, to make a range",
            Action::Add(path.key(op), start_value(*numeric), Some(Field::Scalar)),
        ));
    }
    if let (Some(key), true) = (key, removable) {
        choices.push(Choice::new(
            &format!("× {key}"),
            "remove this operator",
            Action::Set(path.key(key), None),
        ));
    }
    let wrap = |kind: &str| {
        let wrapped = if kind == "not" {
            condition.clone()
        } else {
            Value::Sequence(vec![condition.clone()])
        };
        Action::Set(path.clone(), Some(doc::map_of([(kind, wrapped)])))
    };
    choices.push(Choice::new("NOT", "negate this condition", wrap("not")));
    choices.push(Choice::new(
        "ALL group",
        "wrap it in an all group",
        wrap("all"),
    ));
    choices.push(Choice::new(
        "ANY group",
        "wrap it in an any group",
        wrap("any"),
    ));
    choices
}

/// What a new operator compares against until it is edited.
fn start_value(numeric: bool) -> Value {
    if numeric {
        Value::from(0)
    } else {
        Value::from("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| format!("{} {}", row.label.content.trim(), row.value))
            .collect()
    }

    #[test]
    fn lists_nested_groups_and_ranges() {
        let root: Value = serde_yaml::from_str(
            "when: {any: [{this: A, gt: 1, lt: 9}, {not: {this: B, exists: true}}]}",
        )
        .unwrap();
        let path = Path::default().key("when");
        let rows = rows(&root, &path, "when", "otherwise skipped", true);
        let text = labels(&rows);
        assert!(text[0].starts_with("when any of 2 holds"));
        assert!(text.contains(&"ANY at least one holds · 2 conditions".to_string()));
        assert!(text.contains(&"is Greater than (gt)".to_string()));
        assert!(text.contains(&"is Less than (lt)".to_string()));
        assert!(text.contains(&"NOT unless this holds · enter removes the NOT".to_string()));
        assert!(text.contains(&" true".to_string()));
        // Each condition in the group removes itself; the group's rows remove the whole.
        let gt = rows
            .iter()
            .find(|row| row.value.to_string() == "Greater than (gt)");
        assert_eq!(
            gt.and_then(|row| row.remove.clone()),
            Some(path.key("any").index(0))
        );
        assert_eq!(rows[1].remove, Some(path.clone()));
    }

    #[test]
    fn operator_choices_keep_the_condition_valid() {
        let leaf = new_leaf();
        let map = leaf.as_mapping().unwrap();
        let path = Path::default().key("when");
        for choice in operator_choices(&path, &leaf, map, Some("eq"), false) {
            let Action::Set(at, Some(value)) = choice.action else {
                continue;
            };
            let mut root = doc::map_of([("when", leaf.clone())]);
            node::set(&mut root, &at, Some(value));
            let when = doc::flow(&root["when"]);
            let text = format!("jobs:\n  a:\n    tasks:\n      t: {{cmd: [echo], when: {when}}}\n");
            crate::validate::check(std::path::Path::new("project"), &text)
                .unwrap_or_else(|error| panic!("{}: {error:?}\n{text}", choice.label));
        }
    }
}
