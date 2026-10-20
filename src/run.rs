use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use jiff::Zoned;
use serde_json::json;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

use crate::error::Error;
use crate::job_lock::JobLocks;
use crate::schedule::{Clock, Schedule};
use crate::{exec, progress, project, socket};

/// Runs every job that has a `schedule` until all schedules are done (only `at`
/// jobs ever finish). Every log line is one JSON object on stdout (also appended to
/// `logs/<date>.log`) with `job`, `time` and `event` fields. Each run reloads the
/// project and prints a line per task, branch and loop, then a `done` line (with `failed_task` and its `reason` if a task
/// failed); a run that errors or panics prints an `error` line. Either way the job's next
/// runs still happen. An `at` job whose time has passed (without `run_on_start`) prints a
/// `skipped` line instead of running. A job whose schedule fails prints a `stopped` line and stops;
/// the other jobs keep running.
///
/// With a `socket`, `run` also runs the jobs that clients of it trigger (see [`socket`]),
/// doesn't need any job to have a `schedule`, and runs until stopped. A job's runs,
/// scheduled or triggered, never overlap: each waits for the one before it.
pub async fn run(path: &Path, socket: Option<&Path>) -> Result<(), Error> {
    let project = project::load(path)?;
    let scheduled: Vec<_> = project
        .jobs
        .into_iter()
        .filter_map(|(name, job)| job.schedule.map(|schedule| (name, schedule)))
        .collect();
    if scheduled.is_empty() && socket.is_none() {
        return Err(Error::InvalidProject("no job has a `schedule`".to_string()));
    }
    let listener = match socket {
        Some(socket) => Some((socket::bind(socket).await?, socket)),
        None => None,
    };
    let locks = JobLocks::default();
    let now = Zoned::now();
    let mut jobs = JoinSet::new();
    let mut names = HashMap::new();
    for (name, schedule) in scheduled {
        if let Schedule::Clock {
            clock,
            run_on_start: false,
        } = &schedule
            && clock.next_after(&now)?.is_none()
        {
            progress::emit(
                &name,
                json!({"event": "skipped", "reason": "`at` is in the past"}),
            )
            .await;
            continue;
        }
        let path = path.to_path_buf();
        let job = name.clone();
        let started = now.clone();
        let locks = locks.clone();
        let task = jobs.spawn(async move { run_job(&path, &job, schedule, started, &locks).await });
        names.insert(task.id(), name);
    }
    let schedules = async {
        while let Some(joined) = jobs.join_next_with_id().await {
            let (id, error) = match joined {
                Ok((_, Ok(()))) => continue,
                Ok((id, Err(err))) => (id, err.chain()),
                Err(err) => (err.id(), err.to_string()),
            };
            let name = names.get(&id).map_or("", String::as_str);
            progress::emit(name, json!({"event": "stopped", "error": error})).await;
        }
        Ok(())
    };
    match listener {
        Some((listener, socket)) => {
            tokio::try_join!(schedules, socket::serve(listener, socket, path, locks))?;
            Ok(())
        }
        None => schedules.await,
    }
}

async fn run_job(
    path: &Path,
    name: &str,
    schedule: Schedule,
    started: Zoned,
    locks: &JobLocks,
) -> Result<(), Error> {
    match &schedule {
        Schedule::Every { interval, delay } => {
            tokio::time::sleep(*delay).await;
            let mut ticks = tokio::time::interval(*interval);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticks.tick().await;
                run_once(path, name, locks).await;
            }
        }
        Schedule::Clock {
            clock,
            run_on_start,
        } => {
            // `started` is the time the past-`at` check used, so an `at` it let through isn't
            // lost if that time passes before this task starts.
            if *run_on_start {
                run_once(path, name, locks).await;
            }
            // An `at` time passed during the start run still runs, late: it's the only one.
            let mut after = match clock {
                Clock::At(_) => started,
                _ => Zoned::now(),
            };
            while let Some(next) = clock.next_after(&after)? {
                // Negative when a slot was reached late: run now.
                let wait = Zoned::now().duration_until(&next);
                tokio::time::sleep(Duration::try_from(wait).unwrap_or_default()).await;
                run_once(path, name, locks).await;
                // Slots that passed during a long run are skipped.
                after = Zoned::now().max(next);
            }
            Ok(())
        }
    }
}

/// Runs the job once, after any run of it already going, in its own task, so even a panic
/// is logged as an `error` line and the schedule carries on.
pub(crate) async fn run_once(path: &Path, name: &str, locks: &JobLocks) {
    let lock = locks.get(name).await;
    let _turn = lock.lock().await;
    let (path, job) = (path.to_path_buf(), name.to_string());
    let outcome = tokio::spawn(async move {
        let project = tokio::task::spawn_blocking(move || project::load(&path)).await??;
        exec::exec(project, &job, true).await
    })
    .await;
    let event = match outcome {
        Ok(Ok(outcome)) => {
            let reason = outcome
                .failed_task
                .as_ref()
                .and(outcome.result.as_ref().and_then(exec::TaskResult::failure));
            json!({"event": "done", "failed_task": outcome.failed_task, "reason": reason})
        }
        Ok(Err(err)) => {
            let error = err.chain();
            json!({"event": "error", "error": error})
        }
        Err(err) => json!({"event": "error", "error": err.to_string()}),
    };
    progress::emit(name, event).await;
}
