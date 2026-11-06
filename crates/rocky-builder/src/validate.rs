//! Checks the unsaved document with rocky-core, exactly as `rocky validate` would, shortly
//! after the last edit, and works out which job and task a problem belongs to.

use std::path::Path;
use std::time::{Duration, Instant};

use rocky_core::error::Error;

/// How long after the last edit to validate again.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Why the document is invalid, and where, as far as the error tells.
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub job: Option<String>,
    pub task: Option<String>,
    /// Core's whole error chain.
    pub message: String,
}

impl Problem {
    /// Whether the problem is on task `task` of `job`.
    pub fn is_on(&self, job: &str, task: &str) -> bool {
        self.job.as_deref() == Some(job) && self.task.as_deref() == Some(task)
    }
}

#[derive(Default)]
pub struct Validation {
    /// `None` until the first check; then `Ok` or the problem.
    pub result: Option<Result<(), Problem>>,
    /// The text last checked, so unchanged text isn't checked again.
    checked: String,
    /// When the text last changed without being checked yet.
    pending_since: Option<Instant>,
}

impl Validation {
    /// Validates `text` if it changed at least [`DEBOUNCE`] ago, or right away with `now`.
    /// Returns how long to wait before calling again, while a check is pending.
    pub fn update(&mut self, path: &Path, text: &str, now: bool) -> Option<Duration> {
        if text != self.checked && self.pending_since.is_none() {
            self.pending_since = Some(Instant::now());
        }
        let due = self
            .pending_since
            .map(|since| since.elapsed() >= DEBOUNCE || self.result.is_none())
            .unwrap_or(false);
        if now || due {
            self.result = Some(check(path, text));
            self.checked = text.to_string();
            self.pending_since = None;
            return None;
        }
        self.pending_since
            .map(|since| DEBOUNCE.saturating_sub(since.elapsed()))
    }

    pub fn is_valid(&self) -> bool {
        matches!(self.result, Some(Ok(())))
    }

    pub fn problem(&self) -> Option<&Problem> {
        match &self.result {
            Some(Err(problem)) => Some(problem),
            _ => None,
        }
    }
}

/// Core's verdict on `text` as the contents of the project file at `path`.
pub fn check(path: &Path, text: &str) -> Result<(), Problem> {
    rocky_core::project::load_str(path, text)
        .map(|_| ())
        .map_err(|error| locate(&error))
}

/// Finds the job and task an error is about: from `job` / `task` wrappers, from the leading
/// `` `name` `` of an invalid-task message, or from the `jobs.J.tasks.T` path of a YAML error.
fn locate(error: &Error) -> Problem {
    let mut problem = Problem {
        job: None,
        task: None,
        message: error.chain(),
    };
    let mut current = error;
    loop {
        match current {
            Error::Job { name, source } => {
                problem.job = Some(name.clone());
                current = source;
            }
            Error::Task { name, source } => {
                problem.task = Some(name.clone());
                current = source;
            }
            Error::InvalidTask(message) => {
                problem.task = problem.task.take().or_else(|| quoted_name(message));
                break;
            }
            Error::Yaml { source, .. } => {
                let (job, task) = yaml_path(&source.to_string());
                problem.job = job;
                problem.task = task;
                break;
            }
            _ => break,
        }
    }
    problem
}

/// The name in a message that starts with `` `name` ``.
fn quoted_name(message: &str) -> Option<String> {
    let rest = message.strip_prefix('`')?;
    rest.find('`').map(|end| rest[..end].to_string())
}

/// The job and innermost task named by a YAML error's leading `jobs.J.tasks.T.do.U: …` path.
fn yaml_path(message: &str) -> (Option<String>, Option<String>) {
    let path = message.split(": ").next().unwrap_or_default();
    let parts: Vec<&str> = path.split('.').collect();
    if parts.first() != Some(&"jobs") {
        return (None, None);
    }
    let job = parts.get(1).map(|job| job.to_string());
    let task = parts
        .windows(2)
        .rev()
        .find(|pair| pair[0] == "tasks" || pair[0] == "do")
        .map(|pair| pair[1].to_string());
    (job, task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(yaml: &str) -> Problem {
        check(Path::new("project.yaml"), yaml).unwrap_err()
    }

    #[test]
    fn locates_problems_on_jobs_and_tasks() {
        let empty = problem("jobs:\n  a:\n    tasks:\n      t: {cmd: []}\n");
        assert_eq!(
            (empty.job.as_deref(), empty.task.as_deref()),
            (Some("a"), Some("t"))
        );
        assert!(empty.is_on("a", "t"));

        let nested = problem(
            "jobs:\n  a:\n    tasks:\n      each:\n        for_each: X\n        do:\n          inner: {cmd: []}\n",
        );
        assert_eq!(nested.task.as_deref(), Some("inner"));

        let unknown = problem("jobs:\n  a:\n    tasks:\n      route: {if: {this: A, eq: 1}}\n");
        assert_eq!(
            (unknown.job.as_deref(), unknown.task.as_deref()),
            (Some("a"), Some("route"))
        );

        let trigger = problem("jobs:\n  a:\n    tasks:\n      go: {trigger: nope}\n");
        assert_eq!(
            (trigger.job.as_deref(), trigger.task.as_deref()),
            (Some("a"), Some("go"))
        );

        let job = problem("jobs:\n  a:\n    tasks: {}\n");
        assert_eq!((job.job.as_deref(), job.task), (Some("a"), None));

        let project = problem("jobs: {}\n");
        assert_eq!((project.job, project.task), (None, None));
    }

    #[test]
    fn first_check_is_immediate_then_debounced() {
        let path = Path::new("project.yaml");
        let mut validation = Validation::default();
        validation.update(path, "jobs: {}\n", false);
        assert!(validation.problem().is_some());
        assert!(validation.update(path, "jobs: {a: 1}\n", false).is_some());
        assert!(validation.update(path, "jobs: {a: 1}\n", true).is_none());
    }
}
