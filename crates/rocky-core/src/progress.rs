//! Formats `run`'s log lines: one JSON object per line on stdout, also appended to
//! `.rocky/logs/<date>.log` (or `<service>-<date>.log`, and `-<project>` before `.log` with a
//! project set) in the current directory.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use jiff::Zoned;
use serde_json::{Map, Value, json};

use crate::exec::TaskResult;

/// The `run --service` name that prefixes log file names, set once at startup.
static SERVICE: OnceLock<String> = OnceLock::new();

/// Prefixes every later log file name with `name`: `<name>-<date>.log`.
pub fn set_service(name: String) {
    let _ = SERVICE.set(name);
}

/// The `run -f` project file's stem that suffixes log file names, set once at startup.
static PROJECT: OnceLock<String> = OnceLock::new();

/// Suffixes every later log file name with `name`: `<date>-<name>.log`.
pub fn set_project(name: String) {
    let _ = PROJECT.set(name);
}

/// Prints `event` as one JSON line, prefixed with the job name and the current time,
/// and appends it to `.rocky/logs/<date>.log` (`<service>-<date>.log` with a service set,
/// `<date>-<project>.log` with a project set). A failed log file write is reported on stderr; a
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
    let name = file_stem(
        SERVICE.get().map(String::as_str),
        &now.strftime("%Y-%m-%d").to_string(),
        PROJECT.get().map(String::as_str),
    );
    let written =
        tokio::task::spawn_blocking(move || append(Path::new(".rocky/logs"), &name, &line))
            .await
            .unwrap_or_else(|err| Err(std::io::Error::other(err)));
    if let Err(err) = written {
        eprintln!("cannot write log file: {err}");
    }
}

/// The log file name without `.log`: `<date>`, prefixed with `<service>-` and suffixed with
/// `-<project>` when given.
fn file_stem(service: Option<&str>, date: &str, project: Option<&str>) -> String {
    let mut stem = date.to_string();
    if let Some(service) = service {
        stem = format!("{service}-{stem}");
    }
    if let Some(project) = project {
        stem = format!("{stem}-{project}");
    }
    stem
}

/// Appends `line` to `<dir>/<name>.log` in one write, so lines from concurrent jobs
/// stay whole.
fn append(dir: &Path, name: &str, line: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{name}.log")))?;
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

/// A task skipped because its `when` doesn't hold.
pub fn skip(name: &str) -> Value {
    json!({"event": "skip", "task": name})
}

pub fn for_each(name: &str, var: &str, count: usize) -> Value {
    json!({"event": "for_each", "task": name, "var": var, "items": count})
}

/// One `while` condition check; `iteration` counts checks from 1.
pub fn repeat(name: &str, iteration: u64, holds: bool) -> Value {
    json!({"event": "while", "task": name, "iteration": iteration, "holds": holds})
}

/// A `trigger` task queued a run of `job`, its `target` (`job` is the line's own job).
pub fn trigger(name: &str, job: &str) -> Value {
    json!({"event": "trigger", "task": name, "target": job})
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
    fn names_file_by_service_date_and_project() {
        assert_eq!(file_stem(None, "2026-10-11", None), "2026-10-11");
        assert_eq!(file_stem(Some("web"), "2026-10-11", None), "web-2026-10-11");
        assert_eq!(file_stem(None, "2026-10-11", Some("p")), "2026-10-11-p");
        assert_eq!(
            file_stem(Some("web"), "2026-10-11", Some("p")),
            "web-2026-10-11-p"
        );
    }

    #[test]
    fn formats_control_flow_events() {
        assert_eq!(skip("big"), json!({"event": "skip", "task": "big"}));
        assert_eq!(
            for_each("each", "IDS", 3),
            json!({"event": "for_each", "task": "each", "var": "IDS", "items": 3})
        );
        assert_eq!(
            repeat("wait", 2, true),
            json!({"event": "while", "task": "wait", "iteration": 2, "holds": true})
        );
        assert_eq!(
            trigger("notify", "report"),
            json!({"event": "trigger", "task": "notify", "target": "report"})
        );
    }
}
