//! `run --socket`: a Unix domain socket that triggers jobs. Each line a client sends is a
//! request `{"job": "NAME"}`; each gets one JSON line back, `{"ok": true}` once the run is
//! queued, or `{"error": "..."}`. The run itself is logged like a scheduled one. `rocky
//! trigger` is the client.

use std::fs::Permissions;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::error::Error;
use crate::job_lock::JobLocks;
use crate::trigger::Trigger;
use crate::{progress, project, run};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    job: String,
}

/// Listens on `path`, which only its owner can connect to. A socket file left behind by a
/// stopped `run` is replaced; one that another process still listens on is an error.
pub async fn bind(path: &Path) -> Result<UnixListener, Error> {
    let failed = |source| Error::Socket {
        path: path.to_path_buf(),
        source,
    };
    let stale = std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_socket());
    if stale {
        if UnixStream::connect(path).await.is_ok() {
            return Err(Error::SocketInUse(path.to_path_buf()));
        }
        std::fs::remove_file(path).map_err(failed)?;
    }
    let listener = UnixListener::bind(path).map_err(failed)?;
    std::fs::set_permissions(path, Permissions::from_mode(0o600)).map_err(failed)?;
    Ok(listener)
}

/// Asks the `run` listening on `socket` to run `job`, and returns once the run is queued.
/// An error reply from `run` becomes the error.
pub async fn send(socket: &Path, job: &str) -> Result<(), Error> {
    let failed = |source| Error::Socket {
        path: socket.to_path_buf(),
        source,
    };
    let stream = UnixStream::connect(socket).await.map_err(failed)?;
    let (read, mut write) = stream.into_split();
    let request = json!({"job": job});
    write
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(failed)?;
    let mut reply = String::new();
    BufReader::new(read)
        .read_line(&mut reply)
        .await
        .map_err(failed)?;
    let reply: Value = serde_json::from_str(&reply).unwrap_or_default();
    if reply["ok"] == true {
        return Ok(());
    }
    Err(Error::Trigger(match reply["error"].as_str() {
        Some(error) => error.to_string(),
        None => format!("unexpected reply from socket {}", socket.display()),
    }))
}

/// Serves clients of `listener` until accepting a connection fails. `project` is the
/// project file that triggered runs load; their `trigger` tasks queue runs on `trigger`.
pub async fn serve(
    listener: UnixListener,
    socket: &Path,
    project: &Path,
    locks: JobLocks,
    trigger: Trigger,
) -> Result<(), Error> {
    loop {
        let (stream, _) = listener.accept().await.map_err(|source| Error::Socket {
            path: socket.to_path_buf(),
            source,
        })?;
        tokio::spawn(client(
            stream,
            project.to_path_buf(),
            locks.clone(),
            trigger.clone(),
        ));
    }
}

/// Answers each request line until the client hangs up.
async fn client(stream: UnixStream, project: PathBuf, locks: JobLocks, trigger: Trigger) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = request(&line, &project, &locks, &trigger).await;
        if write
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Checks that the request names a job of the project, then queues a run of it.
async fn request(line: &str, project: &Path, locks: &JobLocks, trigger: &Trigger) -> Value {
    let job = match serde_json::from_str::<Request>(line) {
        Ok(request) => request.job,
        Err(err) => return json!({"error": format!("invalid request: {err}")}),
    };
    let path = project.to_path_buf();
    let name = job.clone();
    let found = tokio::task::spawn_blocking(move || project::load(&path)?.take(&name).map(drop))
        .await
        .map_err(Error::from)
        .and_then(|found| found);
    if let Err(err) = found {
        return json!({"error": err.chain()});
    }
    progress::emit(&job, json!({"event": "triggered"})).await;
    let (path, locks, trigger) = (project.to_path_buf(), locks.clone(), trigger.clone());
    tokio::spawn(async move { run::run_once(&path, &job, &locks, &trigger).await });
    json!({"ok": true})
}
