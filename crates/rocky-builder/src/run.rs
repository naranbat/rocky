//! Runs a job of the unsaved document on a background thread, as `rocky exec` would: the
//! job, then any jobs its triggers queued. Progress comes back as core's JSON events.

use std::path::Path;
use std::time::{Duration, Instant};

use rocky_core::exec::{self, Log};
use rocky_core::trigger::Trigger;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::results::{Results, Shown};

pub struct Run {
    /// The job started, and the task it stops after, if any.
    pub job: String,
    pub until: Option<String>,
    pub results: Results,
    started: Instant,
    finished: Option<Duration>,
    events: UnboundedReceiver<Value>,
    /// Dropping or sending stops the run, killing its running commands.
    stop: Option<oneshot::Sender<()>>,
}

impl Run {
    /// Starts running `job` of the project in directory `path`, with `text` as its
    /// contents. With `until`, the job's tasks after that one don't run.
    pub fn start(path: &Path, text: &str, job: &str, until: Option<&str>) -> Run {
        let (sender, events) = unbounded_channel();
        let (stop, stopped) = oneshot::channel();
        let (path, text) = (path.to_path_buf(), text.to_string());
        let (name, last) = (job.to_string(), until.map(str::to_string));
        std::thread::spawn(move || {
            let end = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(async {
                    tokio::select! {
                        end = run(&path, &text, &name, last.as_deref(), &sender) => end,
                        _ = stopped => Err("stopped".to_string()),
                    }
                }),
                Err(error) => Err(format!("cannot start a runtime: {error}")),
            };
            let event = match end {
                Ok(failed) => json!({"event": "done", "failed": failed}),
                Err(error) => json!({"event": "error", "error": error}),
            };
            let _ = sender.send(event);
        });
        Run {
            job: job.to_string(),
            until: until.map(str::to_string),
            results: Results::default(),
            started: Instant::now(),
            finished: None,
            events,
            stop: Some(stop),
        }
    }

    /// Takes in the events that arrived since the last call.
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.results.apply(&event);
        }
        if self.results.end.is_some() && self.finished.is_none() {
            self.finished = Some(self.started.elapsed());
            self.stop = None;
        }
    }

    pub fn is_running(&self) -> bool {
        self.results.end.is_none()
    }

    /// Stops the run; its commands are killed.
    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }

    /// This run, as the editor shows it.
    pub fn shown(&self) -> Shown<'_> {
        Shown {
            results: &self.results,
            job: &self.job,
            until: self.until.as_deref(),
            started: self.started,
            elapsed: self.elapsed(),
            by: None,
        }
    }

    /// How long the run took, or has taken so far.
    pub fn elapsed(&self) -> Duration {
        self.finished.unwrap_or_else(|| self.started.elapsed())
    }
}

/// Runs `job`, then the jobs its triggers queue (and those they queue), each from `text`.
/// Returns why the first failing job failed, if one did.
async fn run(
    path: &Path,
    text: &str,
    job: &str,
    until: Option<&str>,
    sender: &UnboundedSender<Value>,
) -> Result<Option<String>, String> {
    let log = Log::Send(sender.clone());
    let (trigger, mut queue) = Trigger::channel();
    let mut project = crate::parts::load(path, text).map_err(|error| error.chain())?;
    if let Some(until) = until
        && let Some(tasks) = project.jobs.get_mut(job).map(|job| &mut job.tasks)
        && let Some(index) = tasks.iter().position(|task| task.name == until)
    {
        tasks.truncate(index + 1);
    }
    let outcome = exec::exec(project, job, &log, false, &trigger)
        .await
        .map_err(|error| error.chain())?;
    drop(trigger);
    let mut failed = failure(job, &outcome);
    while let Some(queued) = queue.recv().await {
        let project = crate::parts::load(path, text).map_err(|error| error.chain())?;
        let outcome = exec::exec(project, &queued.job, &log, false, &queued.trigger)
            .await
            .map_err(|error| error.chain())?;
        failed = failed.or_else(|| failure(&queued.job, &outcome));
    }
    Ok(failed)
}

fn failure(job: &str, outcome: &exec::Outcome) -> Option<String> {
    let task = outcome.failed_task.as_ref()?;
    let reason = outcome.reason().unwrap_or_default();
    Some(format!("job `{job}`: task `{task}` {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(run: &mut Run) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while run.is_running() && Instant::now() < deadline {
            run.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    const YAML: &str = "\
jobs:
  main:
    tasks:
      first: {cmd: [echo, one]}
      page: {trigger: other}
      last: {cmd: [echo, two]}
  other:
    tasks:
      only: {cmd: [echo, three]}
";

    #[test]
    fn runs_the_job_then_triggered_jobs() {
        let mut run = Run::start(Path::new("."), YAML, "main", None);
        wait(&mut run);
        let results = &run.results;
        assert!(matches!(
            results.end,
            Some(crate::results::End::Done { failed: None })
        ));
        let first = results.task("main", "first").unwrap();
        assert_eq!(first.last.as_ref().unwrap()["result"]["stdout"], "one");
        assert!(results.task("main", "last").is_some());
        assert!(results.task("other", "only").is_some());
    }

    #[test]
    fn until_stops_after_that_task() {
        let mut run = Run::start(Path::new("."), YAML, "main", Some("first"));
        wait(&mut run);
        assert!(run.results.task("main", "first").is_some());
        assert!(run.results.task("main", "last").is_none());
        assert!(run.results.task("other", "only").is_none());
    }

    #[test]
    fn stop_ends_a_running_job() {
        let yaml = "jobs:\n  slow:\n    tasks:\n      wait: {cmd: [sleep, '30']}\n";
        let mut run = Run::start(Path::new("."), yaml, "slow", None);
        run.stop();
        wait(&mut run);
        assert!(matches!(&run.results.end, Some(crate::results::End::Error(e)) if e == "stopped"));
        assert!(run.elapsed() < Duration::from_secs(10));
    }
}
