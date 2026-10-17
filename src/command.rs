use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::Error;
use crate::process_group::Group;

/// A finished command with its output parts kept separate for independent use.
#[derive(Debug, Serialize, PartialEq)]
pub struct CommandResult {
    /// `-1` when the process was killed by a signal.
    pub exit_code: i32,
    /// Parsed JSON when stdout is valid JSON, otherwise text without its trailing newline.
    pub stdout: Value,
    pub stderr: String,
}

/// Runs `argv` directly (no shell) in `cwd` (default: the current directory) with `env` added
/// to the inherited environment, writes `stdin` to it when given, and waits for it to finish. After `timeout` seconds the command is killed and the run fails.
/// Without `capture_stdout`, stdout goes to /dev/null and the result's `stdout` is `null`.
pub async fn run(
    argv: &[String],
    stdin: Option<&str>,
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
    timeout: Option<u64>,
    capture_stdout: bool,
) -> Result<CommandResult, Error> {
    let (program, arguments) = argv
        .split_first()
        .ok_or_else(|| Error::InvalidStep("`cmd` is empty".to_string()))?;
    let spawn_error = |source| Error::Spawn {
        command: program.clone(),
        source,
    };
    let mut command = Command::new(program);
    command
        .args(arguments)
        .envs(env)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        // Its own process group, so a timeout or a stop signal kills what it started too.
        .process_group(0)
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(|source| match cwd {
        Some(dir) if !dir.is_dir() => Error::Cwd(dir.to_path_buf()),
        _ => spawn_error(source),
    })?;
    let id = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| spawn_error(std::io::Error::other("no process id")))?;
    // Dropped unfinished on timeout, which kills the group.
    let group = Group::new(id);
    let pipe = child.stdin.take();
    let write = async move {
        if let (Some(mut pipe), Some(text)) = (pipe, stdin) {
            // A command may exit without reading it all, so a write error (broken pipe)
            // is not a failure. Dropping the pipe afterwards closes the command's stdin.
            let _ = pipe.write_all(text.as_bytes()).await;
        }
    };
    // Written while the output is read, so a command that fills stdout before reading
    // stdin can't deadlock.
    let finish = async move { tokio::join!(write, child.wait_with_output()).1 };
    let output = match timeout {
        // On timeout the child is dropped, which kills it (`kill_on_drop`), and so is
        // `group`, which kills what it started.
        Some(secs) => tokio::time::timeout(std::time::Duration::from_secs(secs), finish)
            .await
            .map_err(|_| Error::CommandTimeout {
                command: program.clone(),
                secs,
            })?,
        None => finish.await,
    }
    .map_err(spawn_error)?;
    group.finish();
    Ok(CommandResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: if capture_stdout {
            output_value(&output.stdout)
        } else {
            Value::Null
        },
        stderr: text(&output.stderr),
    })
}

