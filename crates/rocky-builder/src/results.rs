//! What a run reported, per job and task, collected from core's progress events (the same
//! JSON objects `rocky run` logs, with `job` and, for tasks, the full `result`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Everything one task reported during a run. A task in a loop runs once per iteration.
#[derive(Default)]
pub struct TaskRun {
    /// The last `task` event: `exit_code` or `error`, `ok`, `ms`, `allowed`, `result`.
    pub last: Option<Value>,
    pub runs: u32,
    pub failures: u32,
    /// Skipped because its `when` didn't hold, and never ran.
    pub skipped: bool,
    /// For a `trigger` task, the job it queued.
    pub queued: Option<String>,
    /// For a `for_each` task, how many items it looped over.
    pub items: Option<u64>,
    /// For a `while` task, the last check: its number and whether it held.
    pub check: Option<(u64, bool)>,
}

/// How a whole run ended.
pub enum End {
    /// Every job ran; `failed` says why one stopped early, if any did.
    Done { failed: Option<String> },
    /// The run couldn't go on: a load or task error, or Stop.
    Error(String),
}

#[derive(Default)]
pub struct Results {
    tasks: HashMap<(String, String), TaskRun>,
    /// Jobs that reported anything, in the order they started.
    pub jobs: Vec<String>,
    pub end: Option<End>,
}

impl Results {
    /// Adds one event.
    pub fn apply(&mut self, event: &Value) {
        let job = event["job"].as_str().unwrap_or_default().to_string();
        if !job.is_empty() && !self.jobs.contains(&job) {
            self.jobs.push(job.clone());
        }
        let kind = event["event"].as_str().unwrap_or_default();
        let Some(task) = event["task"].as_str() else {
            match kind {
                "done" => {
                    self.end = Some(End::Done {
                        failed: event["failed"].as_str().map(str::to_string),
                    })
                }
                "error" => {
                    self.end = Some(End::Error(
                        event["error"].as_str().unwrap_or("failed").to_string(),
                    ))
                }
                _ => {}
            }
            return;
        };
        let run = self.tasks.entry((job, task.to_string())).or_default();
        match kind {
            "task" => {
                run.runs += 1;
                if event["ok"] != Value::Bool(true) {
                    run.failures += 1;
                }
                run.skipped = false;
                run.last = Some(event.clone());
            }
            "skip" => run.skipped = run.runs == 0,
            "trigger" => run.queued = event["target"].as_str().map(str::to_string),
            "for_each" => run.items = event["items"].as_u64(),
            "while" => {
                run.check = event["iteration"]
                    .as_u64()
                    .map(|n| (n, event["holds"] == Value::Bool(true)))
            }
            _ => {}
        }
    }

    pub fn task(&self, job: &str, task: &str) -> Option<&TaskRun> {
        self.tasks.get(&(job.to_string(), task.to_string()))
    }

    /// How many task runs `job` had, and how many of them failed.
    pub fn job_counts(&self, job: &str) -> (u32, u32) {
        self.tasks
            .iter()
            .filter(|((name, _), _)| name == job)
            .fold((0, 0), |(runs, failures), (_, run)| {
                (runs + run.runs, failures + run.failures)
            })
    }
}

/// One run as the editor shows it: a run started in the builder, or the latest run of a
/// job that the scheduler reported.
#[derive(Clone, Copy)]
pub struct Shown<'a> {
    pub results: &'a Results,
    /// The job the run started with, and the task it stops after.
    pub job: &'a str,
    pub until: Option<&'a str>,
    pub started: Instant,
    pub elapsed: Duration,
    /// Who ran it, when it wasn't the builder: `scheduler`.
    pub by: Option<&'static str>,
}

impl Shown<'_> {
    pub fn is_running(&self) -> bool {
        self.results.end.is_none()
    }

    /// Whether this run reported anything about `job`.
    pub fn covers(&self, job: &str) -> bool {
        self.job == job || self.results.jobs.iter().any(|name| name == job)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn collects_runs_per_task() {
        let mut results = Results::default();
        for event in [
            json!({"event": "for_each", "job": "j", "task": "each", "items": 2}),
            json!({"event": "task", "job": "j", "task": "greet", "ok": true, "exit_code": 0}),
            json!({"event": "task", "job": "j", "task": "greet", "ok": false, "exit_code": 1}),
            json!({"event": "skip", "job": "j", "task": "later"}),
            json!({"event": "trigger", "job": "j", "task": "page", "target": "other"}),
            json!({"event": "task", "job": "other", "task": "x", "ok": true, "exit_code": 0}),
            json!({"event": "done", "failed": null}),
        ] {
            results.apply(&event);
        }
        let greet = results.task("j", "greet").unwrap();
        assert_eq!((greet.runs, greet.failures), (2, 1));
        assert_eq!(greet.last.as_ref().unwrap()["exit_code"], 1);
        assert_eq!(results.task("j", "each").unwrap().items, Some(2));
        assert!(results.task("j", "later").unwrap().skipped);
        assert_eq!(
            results.task("j", "page").unwrap().queued.as_deref(),
            Some("other")
        );
        assert_eq!(results.jobs, ["j", "other"]);
        assert_eq!(results.job_counts("j"), (2, 1));
        assert!(matches!(results.end, Some(End::Done { failed: None })));
    }
}
