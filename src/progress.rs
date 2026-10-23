//! Formats `run`'s log lines: one JSON object per line on stdout, also appended to
//! `logs/<date>.log` in the current directory.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use jiff::Zoned;
use serde_json::{Map, Value, json};

use crate::exec::TaskResult;

/// Prints `event` as one JSON line, prefixed with the job name and the current time,
/// and appends it to `logs/<date>.log`. A failed log file write is reported on stderr; a
/// closed stdout (`run | head`) is ignored, so the log file still gets every line.
pub async fn emit(job: &str, event: Value) {
    let now = Zoned::now();
    let mut line = Map::new();
    line.insert("job".to_string(), json!(job));
    line.insert(
        "time".to_string(),
        json!(now.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()),
    );
    if let Value::Object(fields) = event {
        line.extend(fields);
    }
    let line = Value::Object(line).to_string();
    let _ = writeln!(std::io::stdout(), "{line}");
    let date = now.strftime("%Y-%m-%d").to_string();
    let written = tokio::task::spawn_blocking(move || append(Path::new("logs"), &date, &line))
        .await
        .unwrap_or_else(|err| Err(std::io::Error::other(err)));
    if let Err(err) = written {
        eprintln!("cannot write log file: {err}");
    }
}

/// Appends `line` to `<dir>/<date>.log` in one write, so lines from concurrent jobs
/// stay whole.
fn append(dir: &Path, date: &str, line: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{date}.log")))?;
    file.write_all(format!("{line}\n").as_bytes())
}

pub fn task(name: &str, result: &TaskResult, elapsed: Duration, allowed: bool) -> Value {
    let mut event = json!({"event": "task", "task": name});
    match result {
        TaskResult::Cmd(result) => event["exit_code"] = json!(result.exit_code),
        TaskResult::Errored { error } => event["error"] = json!(error),
    }
    event["ms"] = json!(elapsed.as_millis());
    let ok = result.failure().is_none();
    event["ok"] = json!(ok);
    if !ok && allowed {
        event["allowed"] = json!(true);
    }
    event
}

pub fn branch(name: &str, taken: bool, count: usize) -> Value {
    let arm = if taken { "then" } else { "else" };
    json!({"event": "branch", "task": name, "arm": arm, "tasks": count})
}

/// `case` is the 1-based index of the matching case, or `None` for `default`.
pub fn switch(name: &str, case: Option<usize>, count: usize) -> Value {
    let case = case.map_or(json!("default"), |index| json!(index));
    json!({"event": "switch", "task": name, "case": case, "tasks": count})
}

pub fn for_each(name: &str, var: &str, count: usize) -> Value {
    json!({"event": "for_each", "task": name, "var": var, "items": count})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_cmd_and_errored_tasks() {
        let ms = Duration::from_millis(412);
        assert_eq!(
            task("check", &TaskResult::cmd(0), ms, false),
            json!({"event": "task", "task": "check", "exit_code": 0, "ms": 412, "ok": true})
        );
        assert_eq!(
            task("check", &TaskResult::cmd(3), ms, false),
            json!({"event": "task", "task": "check", "exit_code": 3, "ms": 412, "ok": false})
        );
        assert_eq!(
            task("check", &TaskResult::cmd(3), ms, true),
            json!({"event": "task", "task": "check", "exit_code": 3, "ms": 412, "ok": false, "allowed": true})
        );
        let errored = TaskResult::Errored {
            error: "command `sleep 9` timed out after 5s".to_string(),
        };
        assert_eq!(
            task("ping", &errored, Duration::from_millis(5003), true),
            json!({"event": "task", "task": "ping", "error": "command `sleep 9` timed out after 5s", "ms": 5003, "ok": false, "allowed": true})
        );
    }

    #[test]
    fn appends_lines_to_dated_file() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        append(&logs, "2026-10-11", "a").unwrap();
        append(&logs, "2026-10-11", "b").unwrap();
        let text = fs::read_to_string(logs.join("2026-10-11.log")).unwrap();
        assert_eq!(text, "a\nb\n");
    }

    #[test]
    fn formats_control_flow_events() {
        assert_eq!(
            branch("route", true, 1),
            json!({"event": "branch", "task": "route", "arm": "then", "tasks": 1})
        );
        assert_eq!(
            branch("route", false, 0),
            json!({"event": "branch", "task": "route", "arm": "else", "tasks": 0})
        );
        assert_eq!(
            switch("route", Some(2), 1),
            json!({"event": "switch", "task": "route", "case": 2, "tasks": 1})
        );
        assert_eq!(
            switch("route", None, 0),
            json!({"event": "switch", "task": "route", "case": "default", "tasks": 0})
        );
        assert_eq!(
            for_each("each", "IDS", 3),
            json!({"event": "for_each", "task": "each", "var": "IDS", "items": 3})
        );
    }
}
