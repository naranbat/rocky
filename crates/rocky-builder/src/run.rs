//! Runs a job of the unsaved project on a background thread, as `rocky exec` would: the
//! job, then any jobs its triggers queued. Progress comes back as core's JSON events.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rocky_core::exec::{self, Log};
use rocky_core::project;
use rocky_core::trigger::Trigger;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::results::Results;

/// A project's text: its directory, its settings, and each job's (name, text).
pub struct Texts {
    pub root: PathBuf,
    pub settings: Option<String>,
    pub jobs: Vec<(String, String)>,
}

pub struct Run {
    pub results: Results,
    started: Instant,
    finished: Option<Duration>,
    events: UnboundedReceiver<Value>,
    /// Dropping or sending stops the run, killing its running commands.
    stop: Option<oneshot::Sender<()>>,
}

impl Run {
    /// Starts running `job` of the project `texts`.
    pub fn start(texts: Texts, job: &str) -> Run {
        let (sender, events) = unbounded_channel();
        let (stop, stopped) = oneshot::channel();
        let job = job.to_string();
        std::thread::spawn(move || {
            let end = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(async {
                    tokio::select! {
                        end = run(&texts, &job, &sender) => end,
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

    /// How long the run took, or has taken so far.
    pub fn elapsed(&self) -> Duration {
        self.finished.unwrap_or_else(|| self.started.elapsed())
    }
}

fn load(texts: &Texts) -> Result<project::Project, String> {
    project::load_parts(&texts.root, texts.settings.as_deref(), &texts.jobs)
        .map_err(|error| error.chain())
}

/// Runs `job`, then the jobs its triggers queue (and those they queue). Returns why the
/// first failing job failed, if one did.
async fn run(
    texts: &Texts,
    job: &str,
    sender: &UnboundedSender<Value>,
) -> Result<Option<String>, String> {
    let log = Log::Send(sender.clone());
    let (trigger, mut queue) = Trigger::channel();
    let outcome = exec::exec(load(texts)?, job, &log, false, &trigger)
        .await
        .map_err(|error| error.chain())?;
    drop(trigger);
    let mut failed = failure(job, &outcome);
    while let Some(queued) = queue.recv().await {
        let outcome = exec::exec(load(texts)?, &queued.job, &log, false, &queued.trigger)
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

    fn texts(jobs: &[(&str, &str)]) -> Texts {
        Texts {
            root: PathBuf::from("."),
            settings: None,
            jobs: jobs
                .iter()
                .map(|(name, text)| (name.to_string(), text.to_string()))
                .collect(),
        }
    }

    #[test]
    fn runs_the_job_then_triggered_jobs() {
        let main = "tasks:\n  first: {cmd: [echo, one]}\n  page: {trigger: other}\n";
        let other = "tasks:\n  only: {cmd: [echo, three]}\n";
        let mut run = Run::start(texts(&[("main", main), ("other", other)]), "main");
        wait(&mut run);
        let results = &run.results;
        assert!(matches!(
            results.end,
            Some(crate::results::End::Done { failed: None })
        ));
        let first = results.task("main", "first").unwrap();
        assert_eq!(first.last.as_ref().unwrap()["result"]["stdout"], "one");
        assert!(results.task("other", "only").is_some());
    }

    #[test]
    fn stop_ends_a_running_job() {
        let mut run = Run::start(
            texts(&[("slow", "tasks:\n  w: {cmd: [sleep, '30']}\n")]),
            "slow",
        );
        run.stop();
        wait(&mut run);
        assert!(matches!(&run.results.end, Some(crate::results::End::Error(e)) if e == "stopped"));
        assert!(run.elapsed() < Duration::from_secs(10));
    }
}
