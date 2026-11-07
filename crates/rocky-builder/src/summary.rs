//! Tasks and other YAML values in one line, for the job detail and the debug run: a task's
//! kind and summary, a `when` condition, and short inline YAML.

use ratatui::style::Color;
use serde_yaml::Value;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Cmd,
    ForEach,
    While,
    Trigger,
    /// None of the kind keys: the task is invalid.
    Unknown,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Cmd => "cmd",
            Kind::ForEach => "for_each",
            Kind::While => "while",
            Kind::Trigger => "trigger",
            Kind::Unknown => "?",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Kind::Cmd => theme::MUTED,
            Kind::ForEach => theme::PURPLE,
            Kind::While => theme::BLUE,
            Kind::Trigger => theme::ACCENT,
            Kind::Unknown => theme::ERROR,
        }
    }

    /// The fields a task of this kind can have, as `docs/tasks.md` lists them.
    pub fn fields(self) -> &'static str {
        match self {
            Kind::Cmd => {
                "cmd, pipe, when, timeout, cwd, stdin, env, capture_stdout, stdout_file, retry, allow_failure"
            }
            Kind::ForEach => "for_each, as, do, on_failure, when",
            Kind::While => "while, attempts, do, on_failure, when",
            Kind::Trigger => "trigger, when",
            Kind::Unknown => "cmd, for_each, while or trigger",
        }
    }
}

/// The fields of a job file's settings.
pub const SETTINGS_FIELDS: &str = "schedule, values, defaults";

pub fn kind(task: &Value) -> Kind {
    let has = |key: &str| task.get(key).is_some();
    if has("cmd") {
        Kind::Cmd
    } else if has("for_each") {
        Kind::ForEach
    } else if has("while") {
        Kind::While
    } else if has("trigger") {
        Kind::Trigger
    } else {
        Kind::Unknown
    }
}

/// A scalar as text; other values as inline YAML.
pub fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "~".to_string(),
        other => flow(other),
    }
}

/// `value` as inline YAML: `{days: [mon, fri], time: "09:30"}`.
pub fn flow(value: &Value) -> String {
    match value {
        Value::Mapping(_) => format!("{{{}}}", entries(value)),
        Value::Sequence(items) => {
            let items: Vec<String> = items.iter().map(flow).collect();
            format!("[{}]", items.join(", "))
        }
        Value::String(text) if needs_quotes(text) => format!("{text:?}"),
        Value::Tagged(tagged) => flow(&tagged.value),
        other => scalar(other),
    }
}

