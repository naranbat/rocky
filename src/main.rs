#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use nix::sys::signal::Signal;
use rocky::{error, exec, new, process_group, project, run};
use tokio::signal::unix::{SignalKind, signal};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one job of a project.
    ///
    /// Runs `http` and `cmd` tasks in order, where later tasks read earlier results as `${tasks.NAME...}`. Prints the last result.
    Exec {
        file: PathBuf,
        /// The job to run.
        job: String,
    },
    /// Run a project's scheduled jobs.
    ///
    /// Runs until stopped. Prints one JSON line per task, branch, switch, loop, state update and run, also appended to `logs/<date>.log`.
    Run { file: PathBuf },
    /// Check a project for errors without running it.
    ///
    /// Loads the project and its job and task files, and reports whether they are valid. Sends no requests and runs no commands.
    Validate { file: PathBuf },
    /// Create a starter project, job or task file.
    ///
    /// Never overwrites a file.
    New {
        #[command(subcommand)]
        kind: NewKind,
        /// How much the file shows: `basic` is the smallest file that runs, `full` uses every field, with comments.
        #[arg(long, global = true, value_enum, default_value = "basic")]
        level: new::Level,
    },
}

#[derive(Subcommand)]
enum NewKind {
    /// A project with one job, `example`.
    Project { path: PathBuf },
    /// A job file, to list in a project's `jobs`.
    Job { path: PathBuf },
    /// A task file, to list under a job's `tasks` (`name: path`). Asks whether it's an HTTP or cmd task.
    Task { path: PathBuf },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Exec { file, job } => {
            let outcome = until_stopped(exec::exec(project::load(&file)?, &job, false)).await?;
            println!("{}", serde_json::to_string_pretty(&outcome.result)?);
            if let Some(task) = outcome.failed_task {
                let reason = outcome
                    .result
                    .as_ref()
                    .and_then(exec::TaskResult::failure)
                    .unwrap_or_default();
                anyhow::bail!("task `{task}` {reason}");
            }
        }
        Command::Run { file } => until_stopped(run::run(&file)).await?,
        Command::Validate { file } => {
            let project = project::load(&file)?;
            let names: Vec<_> = project.jobs.keys().map(|n| format!("`{n}`")).collect();
            println!("{} is valid (jobs: {})", file.display(), names.join(", "));
        }
        Command::New { kind, level } => {
            let (kind, path) = match kind {
                NewKind::Project { path } => (new::Kind::Project, path),
                NewKind::Job { path } => (new::Kind::Job, path),
                NewKind::Task { path } => {
                    if path.exists() {
                        return Err(error::Error::Exists(path).into());
                    }
                    let http = new::ask_http(std::io::stdin().lock(), std::io::stderr())?;
                    (new::Kind::Task { http }, path)
                }
            };
            println!("{}", new::create(kind, level, &path)?);
        }
    }
    Ok(())
}

/// Runs `work` until it ends, or until Ctrl-C, SIGTERM, SIGHUP (the terminal closed) or
/// SIGQUIT: then kills the running commands, which have their own process groups and so
/// don't get the signal, and exits with 128 + the signal number.
async fn until_stopped<T>(
    work: impl Future<Output = Result<T, error::Error>>,
) -> anyhow::Result<T> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let mut quit = signal(SignalKind::quit())?;
    let code = tokio::select! {
        result = work => return Ok(result?),
        interrupted = tokio::signal::ctrl_c() => {
            interrupted?;
            128 + Signal::SIGINT as i32
        }
        _ = terminate.recv() => 128 + Signal::SIGTERM as i32,
        _ = hangup.recv() => 128 + Signal::SIGHUP as i32,
        _ = quit.recv() => 128 + Signal::SIGQUIT as i32,
    };
    process_group::kill_running();
    std::process::exit(code)
}
