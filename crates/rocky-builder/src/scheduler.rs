//! The scheduler: a `rocky run --socket` child process in the project's folder, started and
//! stopped from the builder. Its JSON log lines become each job's latest run, and the
//! schedules it loaded tell when the next job is due.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use jiff::Zoned;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use rocky_core::schedule::Schedule;
use serde_json::{Value, json};

use crate::results::Results;
use crate::run_view::Shown;

/// A line the child wrote: a JSON event on stdout, or text on stderr.
enum Line {
    Event(Value),
    Error(String),
}

/// One job's latest run, as the scheduler reported it.
struct JobRun {
    results: Results,
    started: Instant,
    finished: Option<Duration>,
}

pub struct Scheduler {
    child: Child,
    lines: Receiver<Line>,
    pub socket: PathBuf,
    /// The log file, relative to the project's folder.
    pub log: String,
    started: Zoned,
    schedules: Vec<(String, Schedule)>,
    runs: HashMap<String, JobRun>,
    /// Why the scheduler isn't running any more, once it isn't.
    pub exited: Option<String>,
    stopping: bool,
    stderr: Vec<String>,
}

/// The `rocky` binary: next to this one (as `cargo build` leaves it), or on `PATH`.
pub fn find_rocky() -> Option<PathBuf> {
    let sibling = std::env::current_exe().ok()?.with_file_name("rocky");
    if sibling.is_file() {
        return Some(sibling);
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("rocky"))
        .find(|path| path.is_file())
}

impl Scheduler {
    /// Starts `rocky run <project> --socket .rocky/rocky.sock` with `rocky`, in the
    /// project's folder.
    pub fn start(rocky: &Path, project: &Path) -> anyhow::Result<Scheduler> {
        let dir = project.parent().unwrap_or(Path::new("")).to_path_buf();
        let loaded = std::fs::read_to_string(project)
            .with_context(|| format!("cannot read {}", project.display()))?;
        let schedules = rocky_core::project::load_str(project, &loaded)
            .map_err(|error| anyhow::anyhow!(error.chain()))?
            .jobs
            .into_iter()
            .filter_map(|(name, job)| job.schedule.map(|schedule| (name, schedule)))
            .collect();
        std::fs::create_dir_all(dir.join(".rocky"))
            .with_context(|| format!("cannot create {}", dir.join(".rocky").display()))?;
        let socket = dir.join(".rocky/rocky.sock");
        let file = project.file_name().map(PathBuf::from).unwrap_or_default();
        let mut child = Command::new(rocky)
            .arg("run")
            .arg(&file)
            .arg("--socket")
            .arg(".rocky/rocky.sock")
            .current_dir(if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                &dir
            })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot start {}", rocky.display()))?;
        let (sender, lines) = channel();
        if let Some(stdout) = child.stdout.take() {
            forward(stdout, sender.clone(), |line| {
                match serde_json::from_str(&line) {
                    Ok(event) => Line::Event(event),
                    Err(_) => Line::Error(line),
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            forward(stderr, sender, Line::Error);
        }
        let started = Zoned::now();
        let log = format!(".rocky/logs/{}.log", started.strftime("%Y-%m-%d"));
        Ok(Scheduler {
            child,
            lines,
            socket,
            log,
            started,
            schedules,
            runs: HashMap::new(),
            exited: None,
            stopping: false,
            stderr: Vec::new(),
        })
    }

    /// Takes in what the scheduler wrote since the last call, and notices if it exited.
    pub fn poll(&mut self) {
        while let Ok(line) = self.lines.try_recv() {
            match line {
                Line::Event(event) => self.apply(event),
                Line::Error(text) => self.stderr.push(text),
            }
        }
        if self.exited.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.exited = Some(if self.stopping {
                "stopped".to_string()
            } else {
                let detail = self.stderr.join(" ");
                format!("exited ({status}) {detail}").trim().to_string()
            });
        }
    }

    /// Adds one log line to its job's latest run, starting a new run when the last one ended.
    fn apply(&mut self, mut event: Value) {
        let Some(job) = event["job"].as_str().map(str::to_string) else {
            return;
        };
        match event["event"].as_str() {
            Some("done") => {
                if let Some(task) = event["failed_task"].as_str() {
                    let reason = event["reason"].as_str().unwrap_or("failed");
                    event["failed"] = json!(format!("task `{task}` {reason}"));
                }
            }
            // Not part of a run: a past `at`, or a schedule that failed and stopped.
            Some("skipped" | "stopped") => return,
            _ => {}
        }
        let finished = self
            .runs
            .get(&job)
            .is_none_or(|run| run.results.end.is_some());
        if finished && event["event"] != "done" && event["event"] != "error" {
            self.runs.insert(
                job.clone(),
                JobRun {
                    results: Results::default(),
                    started: Instant::now(),
                    finished: None,
                },
            );
        }
        if let Some(run) = self.runs.get_mut(&job) {
            run.results.apply(&event);
            if run.results.end.is_some() && run.finished.is_none() {
                run.finished = Some(run.started.elapsed());
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.exited.is_none()
    }

    /// Asks the scheduler to stop, as Ctrl-C would: it kills running commands and exits.
    pub fn stop(&mut self) {
        if self.exited.is_none() {
            self.stopping = true;
            let _ = kill(Pid::from_raw(self.child.id() as i32), Signal::SIGTERM);
        }
    }

    /// `job`'s latest run, as the editor shows it.
    pub fn shown(&self, job: &str) -> Option<Shown<'_>> {
        let (name, run) = self.runs.get_key_value(job)?;
        Some(Shown {
            results: &run.results,
            job: name,
            until: None,
            started: run.started,
            elapsed: run.finished.unwrap_or_else(|| run.started.elapsed()),
            by: Some("scheduler"),
        })
    }

    /// Whether `job` is running in the scheduler now.
    pub fn is_running_job(&self, job: &str) -> bool {
        self.runs
            .get(job)
            .is_some_and(|run| run.results.end.is_none())
    }

    pub fn scheduled(&self) -> usize {
        self.schedules.len()
    }

    /// The job due next after `now`, and when.
    pub fn next_due(&self, now: &Zoned) -> Option<(&str, Zoned)> {
        self.schedules
            .iter()
            .filter_map(|(name, schedule)| {
                next_run(schedule, &self.started, now).map(|at| (name.as_str(), at))
            })
            .min_by(|a, b| a.1.cmp(&b.1))
    }
}

/// When a job with `schedule`, under a scheduler started at `started`, next runs after `now`.
fn next_run(schedule: &Schedule, started: &Zoned, now: &Zoned) -> Option<Zoned> {
    match schedule {
        Schedule::Every { interval, delay } => {
            let first = started.checked_add(*delay).ok()?;
            if first > *now {
                return Some(first);
            }
            let since = Duration::try_from(first.duration_until(now)).ok()?;
            let interval_ns = interval.as_nanos().max(1);
            let ticks = since.as_nanos() / interval_ns + 1;
            let ahead = Duration::from_nanos(u64::try_from(ticks * interval_ns).ok()?);
            first.checked_add(ahead).ok()
        }
        Schedule::Clock { clock, .. } => clock.next_after(now).ok().flatten(),
        Schedule::OnStart => None,
    }
}

/// Reads `source` line by line on its own thread, sending each line through `wrap`.
fn forward(
    source: impl std::io::Read + Send + 'static,
    sender: Sender<Line>,
    wrap: impl Fn(String) -> Line + Send + 'static,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(source).lines().map_while(Result::ok) {
            if sender.send(wrap(line)).is_err() {
                break;
            }
        }
    });
}

