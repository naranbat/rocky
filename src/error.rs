use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0} already exists")]
    Exists(PathBuf),
    #[error("failed to read the answer")]
    Prompt(#[source] std::io::Error),
    #[error("invalid YAML in {path}")]
    Yaml {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("invalid state file {path}")]
    State {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to load env file {path}")]
    EnvFile {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(
        "variable `{0}` is not defined (checked steps, states, loop items, vars, env_file, environment)"
    )]
    UndefinedVar(String),
    #[error("variable `{0}` refers to itself through its own value")]
    CyclicVar(String),
    #[error("unclosed `${{` in `{0}`")]
    UnclosedVar(String),
    #[error("invalid HTTP method `{0}`")]
    InvalidMethod(String),
    #[error("invalid header `{0}`")]
    InvalidHeader(String),
    #[error("invalid step: {0}")]
    InvalidStep(String),
    #[error("invalid condition: {0}")]
    InvalidCondition(String),
    #[error("condition: {0}")]
    Condition(String),
    #[error("for_each: {0}")]
    Loop(String),
    #[error("step `{name}` failed")]
    Step {
        name: String,
        #[source]
        source: Box<Error>,
    },
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
    #[error("invalid project: {0}")]
    InvalidProject(String),
    #[error("no job `{name}` (available: {available})")]
    UnknownJob { name: String, available: String },
    #[error("job `{name}`")]
    Job {
        name: String,
        #[source]
        source: Box<Error>,
    },
    #[error("schedule time")]
    Time(#[from] jiff::Error),
    #[error("job task stopped unexpectedly")]
    Join(#[from] tokio::task::JoinError),
    #[error("failed to start command `{command}`")]
    Spawn {
        command: String,
        source: std::io::Error,
    },
    #[error("`cwd` {} is not a directory", .0.display())]
    Cwd(PathBuf),
    #[error("command `{command}` timed out after {secs}s")]
    CommandTimeout { command: String, secs: u64 },
    #[error("request timed out after {0}s")]
    Timeout(u64),
    #[error("request failed")]
    Http(#[from] reqwest::Error),
}

impl Error {
    /// This error and its causes on one line, joined with `: `.
    pub fn chain(&self) -> String {
        let mut text = self.to_string();
        let mut source = std::error::Error::source(self);
        while let Some(cause) = source {
            text.push_str(": ");
            text.push_str(&cause.to_string());
            source = cause.source();
        }
        text
    }
}