pub(crate) fn output_value(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|_| Value::String(text(bytes)))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches(['\n', '\r'])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(ToString::to_string).collect()
    }

    #[tokio::test]
    async fn json_stdout_is_parsed() {
        let result = run(
            &argv(&["echo", r#"{"a":1}"#]),
            None,
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, json!({"a": 1}));
    }

    #[tokio::test]
    async fn text_stdout_loses_trailing_newline() {
        let result = run(
            &argv(&["echo", "hello world"]),
            None,
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.stdout, json!("hello world"));
    }

    #[tokio::test]
    async fn captures_stderr_and_exit_code() {
        let result = run(
            &argv(&["sh", "-c", "echo oops >&2; exit 3"]),
            None,
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 3);
        assert_eq!(result.stderr, "oops");
        assert_eq!(result.stdout, json!(""));
    }

    #[tokio::test]
    async fn uncaptured_stdout_is_null() {
        let result = run(
            &argv(&["sh", "-c", "echo out; echo oops >&2; exit 3"]),
            None,
            &BTreeMap::new(),
            None,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(result.stdout, Value::Null);
        assert_eq!(result.stderr, "oops");
        assert_eq!(result.exit_code, 3);
    }

    #[tokio::test]
    async fn killed_by_a_signal_exits_with_minus_one() {
        let result = run(
            &argv(&["sh", "-c", "kill -9 $$"]),
            None,
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, -1);
    }

    #[tokio::test]
    async fn stdin_is_written_to_the_command() {
        let result = run(
            &argv(&["cat"]),
            Some(r#"{"n": 2}"#),
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.stdout, json!({"n": 2}));
        let result = run(
            &argv(&["wc", "-l"]),
            Some("a\nb\n"),
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.stdout, json!(2));
    }

    #[tokio::test]
    async fn large_stdin_and_stdout_do_not_deadlock() {
        let big = "x".repeat(1 << 20);
        let result = run(
            &argv(&["cat"]),
            Some(&big),
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.stdout, json!(big));
    }

    /// Whether process `pid` is gone, waiting up to 2s for it to be killed. A zombie counts
    /// as gone: in a container without an init process, nothing may reap it.
    fn gone(pid: i32) -> bool {
        (0..40).any(|_| {
            // The state follows the `)` closing the command name.
            let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                stat.rsplit_once(')')
                    .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
            });
            // Signal 0 only checks that the process exists.
            let alive =
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() && !zombie;
            if alive {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            !alive
        })
    }

    #[tokio::test]
    async fn timeout_kills_the_commands_children() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
        let err = run(
            &argv(&["sh", "-c", &script]),
            None,
            &BTreeMap::new(),
            None,
            Some(1),
            true,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::CommandTimeout { .. }), "{err}");
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(gone(pid), "`sleep` ({pid}) still runs");
    }

    #[tokio::test]
    async fn command_that_ignores_stdin_still_succeeds() {
        let big = "x".repeat(1 << 20);
        let result = run(
            &argv(&["true"]),
            Some(&big),
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn without_stdin_the_command_reads_nothing() {
        let result = run(&argv(&["cat"]), None, &BTreeMap::new(), None, None, true)
            .await
            .unwrap();
        assert_eq!(result.stdout, json!(""));
    }

    #[tokio::test]
    async fn missing_cwd_is_named() {
        let err = run(
            &argv(&["echo"]),
            None,
            &BTreeMap::new(),
            Some(Path::new("/rocky-no-such-dir")),
            None,
            true,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, Error::Cwd(dir) if dir == Path::new("/rocky-no-such-dir")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn missing_program_is_spawn_error() {
        let err = run(
            &argv(&["rocky-no-such-program"]),
            None,
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Spawn { command, .. } if command == "rocky-no-such-program"));
    }

    #[tokio::test]
    async fn timeout_fails_with_command_timeout() {
        let started = std::time::Instant::now();
        let err = run(
            &argv(&["sleep", "5"]),
            None,
            &BTreeMap::new(),
            None,
            Some(1),
            true,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, Error::CommandTimeout { ref command, secs: 1 } if command == "sleep"),
            "{err}"
        );
        assert!(started.elapsed().as_secs() < 3);
    }

    #[tokio::test]
    async fn runs_in_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let result = run(
            &argv(&["pwd"]),
            None,
            &BTreeMap::new(),
            Some(dir.path()),
            None,
            true,
        )
        .await
        .unwrap();
        let printed = std::path::PathBuf::from(result.stdout.as_str().unwrap());
        assert_eq!(
            printed.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn env_is_added_to_the_inherited_environment() {
        let env = BTreeMap::from([("A".to_string(), "x y".to_string())]);
        let result = run(
            &argv(&["sh", "-c", r#"printf "%s" "$A""#]),
            None,
            &env,
            None,
            None,
            true,
        )
        .await
        .unwrap();
        // `sh` is found, so `PATH` is still inherited.
        assert_eq!(result.stdout, json!("x y"));
    }

    #[tokio::test]
    async fn empty_argv_is_invalid() {
        assert!(matches!(
            run(&[], None, &BTreeMap::new(), None, None, true)
                .await
                .unwrap_err(),
            Error::InvalidStep(_)
        ));
    }
}
