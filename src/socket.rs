//! `run --socket`: a Unix domain socket that triggers jobs. Each line a client sends is a
//! request `{"job": "NAME"}`; each gets one JSON line back, `{"ok": true}` once the run is
//! queued, or `{"error": "..."}`. The run itself is logged like a scheduled one.

use std::fs::Permissions;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::error::Error;
use crate::job_lock::JobLocks;
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

/// Serves clients of `listener` until accepting a connection fails. `project` is the
/// project file that triggered runs load.
pub async fn serve(
    listener: UnixListener,
    socket: &Path,
    project: &Path,
    locks: JobLocks,
) -> Result<(), Error> {
    loop {
        let (stream, _) = listener.accept().await.map_err(|source| Error::Socket {
            path: socket.to_path_buf(),
            source,
        })?;
        tokio::spawn(client(stream, project.to_path_buf(), locks.clone()));
    }
}

/// Answers each request line until the client hangs up.
async fn client(stream: UnixStream, project: PathBuf, locks: JobLocks) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let reply = trigger(&line, &project, &locks).await;
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
async fn trigger(line: &str, project: &Path, locks: &JobLocks) -> Value {
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
    let (path, locks) = (project.to_path_buf(), locks.clone());
    tokio::spawn(async move { run::run_once(&path, &job, &locks).await });
    json!({"ok": true})
}
