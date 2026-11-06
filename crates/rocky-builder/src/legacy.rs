//! Converts tasks in the removed `if` / `then` / `else` and `switch` / `cases` / `default`
//! syntax into plain tasks with `when`, in the same order.

use serde_yaml::{Mapping, Value};

use crate::doc;

/// Whether `task` uses the removed branch or switch syntax.
pub fn is_legacy(task: &Value) -> bool {
    task.get("if").is_some() || task.get("switch").is_some()
}

/// The plain tasks `task` stands for, each with the condition it ran under as its `when`.
/// A `then` task gets the branch's condition, an `else` task its negation; a switch case
/// gets `eq` on its key, and `default` gets "none of the keys". Nested branches combine.
pub fn flatten(name: &str, task: &Value) -> Vec<(String, Value)> {
    let mut tasks = Vec::new();
    push(name, task, None, &mut tasks);
    tasks
}

fn push(name: &str, task: &Value, outer: Option<Value>, out: &mut Vec<(String, Value)>) {
    if let Some(condition) = task.get("if") {
        for (branch, guard) in [
            ("then", condition.clone()),
            ("else", not(condition.clone())),
        ] {
            for (name, task) in entries(task.get(branch)) {
                push(&name, task, Some(and(outer.clone(), guard.clone())), out);
            }
        }
    } else if let Some(var) = task.get("switch") {
        let mut tests = Vec::new();
        if let Some(cases) = task.get("cases").and_then(Value::as_mapping) {
            for (key, tasks) in cases {
                let test = doc::map_of([("this", var.clone()), ("eq", key.clone())]);
                tests.push(test.clone());
                for (name, task) in entries(Some(tasks)) {
                    push(&name, task, Some(and(outer.clone(), test.clone())), out);
                }
            }
        }
        let guard =
            (!tests.is_empty()).then(|| not(doc::map_of([("any", Value::Sequence(tests))])));
        for (name, task) in entries(task.get("default")) {
            let condition = match (&outer, &guard) {
                (_, Some(guard)) => Some(and(outer.clone(), guard.clone())),
                (outer, None) => outer.clone(),
            };
            push(&name, task, condition, out);
        }
    } else {
        let mut task = task.clone();
        if let Some(outer) = outer {
            let when = match task.get("when") {
                Some(own) => and(Some(outer), own.clone()),
                None => outer,
            };
            doc::as_map(&mut task).insert(Value::from("when"), when);
        }
        out.push((name.to_string(), task));
    }
}

/// `a` and `b` as one condition: a list (which is `all`), flattening lists already there.
fn and(a: Option<Value>, b: Value) -> Value {
    let Some(a) = a else {
        return b;
    };
    let mut items = Vec::new();
    for condition in [a, b] {
        match condition {
            Value::Sequence(list) => items.extend(list),
            other => items.push(other),
        }
    }
    Value::Sequence(items)
}

fn not(condition: Value) -> Value {
    doc::map_of([("not", condition)])
}

/// The named tasks of a task map, in order.
fn entries(tasks: Option<&Value>) -> Vec<(String, &Value)> {
    tasks
        .and_then(Value::as_mapping)
        .map(|map| {
            map.iter()
                .map(|(key, task)| (doc::key_text(key), task))
                .collect()
        })
        .unwrap_or_default()
}

/// Why task `name` of `tasks` can't be converted: a task it holds has the name of another
/// task, or two of them share a name.
pub fn conflict(tasks: &Mapping, name: &str) -> Option<String> {
    let task = tasks.get(name)?;
    let mut seen: Vec<String> = doc::keys(tasks)
        .into_iter()
        .filter(|key| key != name)
        .collect();
    for (inner, _) in flatten(name, task) {
        if seen.contains(&inner) {
            return Some(format!("`{inner}` would be used twice; rename one first"));
        }
        seen.push(inner);
    }
    None
}

/// Replaces task `name` of `tasks` with its flattened tasks, where it was. Changes nothing
/// if there is a [`conflict`].
pub fn convert(tasks: &mut Mapping, name: &str) -> bool {
    if conflict(tasks, name).is_some() {
        return false;
    }
    let Some(task) = tasks.get(name).cloned() else {
        return false;
    };
    let flat = flatten(name, &task);
    let entries = std::mem::take(tasks);
    for (key, value) in entries {
        if key.as_str() == Some(name) {
            for (inner, task) in &flat {
                tasks.insert(Value::from(inner.as_str()), task.clone());
            }
        } else {
            tasks.insert(key, value);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tasks(yaml: &str) -> Mapping {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn converts_if_then_else_with_nesting() {
        let mut map = tasks(
            "\
A: {cmd: [echo]}
route:
  if: {this: tasks.A.stdout.count, gt: 10}
  then:
    B: {cmd: [echo, big]}
  else:
    warn:
      if: {this: tasks.A.stdout.message, contains: warning}
      then:
        C: {cmd: [echo, warn]}
D: {cmd: [echo]}
",
        );
        assert!(convert(&mut map, "route"));
        assert_eq!(doc::keys(&map), ["A", "B", "C", "D"]);
        assert_eq!(
            doc::flow(&map["B"]["when"]),
            "{this: tasks.A.stdout.count, gt: 10}"
        );
        assert_eq!(
            doc::flow(&map["C"]["when"]),
            "[{not: {this: tasks.A.stdout.count, gt: 10}}, {this: tasks.A.stdout.message, contains: warning}]"
        );
        let text = format!(
            "jobs:\n  j:\n    tasks:\n{}",
            indent(&serde_yaml::to_string(&map).unwrap())
        );
        crate::validate::check(std::path::Path::new("project"), &text).unwrap();
    }

    #[test]
    fn converts_switch_with_default() {
        let mut map = tasks(
            "\
route:
  switch: tasks.check.stdout
  cases:
    200: {healthy: {cmd: [echo, ok]}}
    503: {down: {cmd: [echo, down]}}
  default:
    other: {cmd: [echo, '?']}
",
        );
        assert!(convert(&mut map, "route"));
        assert_eq!(doc::keys(&map), ["healthy", "down", "other"]);
        assert_eq!(
            doc::flow(&map["down"]["when"]),
            "{this: tasks.check.stdout, eq: 503}"
        );
        assert_eq!(
            doc::flow(&map["other"]["when"]),
            "{not: {any: [{this: tasks.check.stdout, eq: 200}, {this: tasks.check.stdout, eq: 503}]}}"
        );
    }

    #[test]
    fn refuses_name_clashes() {
        let mut map =
            tasks("B: {cmd: [echo]}\nroute: {if: {this: X, eq: 1}, then: {B: {cmd: [echo]}}}\n");
        assert!(conflict(&map, "route").is_some());
        assert!(!convert(&mut map, "route"));
        assert_eq!(doc::keys(&map), ["B", "route"]);
    }

    fn indent(text: &str) -> String {
        text.lines().map(|line| format!("      {line}\n")).collect()
    }
}