impl Drop for Scheduler {
    /// Closing the builder stops its scheduler too.
    fn drop(&mut self) {
        self.stop();
    }
}

/// Queues a run of `job` on the scheduler's `socket`, as `rocky trigger` does.
pub fn trigger(socket: &Path, job: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime
        .block_on(rocky_core::socket::send(socket, job))
        .map_err(|error| error.chain())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A stand-in for `rocky` that logs a run of `main` and then waits to be stopped.
    const FAKE: &str = r#"#!/bin/sh
echo '{"job":"main","event":"task","task":"a","exit_code":0,"ms":3,"ok":true}'
echo '{"job":"main","event":"done","failed_task":null,"reason":null}'
echo '{"job":"main","event":"task","task":"a","exit_code":1,"ms":2,"ok":false}'
echo '{"job":"main","event":"done","failed_task":"a","reason":"exited with code 1"}'
echo "starting" >&2
exec sleep 30
"#;

    fn wait_until(scheduler: &mut Scheduler, done: impl Fn(&Scheduler) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(scheduler) && Instant::now() < deadline {
            scheduler.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn reads_runs_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let rocky = dir.path().join("rocky");
        std::fs::write(&rocky, FAKE).unwrap();
        std::fs::set_permissions(&rocky, std::fs::Permissions::from_mode(0o755)).unwrap();
        let project = dir.path().join("project.yaml");
        std::fs::write(
            &project,
            "jobs:\n  main:\n    schedule: {every: 1h}\n    tasks:\n      a: {cmd: [echo]}\n",
        )
        .unwrap();

        let mut scheduler = Scheduler::start(&rocky, &project).unwrap();
        wait_until(&mut scheduler, |s| {
            s.shown("main").is_some_and(|run| {
                run.results.end.is_some()
                    && run
                        .results
                        .task("main", "a")
                        .is_some_and(|a| a.failures == 1)
            })
        });
        let run = scheduler.shown("main").unwrap();
        assert_eq!(
            run.results.task("main", "a").unwrap().runs,
            1,
            "a new run starts after done"
        );
        assert!(
            matches!(&run.results.end, Some(crate::results::End::Done { failed: Some(reason) }) if reason.contains("exited with code 1"))
        );
        assert_eq!(scheduler.scheduled(), 1);
        assert!(
            scheduler
                .next_due(&Zoned::now())
                .is_some_and(|(job, _)| job == "main")
        );
        assert!(dir.path().join(".rocky").is_dir());

        assert!(scheduler.is_running());
        scheduler.stop();
        wait_until(&mut scheduler, |s| !s.is_running());
        assert_eq!(scheduler.exited.as_deref(), Some("stopped"));
    }

    #[test]
    fn every_runs_count_from_the_start() {
        let started: Zoned = "2026-11-06T10:00:00[UTC]".parse().unwrap();
        let every = Schedule::Every {
            interval: Duration::from_secs(300),
            delay: Duration::from_secs(60),
        };
        let at = |time: &str| time.parse::<Zoned>().unwrap();
        assert_eq!(
            next_run(&every, &started, &at("2026-11-06T10:00:30[UTC]")),
            Some(at("2026-11-06T10:01:00[UTC]"))
        );
        assert_eq!(
            next_run(&every, &started, &at("2026-11-06T10:03:00[UTC]")),
            Some(at("2026-11-06T10:06:00[UTC]"))
        );
        assert_eq!(next_run(&Schedule::OnStart, &started, &started), None);
    }
}
