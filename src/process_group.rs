//! Commands run in their own process group, so a timeout or a stop signal can kill
//! the commands they started too, not just the direct child.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::process::{Child, Command};

/// Process groups of commands still running, and whether `rocky` is stopping.
struct Running {
    groups: BTreeSet<i32>,
    stopping: bool,
}

static RUNNING: Mutex<Running> = Mutex::new(Running {
    groups: BTreeSet::new(),
    stopping: false,
});

/// A running command's process group. Dropping it before [`Group::finish`] (a timeout, or a
/// cancelled run) kills the whole group.
pub struct Group {
    id: i32,
    finished: bool,
}

/// Starts `command` and tracks the group it leads. Holds the same lock as [`kill_running`],
/// so a stop can't miss a command that's starting; once stopping, nothing new starts.
pub fn spawn(command: &mut Command) -> io::Result<(Child, Group)> {
    let mut running = running();
    if running.stopping {
        return Err(io::Error::other("rocky is stopping"));
    }
    let child = command.spawn()?;
    let id = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| io::Error::other("no process id"))?;
    running.groups.insert(id);
    let group = Group {
        id,
        finished: false,
    };
    Ok((child, group))
}

impl Group {
    /// The command finished on its own: stop tracking the group without killing it.
    pub fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        running().groups.remove(&self.id);
        if !self.finished {
            kill(self.id);
        }
    }
}

/// Kills every running command's process group, for when `rocky` is told to stop.
/// Commands started after this are refused.
pub fn kill_running() {
    let mut running = running();
    running.stopping = true;
    for &id in &running.groups {
        kill(id);
    }
}

fn kill(group: i32) {
    // The id can't be reused while any process of the group still runs.
    let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
}

/// Each change is one insert, remove or flag, so a poisoned lock is still safe to use.
fn running() -> MutexGuard<'static, Running> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}
