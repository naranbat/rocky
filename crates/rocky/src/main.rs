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
    /// Run one job of the project.
    ///
    /// Runs the `cmd` tasks of the job's file in order, where later tasks read earlier results as `${tasks.NAME...}`. Prints the last result.
    Exec {
        /// The job to run: its name in the project file's `jobs`.
        job: String,
        /// The project file (default: `rocky.yaml`).
        #[arg(short, long)]
        file: Option<PathBuf>,
        /// Print each command to stderr before it runs, with `${VAR}`s filled in.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Run the scheduled jobs of the project.
    ///
    /// Runs until stopped, or, without `--socket`, until no job has runs left: only finished `at` and `run_on_start`-only schedules, or stopped ones. Prints one JSON line per event (`task`, `skip`, `for_each`, `while`, `trigger`, `triggered`, `done`, `error`, `stopped`, `skipped`), also appended to `.rocky/logs/<date>.log` (`<date>-<PROJECT>.log` with `-f`, where PROJECT is the file name without its extension).
    Run {
        /// The project file (default: `rocky.yaml`).
        #[arg(short, long)]
        file: Option<PathBuf>,
        /// Also run any job of the project when a client of this Unix socket sends `{"job": "NAME"}`.
        #[arg(long)]
        socket: Option<PathBuf>,
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
    /// Check the project for errors without running it.
    ///
    /// Loads the project file and every job file it lists, and reports whether they are valid. Sends no requests and runs no commands.
    Validate {
        /// The project file (default: `rocky.yaml`).
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
    /// Create a starter project or job file.
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Exec { job, file, verbose } => {
            let file = project_file(file);
            let root = file.as_path();
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
        Command::Run { file, socket } => {
            if let Some(stem) = file.as_deref().and_then(Path::file_stem) {
                progress::set_project(stem.to_string_lossy().into_owned());
            }
            let file = project_file(file);
            until_stopped(run::run(&file, socket.as_deref())).await?
        }
        Command::Trigger { socket, job } => {
            socket::send(&socket, &job).await?;
            println!("queued `{job}`");
        }
        Command::Validate { file } => {
            let project = project::load(&project_file(file))?;
            println!("project is valid (jobs: {})", project.job_names());
        }
        Command::New { kind, level } => {
            let (kind, path) = match kind {
                NewKind::Project { dir } => (new::Kind::Project, dir),
                NewKind::Job { name } => {
                    project::check_job_name(&name)?;
                    (
                        new::Kind::Job,
                        PathBuf::from(project::JOBS_DIR).join(format!("{name}.yaml")),
                    )
                }
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

/// The project file given with `-f`, or `rocky.yaml`.
fn project_file(file: Option<PathBuf>) -> PathBuf {
    file.unwrap_or_else(|| PathBuf::from(project::SETTINGS_FILE))
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
