//! `${…}` autocomplete: which variables a field can read, which of them match what was
//! typed, and how picking one rewrites the text.

/// A variable that a `${…}` can read at this point of the job.
#[derive(Clone, Debug, PartialEq)]
pub struct Var {
    /// What gets inserted: `tasks.login.stdout.json.token`, `BASE`, `ITEM`.
    pub path: String,
    /// Shown on the right: the value from the last run, or where it comes from.
    pub detail: String,
    /// The heading it is listed under: `EARLIER TASK · login`, `VALUES · ENV FILE`.
    pub group: String,
}

/// What a field holds: free text with `${…}` references in it, or a whole variable path
/// (`this`, `for_each`), where `${}` is optional.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Inline,
    Whole,
}

/// Most suggestions shown at once.
const MAX: usize = 8;

/// The part of `text` being completed with the cursor at char `cursor`: its start char
/// index (the typed prefix runs from there to the cursor), or `None` when the cursor isn't
/// in an unclosed `${`, or (in [`Mode::Whole`]) when the field holds more than a path.
pub fn query(text: &str, cursor: usize, mode: Mode) -> Option<usize> {
    let before: Vec<char> = text.chars().take(cursor).collect();
    let open = (0..before.len().saturating_sub(1))
        .rev()
        .find(|&i| before[i] == '$' && before[i + 1] == '{');
    match (open, mode) {
        (Some(open), _) if !before[open + 2..].contains(&'}') => Some(open + 2),
        (None, Mode::Whole) if !before.iter().any(|c| c.is_whitespace()) => Some(0),
        _ => None,
    }
}

/// The variables matching `prefix`: those starting with it first, then those containing it.
pub fn matches<'a>(vars: &'a [Var], prefix: &str) -> Vec<&'a Var> {
    let starts = vars.iter().filter(|var| var.path.starts_with(prefix));
    let contains = vars
        .iter()
        .filter(|var| !var.path.starts_with(prefix) && var.path.contains(prefix));
    starts.chain(contains).take(MAX).collect()
}

/// `text` with the query starting at `start` and ending at `cursor` replaced by `path`,
/// closing an open `${` with `}`; returns the new text and the char index after the insert.
pub fn apply(text: &str, start: usize, cursor: usize, path: &str) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let in_braces = start >= 2 && chars[start - 2] == '$' && chars[start - 1] == '{';
    let closed = chars.get(cursor) == Some(&'}');
    let mut inserted = path.to_string();
    if in_braces && !closed {
        inserted.push('}');
    }
    let before: String = chars[..start].iter().collect();
    let after: String = chars[cursor.min(chars.len())..].iter().collect();
    let mut position = start + inserted.chars().count();
    if in_braces && closed {
        position += 1;
    }
    (format!("{before}{inserted}{after}"), position)
}

/// The vars of a JSON value at `path`: the value itself and, for objects, each field
/// below it, as deep as `depth`.
pub fn json_paths(
    path: &str,
    value: &serde_json::Value,
    group: &str,
    depth: usize,
    out: &mut Vec<Var>,
) {
    let detail = match value {
        serde_json::Value::String(text) => format!("{:?}", text.lines().next().unwrap_or_default()),
        serde_json::Value::Object(_) => "object".to_string(),
        serde_json::Value::Array(items) => format!("list of {}", items.len()),
        other => other.to_string(),
    };
    out.push(Var {
        path: path.to_string(),
        detail,
        group: group.to_string(),
    });
    if depth == 0 {
        return;
    }
    if let serde_json::Value::Object(fields) = value {
        for (key, value) in fields.iter().take(MAX) {
            json_paths(&format!("{path}.{key}"), value, group, depth - 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(path: &str) -> Var {
        Var {
            path: path.to_string(),
            detail: String::new(),
            group: String::new(),
        }
    }

    #[test]
    fn finds_the_open_reference() {
        let text = "Bearer ${tasks.lo";
        assert_eq!(query(text, text.chars().count(), Mode::Inline), Some(9));
        assert_eq!(query("${A} done", 9, Mode::Inline), None);
        assert_eq!(query("plain", 5, Mode::Inline), None);
        assert_eq!(query("tasks.a", 7, Mode::Whole), Some(0));
        assert_eq!(query("two words", 9, Mode::Whole), None);
    }

    #[test]
    fn ranks_prefix_matches_first() {
        let vars = [var("tasks.blogin.ok"), var("tasks.login.ok"), var("BASE")];
        let found: Vec<&str> = matches(&vars, "tasks.lo")
            .iter()
            .map(|v| v.path.as_str())
            .collect();
        assert_eq!(found, ["tasks.login.ok"]);
        let found: Vec<&str> = matches(&vars, "login")
            .iter()
            .map(|v| v.path.as_str())
            .collect();
        assert_eq!(found, ["tasks.blogin.ok", "tasks.login.ok"]);
    }

    #[test]
    fn inserts_and_closes() {
        assert_eq!(
            apply("Bearer ${tasks.lo", 9, 17, "tasks.login.stdout"),
            ("Bearer ${tasks.login.stdout}".to_string(), 28)
        );
        assert_eq!(apply("${BA}/x", 2, 4, "BASE"), ("${BASE}/x".to_string(), 7));
        assert_eq!(
            apply("tasks.c", 0, 7, "tasks.check.ok"),
            ("tasks.check.ok".to_string(), 14)
        );
    }

    #[test]
    fn lists_json_fields() {
        let mut out = Vec::new();
        let value = serde_json::json!({"json": {"token": "abc"}, "n": 1});
        json_paths("tasks.login.stdout", &value, "g", 2, &mut out);
        let paths: Vec<&str> = out.iter().map(|v| v.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "tasks.login.stdout",
                "tasks.login.stdout.json",
                "tasks.login.stdout.json.token",
                "tasks.login.stdout.n"
            ]
        );
        assert_eq!(out[2].detail, "\"abc\"");
    }
}
