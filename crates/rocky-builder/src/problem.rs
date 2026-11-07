//! Core's load error, with the job and task it is about.

use rocky_core::error::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub job: Option<String>,
    pub task: Option<String>,
    /// The whole error chain.
    pub message: String,
}

impl Problem {
    /// Whether this problem is in job `name`, or in no job and so in every one.
    pub fn is_in(&self, name: &str) -> bool {
        self.job.as_deref().is_none_or(|job| job == name)
    }
}

/// Finds the job and task an error is about: from `job` / `task` wrappers, from the leading
/// `` `name` `` of an invalid-task message, or from the `tasks.T` path of a YAML error.
pub fn locate(error: &Error) -> Problem {
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
                problem.task = yaml_task(&source.to_string());
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

/// The innermost task named by a YAML error's leading `tasks.T.do.U: …` path.
fn yaml_task(message: &str) -> Option<String> {
    let path = message.split(": ").next().unwrap_or_default();
    let parts: Vec<&str> = path.split('.').collect();
    parts
        .windows(2)
        .rev()
        .find(|pair| pair[0] == "tasks" || pair[0] == "do")
        .map(|pair| pair[1].to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn problem(job: &str) -> Problem {
        let jobs = [("a".to_string(), job.to_string())];
        let error = rocky_core::project::load_parts(Path::new("."), None, &jobs).unwrap_err();
        locate(&error)
    }

    #[test]
    fn locates_problems_on_jobs_and_tasks() {
        let empty = problem("tasks:\n  t: {cmd: []}\n");
        assert_eq!(
            (empty.job.as_deref(), empty.task.as_deref()),
            (Some("a"), Some("t"))
        );
        let unknown = problem("tasks:\n  t: {cmd: [x], nope: 1}\n");
        assert_eq!(unknown.task.as_deref(), Some("t"));
        assert!(unknown.is_in("a"));
        assert!(!unknown.is_in("b"));
    }
}
