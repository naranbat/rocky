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

/// Starts `commands` (the stages of a pipeline) in one process group, led by the first, and
/// tracks it. Holds the same lock as [`kill_running`] across the spawns, so a stop can't exit
/// while a command is starting but not yet tracked; once stopping, nothing new starts.
/// On failure, returns the index of the command that failed, and the ones already started
/// are killed.
pub fn spawn(commands: &mut [Command]) -> Result<(Vec<Child>, Group), (usize, io::Error)> {
    let mut running = running();
    if running.stopping {
        return Err((0, io::Error::other("rocky is stopping")));
    }
    let mut children = Vec::with_capacity(commands.len());
    let mut leader = None;
    for (index, command) in commands.iter_mut().enumerate() {
        command.process_group(leader.unwrap_or(0));
        let spawned = command.spawn().and_then(|child| {
            let id = child
                .id()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or_else(|| io::Error::other("no process id"))?;
            Ok((child, id))
        });
        match spawned {
            Ok((child, id)) => {
                leader.get_or_insert(id);
                children.push(child);
            }
            Err(source) => {
                if let Some(id) = leader {
                    kill(id);
                }
                return Err((index, source));
            }
        }
    }
    let id = leader.ok_or_else(|| (0, io::Error::other("no command")))?;
    running.groups.insert(id);
    let group = Group {
        id,
        finished: false,
    };
    Ok((children, group))
}

impl Group {
    /// The command finished on its own: stop tracking the group without killing it.
    pub fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        // Killed before the lock is released, so `kill_running` can't miss the group.
        let mut running = running();
        running.groups.remove(&self.id);
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