/// A mapping's entries as inline YAML without the braces: `every: 5m`.
pub fn entries(value: &Value) -> String {
    let Some(map) = value.as_mapping() else {
        return flow(value);
    };
    map.iter()
        .map(|(key, value)| format!("{}: {}", scalar(key), flow(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn needs_quotes(text: &str) -> bool {
    text.is_empty()
        || text.contains([':', '#', ',', '{', '}', '[', ']', '"', '\''])
        || text.starts_with(['*', '&', '!', '|', '>', '%', '@', '`', ' ', '-'])
}

/// A condition in words: `tasks.check.stdout ne 200`, `not x eq 1`, `a and b`.
pub fn condition(value: &Value) -> String {
    let join = |items: &Value, word: &str| {
        let parts: Vec<String> = items
            .as_sequence()
            .into_iter()
            .flatten()
            .map(condition)
            .collect();
        parts.join(&format!(" {word} "))
    };
    if value.is_sequence() {
        return join(value, "and");
    }
    let Some(map) = value.as_mapping() else {
        return scalar(value);
    };
    if let Some(inner) = map.get("not") {
        return format!("not {}", condition(inner));
    }
    if let Some(all) = map.get("all") {
        return join(all, "and");
    }
    if let Some(any) = map.get("any") {
        return join(any, "or");
    }
    let mut words: Vec<String> = map.get("this").map(scalar).into_iter().collect();
    for (key, value) in map {
        if key.as_str() != Some("this") {
            words.push(format!("{} {}", scalar(key), scalar(value)));
        }
    }
    words.join(" ")
}

/// A command's arguments as a shell would show them.
fn command(args: &Value) -> String {
    let args: Vec<String> = args
        .as_sequence()
        .into_iter()
        .flatten()
        .map(|arg| {
            let arg = scalar(arg);
            if arg.is_empty() || arg.contains([' ', '"']) {
                format!("{arg:?}")
            } else {
                arg
            }
        })
        .collect();
    args.join(" ")
}

/// A cmd task's command line, its `pipe` stages included.
pub fn command_line(task: &Value) -> String {
    let mut line = task.get("cmd").map(command).unwrap_or_default();
    for stage in task
        .get("pipe")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
    {
        line.push_str(" | ");
        line.push_str(&command(stage));
    }
    line
}

/// What a task does, in one line.
pub fn line(task: &Value) -> String {
    let field = |key: &str| task.get(key);
    let mut line = match kind(task) {
        Kind::Cmd => {
            let mut line = command_line(task);
            if let Some(stdin) = field("stdin").and_then(Value::as_str) {
                line.push_str(&format!(" ← stdin {stdin}"));
            }
            line
        }
        Kind::Trigger => format!("→ {}", field("trigger").map(scalar).unwrap_or_default()),
        Kind::ForEach => {
            let list = field("for_each").map(scalar).unwrap_or_default();
            match field("as") {
                Some(name) => format!("{list} as {}", scalar(name)),
                None => list,
            }
        }
        Kind::While => {
            let check = field("while").map(condition).unwrap_or_default();
            match field("attempts") {
                Some(n) => format!("{check} · attempts: {}", scalar(n)),
                None => check,
            }
        }
        Kind::Unknown => entries(task),
    };
    if field("allow_failure").and_then(Value::as_bool) == Some(true) {
        line.push_str(" · allow_failure");
    }
    if let Some(on_failure) = field("on_failure") {
        line.push_str(&format!(" · on_failure: {}", scalar(on_failure)));
    }
    line
}

/// A task map's tasks in order, with their loops' `do` tasks right after them: each with
/// its depth, its name and its YAML.
pub fn tasks(map: Option<&Value>) -> Vec<(usize, String, &Value)> {
    let mut out = Vec::new();
    add_tasks(map, 0, &mut out);
    out
}

fn add_tasks<'a>(map: Option<&'a Value>, depth: usize, out: &mut Vec<(usize, String, &'a Value)>) {
    for (name, task) in map.and_then(Value::as_mapping).into_iter().flatten() {
        out.push((depth, scalar(name), task));
        add_tasks(task.get("do"), depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml::from_str(text).unwrap()
    }

    #[test]
    fn summarizes_each_kind() {
        let cmd = yaml("cmd: [echo, 'not found, using defaults']\nwhen: {this: x, ne: 0}");
        assert_eq!(kind(&cmd), Kind::Cmd);
        assert_eq!(line(&cmd), "echo \"not found, using defaults\"");
        let pipe = yaml("cmd: [jq, -r, .token]\nstdin: x\nallow_failure: true");
        assert_eq!(line(&pipe), "jq -r .token ← stdin x · allow_failure");
        let each = yaml("for_each: tasks.u.stdout\nas: USER\non_failure: next\ndo: {}");
        assert_eq!(line(&each), "tasks.u.stdout as USER · on_failure: next");
        let wait = yaml("while: {this: tasks.c.stdout, ne: 200}\nattempts: 10\ndo: {}");
        assert_eq!(line(&wait), "tasks.c.stdout ne 200 · attempts: 10");
        assert_eq!(line(&yaml("trigger: health")), "→ health");
        assert_eq!(kind(&yaml("when: {}")), Kind::Unknown);
    }

    #[test]
    fn writes_conditions_in_words() {
        let all = yaml("[{this: a, lte: 10}, {not: {this: m, matches: '^x'}}]");
        assert_eq!(condition(&all), "a lte 10 and not m matches ^x");
        assert_eq!(
            condition(&yaml("any: [{this: a, eq: 1}, {this: b, eq: 2}]")),
            "a eq 1 or b eq 2"
        );
    }

    #[test]
    fn writes_inline_yaml() {
        let schedule = yaml("weekly: {days: [mon, fri], time: '09:30'}");
        assert_eq!(
            entries(&schedule),
            "weekly: {days: [mon, fri], time: \"09:30\"}"
        );
        assert_eq!(entries(&yaml("timeout: 30s")), "timeout: 30s");
    }

    #[test]
    fn lists_nested_tasks_in_order() {
        let job = yaml("a: {cmd: [x]}\nb: {for_each: l, do: {c: {cmd: [y]}}}\nd: {cmd: [z]}");
        let names: Vec<_> = tasks(Some(&job))
            .into_iter()
            .map(|(d, n, _)| (d, n))
            .collect();
        let expected = [(0, "a"), (0, "b"), (1, "c"), (0, "d")].map(|(d, n)| (d, n.to_string()));
        assert_eq!(names, expected);
    }
}
