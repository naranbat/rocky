//! Commands run in their own process group, so a timeout or a stop signal can kill
//! the commands they started too, not just the direct child.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Process groups of commands still running.
static RUNNING: Mutex<BTreeSet<i32>> = Mutex::new(BTreeSet::new());

/// A running command's process group. Dropping it before [`Group::finish`] (a timeout, or a
/// cancelled run) kills the whole group.
pub struct Group {
    id: i32,
    finished: bool,
}

impl Group {
    /// Tracks the group led by the command with process id `id`.
    pub fn new(id: i32) -> Self {
        running().insert(id);
        Group {
            id,
            finished: false,
        }
    }

    /// The command finished on its own: stop tracking the group without killing it.
    pub fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        running().remove(&self.id);
        if !self.finished {
            kill(self.id);
        }
    }
}

/// Kills every running command's process group, for when `rocky` is told to stop.
pub fn kill_running() {
    for &id in running().iter() {
        kill(id);
    }
}

fn kill(group: i32) {
    // SAFETY: `kill` takes no pointers. A negative id signals the whole process group; the
    // id can't be reused while any process of the group still runs.
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
}

/// Each change is one insert or remove, so a poisoned lock is still safe to use.
fn running() -> MutexGuard<'static, BTreeSet<i32>> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}
