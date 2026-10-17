//! Formats `run`'s log lines: one JSON object per line on stdout, also appended to
//! `logs/<date>.log` in the current directory.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use jiff::Zoned;
use serde_json::{Map, Value, json};

use crate::exec::StepResult;

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

pub fn step(name: &str, result: &StepResult, elapsed: Duration, allowed: bool) -> Value {
    let mut event = json!({"event": "step", "step": name});
    match result {
        StepResult::Http(response) => event["status"] = json!(response.status),
        StepResult::Cmd(result) => event["exit_code"] = json!(result.exit_code),
        StepResult::Errored { error } => event["error"] = json!(error),
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
    json!({"event": "branch", "step": name, "arm": arm, "steps": count})
}

/// `case` is the 1-based index of the matching case, or `None` for `default`.
pub fn switch(name: &str, case: Option<usize>, count: usize) -> Value {
    let case = case.map_or(json!("default"), |index| json!(index));
    json!({"event": "switch", "step": name, "case": case, "steps": count})
}

pub fn for_each(name: &str, var: &str, count: usize) -> Value {
    json!({"event": "for_each", "step": name, "var": var, "items": count})
}

/// Names only: like `step` lines, it leaves values out.
pub fn set(name: &str, states: &Map<String, Value>, vars: &Map<String, Value>) -> Value {
    let states: Vec<&String> = states.keys().collect();
    let vars: Vec<&String> = vars.keys().collect();
    json!({"event": "set", "step": name, "states": states, "vars": vars})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_http_and_cmd_steps() {
        let http = StepResult::http;
        let ms = Duration::from_millis(412);
        assert_eq!(
            step("login", &http(200), ms, false),
            json!({"event": "step", "step": "login", "status": 200, "ms": 412, "ok": true})
        );
        assert_eq!(
            step("login", &http(404), ms, false),
            json!({"event": "step", "step": "login", "status": 404, "ms": 412, "ok": false})
        );
        assert_eq!(
            step("login", &http(404), ms, true),
            json!({"event": "step", "step": "login", "status": 404, "ms": 412, "ok": false, "allowed": true})
        );
        let cmd = StepResult::cmd(3);
        assert_eq!(
            step("check", &cmd, Duration::from_millis(5), false),
            json!({"event": "step", "step": "check", "exit_code": 3, "ms": 5, "ok": false})
        );
        let errored = StepResult::Errored {
            error: "request timed out after 5s".to_string(),
        };
        assert_eq!(
            step("ping", &errored, Duration::from_millis(5003), true),
            json!({"event": "step", "step": "ping", "error": "request timed out after 5s", "ms": 5003, "ok": false, "allowed": true})
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
            json!({"event": "branch", "step": "route", "arm": "then", "steps": 1})
        );
        assert_eq!(
            branch("route", false, 0),
            json!({"event": "branch", "step": "route", "arm": "else", "steps": 0})
        );
        assert_eq!(
            switch("route", Some(2), 1),
            json!({"event": "switch", "step": "route", "case": 2, "steps": 1})
        );
        assert_eq!(
            switch("route", None, 0),
            json!({"event": "switch", "step": "route", "case": "default", "steps": 0})
        );
        assert_eq!(
            for_each("each", "IDS", 3),
            json!({"event": "for_each", "step": "each", "var": "IDS", "items": 3})
        );
        let states = Map::from_iter([("A".to_string(), json!(1)), ("B".to_string(), json!("x"))]);
        assert_eq!(
            set("save", &states, &Map::new()),
            json!({"event": "set", "step": "save", "states": ["A", "B"], "vars": []})
        );
    }
}
