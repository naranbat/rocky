use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::Error;
use crate::process_group;

/// A finished command with its output parts kept separate for independent use.
#[derive(Debug, Serialize, PartialEq)]
pub struct CommandResult {
    /// `-1` when the process was killed by a signal.
    pub exit_code: i32,
    /// Parsed JSON when stdout is valid JSON, otherwise text without its trailing newline.
    pub stdout: Value,
    pub stderr: String,
}

/// Runs `stages` (each a program and its arguments) directly, with no shell, each stage's
/// stdout piped into the next one's stdin. Runs in `cwd` (default: the current directory)
/// with `env` added to the inherited environment, writes `stdin` to the first stage when
/// given, and waits for every stage to finish. After `timeout` seconds every stage is killed
/// and the run fails. Without `capture_stdout`, the last stage's stdout goes to /dev/null and
/// the result's `stdout` is `null`.
pub async fn run(
    stages: &[Vec<String>],
    stdin: Option<&str>,
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
    timeout: Option<u64>,
    capture_stdout: bool,
) -> Result<CommandResult, Error> {
    let empty = || Error::InvalidTask("`cmd` is empty".to_string());
    let programs = stages
        .iter()
        .map(|argv| argv.first().ok_or_else(empty))
        .collect::<Result<Vec<_>, _>>()?;
    let first = *programs.first().ok_or_else(empty)?;
    let spawn_error = |program: &String, source| Error::Spawn {
        command: program.clone(),
        source,
    };
    let mut commands = Vec::with_capacity(stages.len());
    let mut next_stdin = if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    for (index, argv) in stages.iter().enumerate() {
        let (program, arguments) = argv.split_first().ok_or_else(empty)?;
        let input = std::mem::replace(&mut next_stdin, Stdio::null());
        let stdout = if index + 1 < stages.len() {
            let (reader, writer) =
                std::io::pipe().map_err(|source| spawn_error(program, source))?;
            next_stdin = reader.into();
            writer.into()
        } else if capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        };
        let mut command = Command::new(program);
        command
            .args(arguments)
            .envs(env)
            .stdin(input)
            .stdout(stdout)
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        commands.push(command);
    }
    // One process group for all stages, so a timeout or a stop signal kills what they started
    // too. `group` is dropped unfinished on timeout, which kills the group.
    let (mut children, group) =
        process_group::spawn(&mut commands).map_err(|(index, source)| match cwd {
            Some(dir) if !dir.is_dir() => Error::Cwd(dir.to_path_buf()),
            _ => spawn_error(programs[index], source),
        })?;
    // The commands hold the parent's copies of the pipe ends; a stage sees EOF only once
    // they are closed.
    drop(commands);
    let pipe = children.first_mut().and_then(|child| child.stdin.take());
    let write = async move {
        if let (Some(mut pipe), Some(text)) = (pipe, stdin) {
            // A command may exit without reading it all, so a write error (broken pipe)
            // is not a failure. Dropping the pipe afterwards closes the command's stdin.
            let _ = pipe.write_all(text.as_bytes()).await;
        }
    };
    // Every stage is read at once, so none blocks on a full stderr pipe.
    let waits: Vec<_> = children
        .into_iter()
        .map(|child| tokio::spawn(child.wait_with_output()))
        .collect();
    let wait_all = async move {
        let mut outputs = Vec::with_capacity(waits.len());
        for wait in waits {
            outputs.push(wait.await.map_err(std::io::Error::other)??);
        }
        Ok(outputs)
    };
    // Written while the output is read, so a command that fills stdout before reading
    // stdin can't deadlock.
    let finish = async move { tokio::join!(write, wait_all).1 };
    let outputs = match timeout {
        // On timeout `group` is dropped, which kills every stage and what it started.
        Some(secs) => tokio::time::timeout(std::time::Duration::from_secs(secs), finish)
            .await
            .map_err(|_| Error::CommandTimeout {
                command: first.clone(),
                secs,
            })?,
        None => finish.await,
    }
    .map_err(|source| spawn_error(first, source))?;
    group.finish();
    Ok(CommandResult {
        // Like `set -o pipefail`: the last non-zero exit code.
        exit_code: outputs
            .iter()
            .map(|output| output.status.code().unwrap_or(-1))
            .rfind(|&code| code != 0)
            .unwrap_or(0),
        stdout: match outputs.last() {
            Some(output) if capture_stdout => output_value(&output.stdout),
            _ => Value::Null,
        },
        stderr: outputs
            .iter()
            .map(|output| text(&output.stderr))
            .filter(|stderr| !stderr.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
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

    /// Runs `args` with `stdin`, the inherited environment and cwd, no timeout, and stdout
    /// captured.
    async fn run_cmd(args: &[&str], stdin: Option<&str>) -> Result<CommandResult, Error> {
        run(&[argv(args)], stdin, &BTreeMap::new(), None, None, true).await
    }

    /// Runs `stages` as a pipeline, like [`run_cmd`].
    async fn run_pipe(stages: &[&[&str]], stdin: Option<&str>) -> Result<CommandResult, Error> {
        let stages: Vec<_> = stages.iter().map(|stage| argv(stage)).collect();
        run(&stages, stdin, &BTreeMap::new(), None, None, true).await
    }

    #[tokio::test]
    async fn json_stdout_is_parsed() {
        let result = run_cmd(&["echo", r#"{"a":1}"#], None).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, json!({"a": 1}));
    }

    #[tokio::test]
    async fn text_stdout_loses_trailing_newline() {
        let result = run_cmd(&["echo", "hello world"], None).await.unwrap();
        assert_eq!(result.stdout, json!("hello world"));
    }

    #[tokio::test]
    async fn captures_stderr_and_exit_code() {
        let result = run_cmd(&["sh", "-c", "echo oops >&2; exit 3"], None)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 3);
        assert_eq!(result.stderr, "oops");
        assert_eq!(result.stdout, json!(""));
    }

    #[tokio::test]
    async fn uncaptured_stdout_is_null() {
        let result = run(
            &[argv(&["sh", "-c", "echo out; echo oops >&2; exit 3"])],
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
        let result = run_cmd(&["sh", "-c", "kill -9 $$"], None).await.unwrap();
        assert_eq!(result.exit_code, -1);
    }

    #[tokio::test]
    async fn stdin_is_written_to_the_command() {
        let result = run_cmd(&["cat"], Some(r#"{"n": 2}"#)).await.unwrap();
        assert_eq!(result.stdout, json!({"n": 2}));
        let result = run_cmd(&["wc", "-l"], Some("a\nb\n")).await.unwrap();
        assert_eq!(result.stdout, json!(2));
    }

    #[tokio::test]
    async fn large_stdin_and_stdout_do_not_deadlock() {
        let big = "x".repeat(1 << 20);
        let result = run_cmd(&["cat"], Some(&big)).await.unwrap();
        assert_eq!(result.stdout, json!(big));
    }

    /// Whether process `pid` is gone, waiting up to 2s for it to be killed. Waits on the
    /// runtime, so the run's tasks can reap a killed stage. A zombie counts as gone: in a
    /// container without an init process, nothing may reap it.
    async fn gone(pid: i32) -> bool {
        for _ in 0..40 {
            // The state follows the `)` closing the command name.
            let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                stat.rsplit_once(')')
                    .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
            });
            // Signal 0 only checks that the process exists.
            let alive =
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() && !zombie;
            if !alive {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    async fn timeout_fails_and_kills_the_commands_children() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
        let started = std::time::Instant::now();
        let err = run(
            &[argv(&["sh", "-c", &script])],
            None,
            &BTreeMap::new(),
            None,
            Some(1),
            true,
        )
        .await
        .unwrap_err();
        assert!(started.elapsed().as_secs() < 3);
        assert!(
            matches!(err, Error::CommandTimeout { ref command, secs: 1 } if command == "sh"),
            "{err}"
        );
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(gone(pid).await, "`sleep` ({pid}) still runs");
    }

    #[tokio::test]
    async fn command_that_ignores_stdin_still_succeeds() {
        let big = "x".repeat(1 << 20);
        let result = run_cmd(&["true"], Some(&big)).await.unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn without_stdin_the_command_reads_nothing() {
        let result = run_cmd(&["cat"], None).await.unwrap();
        assert_eq!(result.stdout, json!(""));
    }

    #[tokio::test]
    async fn missing_cwd_is_named() {
        let err = run(
            &[argv(&["echo"])],
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
        let err = run_cmd(&["rocky-no-such-program"], None).await.unwrap_err();
        assert!(matches!(err, Error::Spawn { command, .. } if command == "rocky-no-such-program"));
    }

    #[tokio::test]
    async fn runs_in_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let result = run(
            &[argv(&["pwd"])],
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
            &[argv(&["sh", "-c", r#"printf "%s" "$A""#])],
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
            run_cmd(&[], None).await.unwrap_err(),
            Error::InvalidTask(_)
        ));
    }

    #[tokio::test]
    async fn pipe_feeds_each_stage_into_the_next() {
        let result = run_pipe(&[&["printf", "b\\na\\nb\\n"], &["sort", "-u"]], None)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, json!("a\nb"));
        let result = run_pipe(&[&["cat"], &["wc", "-l"]], Some("a\nb\n"))
            .await
            .unwrap();
        assert_eq!(result.stdout, json!(2));
    }

    #[tokio::test]
    async fn pipe_exit_code_is_the_last_non_zero_one() {
        let result = run_pipe(&[&["sh", "-c", "exit 3"], &["cat"]], None)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 3);
        let result = run_pipe(
            &[&["sh", "-c", "exit 3"], &["sh", "-c", "exit 4"], &["true"]],
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 4);
    }

    #[tokio::test]
    async fn pipe_joins_stderr_in_stage_order() {
        let result = run_pipe(
            &[
                &["sh", "-c", "echo one >&2"],
                &["sh", "-c", "cat"],
                &["sh", "-c", "sleep 0.1; echo three >&2"],
            ],
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.stderr, "one\nthree");
    }

    #[tokio::test]
    async fn large_data_through_a_pipe_does_not_deadlock() {
        let big = "x".repeat(1 << 20);
        let result = run_pipe(&[&["cat"], &["cat"], &["cat"]], Some(&big))
            .await
            .unwrap();
        assert_eq!(result.stdout, json!(big));
    }

    #[tokio::test]
    async fn missing_program_in_a_later_stage_is_named() {
        let err = run_pipe(&[&["sleep", "30"], &["rocky-no-such-program"]], None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Spawn { command, .. } if command == "rocky-no-such-program"));
    }

    #[tokio::test]
    async fn timeout_kills_every_stage() {
        let dir = tempfile::tempdir().unwrap();
        let stage = |name: &str| {
            let file = dir.path().join(name);
            format!("echo $$ > {}; sleep 30", file.display())
        };
        let (first, second) = (stage("first"), stage("second"));
        let stages = [argv(&["sh", "-c", &first]), argv(&["sh", "-c", &second])];
        let err = run(&stages, None, &BTreeMap::new(), None, Some(1), true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::CommandTimeout { ref command, secs: 1 } if command == "sh"),
            "{err}"
        );
        for name in ["first", "second"] {
            let pid: i32 = std::fs::read_to_string(dir.path().join(name))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert!(gone(pid).await, "`{name}` stage ({pid}) still runs");
        }
    }
}
