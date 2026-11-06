#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use nix::sys::signal::Signal;
use rocky_core::trigger::Trigger;
use rocky_core::{error, exec, new, process_group, progress, project, run, socket};
use tokio::signal::unix::{SignalKind, signal};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one job of the project in the current directory.
    ///
    /// Runs the `cmd` tasks of `jobs/<JOB>.yaml` in order, where later tasks read earlier results as `${tasks.NAME...}`. Prints the last result.
    Exec {
        /// The job to run: the name of its file in `jobs/`, without `.yaml`.
        job: String,
        /// Print each command to stderr before it runs, with `${VAR}`s filled in.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Run the scheduled jobs of the project in the current directory.
    ///
    /// Runs until stopped, or until only finished `at` schedules are left. Prints one JSON line per task, skip, loop and run, also appended to `.rocky/logs/<date>.log` (`<service>-<date>.log` with `--service`).
    Run {
        /// Also run any job of the project when a client of this Unix socket sends `{"job": "NAME"}`.
        #[arg(long)]
        socket: Option<PathBuf>,
        /// Prefix log file names with this name: `.rocky/logs/<NAME>-<date>.log`.
        #[arg(long)]
        service: Option<String>,
    },
    /// Ask a `rocky run --socket` to run one job.
    ///
    /// Returns once the run is queued; the run logs to `run`'s output like a scheduled one.
    Trigger {
        /// The socket `run` listens on.
        socket: PathBuf,
        /// The job to run.
        job: String,
    },
    /// Check the project in the current directory for errors without running it.
    ///
    /// Loads `rocky.yaml`, every job file in `jobs/` and their task files, and reports whether they are valid. Sends no requests and runs no commands.
    Validate,
    /// Create a starter project, job or task file.
    ///
    /// Never overwrites a file.
    New {
        #[command(subcommand)]
        kind: NewKind,
        /// How much the file shows: `basic` is the smallest file that runs, `full` uses every field, with comments.
        #[arg(long, global = true, value_enum, default_value = "basic")]
        level: Level,
    },
}

/// How much a starter file shows.
#[derive(Clone, Copy, clap::ValueEnum)]
enum Level {
    /// The smallest file that runs.
    Basic,
    /// Every field, with comments.
    Full,
}

#[derive(Subcommand)]
enum NewKind {
    /// A project with one job, `example`, in a directory (default: the current one).
    Project {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// A job file, `jobs/<NAME>.yaml` in the current directory.
    Job { name: String },
    /// A task file, to list under a job's `tasks` (`name: path`).
    Task { path: PathBuf },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Exec { job, verbose } => {
            let root = Path::new(".");
            let (trigger, mut queue) = Trigger::channel();
            let project = project::load(root)?;
            let outcome = until_stopped(exec::exec(
                project,
                &job,
                &exec::Log::Off,
                verbose,
                &trigger,
            ))
            .await?;
            drop(trigger);
            println!("{}", serde_json::to_string_pretty(&outcome.result)?);
            let failed = until_stopped(async {
                let mut failed = failure(&outcome);
                // Jobs that `trigger` tasks queued, one at a time, and those they queue in turn.
                while let Some(queued) = queue.recv().await {
                    let project = project::load(root)?;
                    let outcome = exec::exec(
                        project,
                        &queued.job,
                        &exec::Log::Off,
                        verbose,
                        &queued.trigger,
                    )
                    .await?;
                    if failed.is_none() {
                        failed = failure(&outcome)
                            .map(|failure| format!("triggered job `{}`: {failure}", queued.job));
                    }
                }
                Ok(failed)
            })
            .await?;
            if let Some(failure) = failed {
                anyhow::bail!(failure);
            }
        }
        Command::Run { socket, service } => {
            if let Some(name) = service {
                progress::set_service(name);
            }
            let root = Path::new(".");
            until_stopped(run::run(root, socket.as_deref())).await?
        }
        Command::Trigger { socket, job } => {
            socket::send(&socket, &job).await?;
            println!("queued `{job}`");
        }
        Command::Validate => {
            let project = project::load(Path::new("."))?;
            println!("project is valid (jobs: {})", project.job_names());
        }
        Command::New { kind, level } => {
            let (kind, path) = match kind {
                NewKind::Project { dir } => (new::Kind::Project, dir),
                NewKind::Job { name } => (
                    new::Kind::Job,
                    PathBuf::from(project::JOBS_DIR).join(format!("{name}.yaml")),
                ),
                NewKind::Task { path } => (new::Kind::Task, path),
            };
            let level = match level {
                Level::Basic => new::Level::Basic,
                Level::Full => new::Level::Full,
            };
            println!("{}", new::create(kind, level, &path)?);
        }
    }
    Ok(())
}

/// Why `outcome`'s job failed, as `task `name` reason`, if it did.
fn failure(outcome: &exec::Outcome) -> Option<String> {
    let task = outcome.failed_task.as_ref()?;
    let reason = outcome.reason().unwrap_or_default();
    Some(format!("task `{task}` {reason}"))
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
