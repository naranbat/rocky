//! The queue of runs that `trigger` tasks ask for.

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Queues runs of jobs that `trigger` tasks ask for.
#[derive(Clone)]
pub struct Trigger(UnboundedSender<Queued>);

/// A queued run of `job`, with the [`Trigger`] that run's own `trigger` tasks use. Each
/// request carries one, so the queue closes once nothing that could still trigger a run is
/// running or queued.
pub struct Queued {
    pub job: String,
    pub trigger: Trigger,
}

impl Trigger {
    pub fn channel() -> (Trigger, UnboundedReceiver<Queued>) {
        let (sender, queue) = mpsc::unbounded_channel();
        (Trigger(sender), queue)
    }

    /// Queues a run of `job`.
    pub fn send(&self, job: &str) {
        let queued = Queued {
            job: job.to_string(),
            trigger: self.clone(),
        };
        // The queue only closes while `rocky` is stopping, when the run wouldn't start anyway.
        let _ = self.0.send(queued);
    }
}
