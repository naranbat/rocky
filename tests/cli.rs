use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::{Value, json};

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// A `rocky` command with `args`, run in `cwd`.
fn rocky_command(args: &[&str], cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rocky"));
    command.args(args).current_dir(cwd);
    command
}

/// Runs `rocky` with `args` in `cwd` to completion.
fn rocky(args: &[&str], cwd: &Path) -> Output {
    rocky_command(args, cwd).output().unwrap()
}

/// Starts `rocky` with `args` in `cwd`, its stdout and stderr piped.
fn spawn_rocky(args: &[&str], cwd: &Path) -> Child {
    rocky_command(args, cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Runs `exec` on job `w` of `file` in `cwd`.
fn exec_project(file: &Path, cwd: &Path) -> Output {
    rocky(&["exec", path_str(file), "w"], cwd)
}

/// Writes `yaml` as `project.yaml` in `dir` and runs `exec` on its job `w` there.
fn exec_yaml(dir: &Path, yaml: &str) -> Output {
    exec_project(&write(dir, "project.yaml", yaml), dir)
}

/// Writes `yaml` as `project.yaml` in `dir` and runs `run` on it there until it exits.
fn run_yaml(dir: &Path, yaml: &str) -> Output {
    run(&write(dir, "project.yaml", yaml), dir)
}

fn validate(file: &Path, cwd: &Path) -> Output {
    rocky(&["validate", path_str(file)], cwd)
}

fn stdout_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stderr(output), "");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn stdout_lines(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The `event` of each of `run`'s log lines.
fn events(output: &Output) -> Vec<Value> {
    stdout_lines(output)
        .iter()
        .map(|line| line["event"].clone())
        .collect()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_stderr_contains(output: &Output, text: &str) {
    let stderr = stderr(output);
    assert!(stderr.contains(text), "{stderr}");
}

/// How long a test waits for `rocky` before killing it and failing.
const DEADLINE: Duration = Duration::from_secs(10);

/// A local datetime `ms` milliseconds from now, as an `at` schedule.
fn at_in(ms: i64) -> String {
    jiff::Zoned::now()
        .checked_add(jiff::SignedDuration::from_millis(ms))
        .unwrap()
        .strftime("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

fn spawn_run(project: &Path, cwd: &Path) -> Child {
    spawn_rocky(&["run", path_str(project)], cwd)
}

/// Runs `run` on `project` in `cwd` until it exits.
fn run(project: &Path, cwd: &Path) -> Output {
    wait_with_deadline(spawn_run(project, cwd))
}

fn read_all(pipe: Option<impl Read + Send + 'static>) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut bytes).unwrap();
        }
        bytes
    })
}

/// Waits for `child` to exit and collects its output; kills it and panics after `DEADLINE`.
fn wait_with_deadline(mut child: Child) -> Output {
    let stdout = read_all(child.stdout.take());
    let stderr = read_all(child.stderr.take());
    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("rocky did not exit within {DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

/// Reads `child`'s stdout JSON lines until `enough` holds for the lines so far, then stops
/// it with SIGTERM (so a coverage build still writes its profile); kills it and panics if
/// that takes longer than `DEADLINE`.
fn lines_until(mut child: Child, enough: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + DEADLINE;
    let mut lines: Vec<Value> = Vec::new();
    while !enough(&lines) {
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => lines.push(serde_json::from_str(&line).unwrap()),
            Err(_) => {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("not enough lines within {DEADLINE:?}: {lines:?}");
            }
        }
    }
    let rocky = i32::try_from(child.id()).unwrap();
    kill(Pid::from_raw(rocky), Signal::SIGTERM).unwrap();
    child.wait().unwrap();
    lines
}

/// Waits for the pid a cmd task wrote to `pid_file`; kills `child` and panics if it
/// never appears.
fn wait_for_pid(child: &mut Child, pid_file: &Path) -> i32 {
    let pid = (0..100).find_map(|_| {
        let text = std::fs::read_to_string(pid_file).unwrap_or_default();
        let pid = text.trim().parse().ok();
        if pid.is_none() {
            std::thread::sleep(Duration::from_millis(50));
        }
        pid
    });
    let Some(pid) = pid else {
        child.kill().unwrap();
        child.wait().unwrap();
        panic!("{} never appeared", pid_file.display());
    };
    pid
}

/// Asserts process `pid` is gone, waiting up to 2s for it to be killed. A zombie counts
/// as gone: in a container without an init process, nothing may reap it.
fn assert_gone(pid: i32) {
    let gone = (0..40).any(|_| {
        // The state follows the `)` closing the command name.
        let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
        });
        let alive = kill(Pid::from_raw(pid), None).is_ok() && !zombie;
        if alive {
            std::thread::sleep(Duration::from_millis(50));
        }
        !alive
    });
    assert!(gone, "`sleep` ({pid}) still runs");
}

#[test]
fn missing_env_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let out = exec_yaml(
        dir.path(),
        "env_file: missing.env\njobs:\n  w:\n    tasks:\n      e:\n        cmd: [echo]\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("failed to load env file"), "{err}");
    assert_eq!(err.matches("No such file or directory").count(), 1, "{err}");
}

#[test]
fn verbose_exec_prints_substituted_commands_to_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  w:\n    values:\n      NAME: hi there\n    tasks:\n\
         \x20     t:\n        cmd: [echo, '${NAME}']\n        pipe: [[cat]]\n",
    );
    let out = rocky(&["exec", path_str(&file), "w", "--verbose"], dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "+ [t] echo 'hi there' | cat\n");
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["stdout"], json!("hi there"));
}

#[test]
fn invalid_condition_fails_at_load_before_any_task() {
    let dir = tempfile::tempdir().unwrap();
    let out = exec_yaml(
        dir.path(),
        "jobs:\n  w:\n    tasks:\n\
         \x20     first:\n        cmd: [touch, ran]\n        cwd: .\n\
         \x20     route:\n        if: {this: tasks.first.exit_code, gt: ten}\n        then: {again: {cmd: [echo]}}\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("invalid task: `route`: invalid condition: `gt` needs a number"),
        "{err}"
    );
    assert!(!err.contains("failed"), "{err}");
    assert!(!dir.path().join("ran").exists());
}

#[test]
fn job_prints_failing_task_result() {
    let dir = tempfile::tempdir().unwrap();
    let out = exec_yaml(
        dir.path(),
        "jobs:\n  w:\n    tasks:\n      stats:\n        cmd: [sh, -c, 'echo gone; exit 3']\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["exit_code"], 3);
    assert_eq!(result["stdout"], "gone");
    let err = stderr(&out);
    assert!(err.contains("task `stats` exited with code 3"), "{err}");
    assert!(!err.contains("task `stats`:"), "{err}");
}

/// Run from another directory, so the env file must be found next to the project.
#[test]
fn cmd_output_branch_and_env_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "env_file: .env\n\
         jobs:\n  w:\n    tasks:\n\
         \x20     count:\n        cmd: [echo, '{\"n\": 5}']\n\
         \x20     size:\n        if: {this: tasks.count.stdout.n, gt: 3}\n        then: {big: {cmd: [echo, '${GREETING} ${USER_NAME} n=${tasks.count.stdout.n}']}}\n        else: {small: {cmd: [echo, small]}}\n",
    );
    write(dir.path(), ".env", "GREETING=hello\nUSER_NAME=alice\n");
    let other_cwd = tempfile::tempdir().unwrap();

    let out = exec_project(&file, other_cwd.path());
    assert_eq!(
        stdout_json(&out),
        json!({"ok": true, "exit_code": 0, "stdout": "hello alice n=5", "stderr": ""})
    );
}

#[test]
fn cmd_missing_program_fails_with_task_name() {
    let dir = tempfile::tempdir().unwrap();
    let out = exec_yaml(
        dir.path(),
        "jobs:\n  w:\n    tasks:\n      nope:\n        cmd: [rocky-no-such-program]\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let log = stderr(&out);
    assert!(log.contains("task `nope` failed"), "{log}");
    assert!(
        log.contains("failed to start command `rocky-no-such-program`"),
        "{log}"
    );
}

#[test]
fn job_tasks_read_earlier_results_via_args_and_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let out = exec_yaml(
        dir.path(),
        "jobs:\n  w:\n    tasks:\n\
         \x20     fetch:\n        cmd: [echo, '{\"token\": \"t1\", \"users\": [\"a\", \"b\", \"c\"]}']\n\
         \x20     count:\n        cmd: [wc, -c]\n        stdin: ${tasks.fetch.stdout.users}\n\
         \x20     report:\n        cmd: [cat]\n        stdin:\n          token: Bearer ${tasks.fetch.stdout.token}\n          users: ${tasks.fetch.stdout.users}\n          bytes: ${tasks.count.stdout}\n          prev_exit: ${tasks.fetch.exit_code}\n\
         \x20     check:\n        if: {this: tasks.report.stdout.bytes, eq: 13}\n        then:\n          print:\n            cmd: [echo, 'reported ${tasks.report.stdout}']\n",
    );
    assert_eq!(
        stdout_json(&out)["stdout"],
        r#"reported {"token":"Bearer t1","users":["a","b","c"],"bytes":13,"prev_exit":0}"#
    );
}

#[test]
fn run_runs_at_job_once_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let output = run_yaml(
        dir.path(),
        &format!(
            "jobs:\n\
             \x20 hello:\n    schedule:\n      at: \"{at}\"\n    tasks:\n      greet:\n        cmd: [echo, hi]\n\
             \x20 manual:\n    tasks:\n      never:\n        cmd: [echo, never]\n"
        ),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    let lines = stdout_lines(&output);
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in &lines {
        assert_eq!(line["job"], "hello", "{line}");
        assert!(line["time"].is_string(), "{line}");
    }
    assert_eq!(lines[0]["event"], "task");
    assert_eq!(lines[0]["task"], "greet");
    assert_eq!(lines[0]["exit_code"], 0);
    assert_eq!(lines[0]["ok"], true);
    assert_eq!(lines[1]["event"], "done");
    assert_eq!(lines[1].get("result"), None);
    assert_eq!(lines[1]["failed_task"], Value::Null);
    assert_eq!(lines[1]["reason"], Value::Null);
    let date = &lines[0]["time"].as_str().unwrap()[..10];
    let log = std::fs::read_to_string(dir.path().join("logs").join(format!("{date}.log"))).unwrap();
    assert_eq!(log, String::from_utf8_lossy(&output.stdout));
}

#[test]
fn run_reports_unwritable_log_file_and_keeps_printing() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "logs", "not a directory");
    let at = at_in(500);
    let output = run_yaml(
        dir.path(),
        &format!(
            "jobs:\n  hello:\n    schedule:\n      at: \"{at}\"\n    tasks:\n      greet:\n        cmd: [echo, hi]\n"
        ),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_stderr_contains(&output, "cannot write log file");
    assert_eq!(events(&output), ["task", "done"]);
}

/// SIGINT is tested on `run` and the others on `exec`: both stop the same way.
#[test]
fn stop_signals_stop_and_kill_running_commands() {
    let wait =
        "      wait:\n        cmd: [sh, -c, 'sleep 30 & echo $! > pid; wait']\n        cwd: .\n";
    // SIGHUP comes from closing the terminal, which no longer reaches the commands' groups.
    for signal in [
        Signal::SIGINT,
        Signal::SIGTERM,
        Signal::SIGHUP,
        Signal::SIGQUIT,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut child = if signal == Signal::SIGINT {
            let at = at_in(500);
            let project = write(
                dir.path(),
                "project.yaml",
                &format!("jobs:\n  w:\n    schedule:\n      at: \"{at}\"\n    tasks:\n{wait}"),
            );
            spawn_run(&project, dir.path())
        } else {
            let file = write(
                dir.path(),
                "project.yaml",
                &format!("jobs:\n  w:\n    tasks:\n{wait}"),
            );
            spawn_rocky(&["exec", path_str(&file), "w"], dir.path())
        };
        let pid = wait_for_pid(&mut child, &dir.path().join("pid"));
        let rocky = i32::try_from(child.id()).unwrap();
        kill(Pid::from_raw(rocky), signal).unwrap();
        let out = wait_with_deadline(child);
        assert_eq!(
            out.status.code(),
            Some(128 + signal as i32),
            "{signal}: {}",
            stderr(&out)
        );
        assert_gone(pid);
    }
}

#[test]
fn run_keeps_logging_when_stdout_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "jobs:\n  hello:\n    schedule:\n      at: \"{at}\"\n    tasks:\n      greet:\n        cmd: [echo, hi]\n"
        ),
    );

    let mut child = spawn_run(&project, dir.path());
    drop(child.stdout.take());
    let out = wait_with_deadline(child);
    assert!(out.status.success(), "{}", stderr(&out));
    let logs: Vec<_> = std::fs::read_dir(dir.path().join("logs"))
        .unwrap()
        .collect();
    let log = std::fs::read_to_string(logs[0].as_ref().unwrap().path()).unwrap();
    assert_eq!(log.lines().count(), 2, "{log}");
}

#[test]
fn run_runs_at_late_when_the_start_run_overlaps_it() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let output = run_yaml(
        dir.path(),
        &format!(
            "jobs:\n  slow:\n    schedule:\n      at: \"{at}\"\n      run_on_start: true\n    tasks:\n      wait:\n        cmd: [sleep, '1']\n"
        ),
    );
    let done = events(&output)
        .iter()
        .filter(|event| *event == "done")
        .count();
    assert_eq!(done, 2, "{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn run_on_start_and_the_at_run_log_control_flow() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let output = run_yaml(
        dir.path(),
        &format!(
            "jobs:\n\
             \x20 hello:\n    schedule:\n      at: \"{at}\"\n      run_on_start: true\n    values: {{N: 1}}\n    tasks:\n\
             \x20     check:\n        if: {{this: N, eq: 1}}\n        then: {{hi: {{cmd: [echo, hi]}}}}\n\
             \x20     route:\n        switch: N\n        cases: {{1: {{one: {{cmd: [echo, one]}}}}}}\n\
             \x20     list:\n        cmd: [echo, '[1]']\n\
             \x20     each:\n        for_each: tasks.list.stdout\n        do: {{print: {{cmd: [echo, '${{ITEM}}']}}}}\n"
        ),
    );
    let run = [
        "branch", "task", "switch", "task", "task", "for_each", "task", "done",
    ];
    assert_eq!(events(&output), [run, run].concat());
}

#[test]
fn run_on_start_alone_runs_once_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_yaml(
        dir.path(),
        "jobs:\n  hello:\n    schedule: {run_on_start: true}\n    tasks:\n      greet:\n        cmd: [echo, hi]\n",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(events(&output), ["task", "done"]);
}

#[test]
fn run_rejects_a_project_without_a_schedule() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_yaml(dir.path(), "jobs:\n  a:\n    tasks: {e: {cmd: [echo]}}\n");
    assert_eq!(output.status.code(), Some(1));
    assert_stderr_contains(&output, "no job has a `schedule`");
}

#[test]
fn run_reports_errored_run_on_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let output = run_yaml(
        dir.path(),
        &format!(
            "jobs:\n  broken:\n    schedule:\n      at: \"{at}\"\n    tasks:\n      e:\n        cmd: [echo, '${{ROCKY_TEST_UNDEFINED}}']\n"
        ),
    );
    let out = stdout_json(&output);
    assert_eq!(out["job"], "broken");
    assert_eq!(out["event"], "error");
    let error = out["error"].as_str().unwrap();
    assert!(
        error.contains("variable `ROCKY_TEST_UNDEFINED` is not defined"),
        "{error}"
    );
}

#[test]
fn run_every_jobs_run_independently_and_survive_errors_and_failures() {
    let dir = tempfile::tempdir().unwrap();
    let project = write(
        dir.path(),
        "project.yaml",
        "jobs:\n\
         \x20 ok:\n    schedule: {every: 1s}\n    tasks: {ok: {cmd: [echo, ok]}}\n\
         \x20 broken:\n    schedule: {every: 1s}\n    tasks: {e: {cmd: [echo, '${ROCKY_TEST_UNDEFINED}']}}\n\
         \x20 failing:\n    schedule: {every: 1s}\n    tasks: {check: {cmd: [sh, -c, 'exit 3']}, never: {cmd: [echo, never]}}\n",
    );

    let events = |lines: &[Value], job: &str, event: &str| {
        lines
            .iter()
            .filter(|line| line["job"] == job && line["event"] == event)
            .cloned()
            .collect::<Vec<_>>()
    };
    // Stops between runs of `failing`, so each of its `task` lines has its `done` line.
    let lines = lines_until(spawn_run(&project, dir.path()), |lines| {
        let failing = events(lines, "failing", "done").len();
        failing >= 2
            && events(lines, "ok", "done").len() >= 2
            && events(lines, "broken", "error").len() >= 2
            && events(lines, "failing", "task").len() == failing
    });
    for line in &lines {
        let event = line["event"].as_str().unwrap();
        assert!(
            matches!(event, "task" | "done" | "error"),
            "unexpected event {event}: {line}"
        );
    }
    for line in events(&lines, "ok", "done") {
        assert_eq!(line["failed_task"], Value::Null, "{line}");
        assert_eq!(line["reason"], Value::Null, "{line}");
    }
    for line in events(&lines, "broken", "error") {
        assert!(
            line["error"].as_str().unwrap().contains("is not defined"),
            "{line}"
        );
    }
    let failing = events(&lines, "failing", "done");
    for line in &failing {
        assert_eq!(line["failed_task"], "check", "{line}");
        assert_eq!(line["reason"], "exited with code 3", "{line}");
    }
    let tasks = events(&lines, "failing", "task");
    assert_eq!(tasks.len(), failing.len(), "{lines:?}");
    for line in &tasks {
        assert_eq!(line["task"], "check", "{line}");
        assert_eq!(line["ok"], false, "{line}");
    }
}

#[test]
fn new_project_runs_and_full_level_validates() {
    let dir = tempfile::tempdir().unwrap();
    let out = rocky(&["new", "project", "project.yaml"], dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("rocky exec project.yaml example"));
    let out = rocky(&["exec", "project.yaml", "example"], dir.path());
    assert_eq!(stdout_json(&out)["stdout"], "hello");

    let out = rocky(
        &["new", "project", "full.yaml", "--level", "full"],
        dir.path(),
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let out = validate(Path::new("full.yaml"), dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn new_creates_dirs_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let out = rocky(&["new", "job", "sub/dir/deploy.yaml"], dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("deploy: sub/dir/deploy.yaml"));

    let new_task = || rocky(&["new", "task", "tasks/a.yaml"], dir.path());
    assert!(new_task().status.success());
    let before = std::fs::read_to_string(dir.path().join("tasks/a.yaml")).unwrap();
    let out = new_task();
    assert_eq!(out.status.code(), Some(1));
    assert_stderr_contains(&out, "tasks/a.yaml already exists");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("tasks/a.yaml")).unwrap(),
        before
    );
}

#[test]
fn new_task_writes_a_cmd_task() {
    let dir = tempfile::tempdir().unwrap();
    let out = rocky_command(&["new", "task", "tasks/login.yaml"], dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "");
    let text = std::fs::read_to_string(dir.path().join("tasks/login.yaml")).unwrap();
    assert!(
        !text.contains("\nname:") && text.contains("\ncmd:"),
        "{text}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("login: tasks/login.yaml"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Its `at` time is in the past, which is valid too.
#[test]
fn validate_accepts_a_valid_project_without_running_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  b:\n    schedule: {at: '2000-01-01T00:00:00'}\n    tasks:\n      t:\n        cmd: [touch, ran]\n  a:\n    tasks:\n      g:\n        cmd: [echo]\n",
    );
    let out = validate(&file, dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{} is valid (jobs: `a`, `b`)\n", file.display())
    );
    assert!(!dir.path().join("ran").exists());
}

#[test]
fn validate_reports_the_load_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  w:\n    tasks:\n      t:\n        if: {this: X, matches: '('}\n        then: {touch: {cmd: [touch, ran]}}\n",
    );
    let out = validate(&file, dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_stderr_contains(&out, "invalid regex `(`");
    assert!(!dir.path().join("ran").exists());
}

#[test]
fn run_skips_an_at_schedule_in_the_past_and_runs_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(300);
    let output = run_yaml(
        dir.path(),
        &format!(
            "jobs:\n  old:\n    schedule: {{at: '2000-01-01T00:00:00'}}\n    tasks:\n      t:\n        cmd: [touch, old]\n\
             \x20 new:\n    schedule: {{at: \"{at}\"}}\n    tasks:\n      t:\n        cmd: [touch, new]\n"
        ),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let lines = stdout_lines(&output);
    assert!(
        lines
            .iter()
            .any(|line| line["job"] == "old" && line["event"] == "skipped"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!dir.path().join("old").exists());
    assert!(dir.path().join("new").exists());
}

/// Connects to the socket `run` listens on at `path`, waiting up to `DEADLINE` for it.
fn connect(path: &Path) -> UnixStream {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return stream,
            Err(err) if Instant::now() > deadline => panic!("cannot connect: {err}"),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Sends each request line on `stream` and returns the replies.
fn requests(stream: &UnixStream, lines: &[&str]) -> Vec<Value> {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    lines
        .iter()
        .map(|line| {
            writeln!(&*stream, "{line}").unwrap();
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            serde_json::from_str(&reply).unwrap()
        })
        .collect()
}

#[test]
fn run_socket_triggers_jobs_one_run_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    // Fails if two runs overlap.
    let project = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  slow:\n    tasks:\n      t:\n        \
         cmd: [sh, -c, 'test ! -e busy && touch busy && sleep 0.3 && rm busy']\n        cwd: .\n",
    );
    let socket = dir.path().join("rocky.sock");
    let child = spawn_rocky(
        &["run", path_str(&project), "--socket", path_str(&socket)],
        dir.path(),
    );
    let stream = connect(&socket);
    let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let replies = requests(
        &stream,
        &[
            r#"{"job": "slow"}"#,
            r#"{"job": "slow"}"#,
            r#"{"job": "nope"}"#,
            "not json",
        ],
    );
    assert_eq!(replies[0], json!({"ok": true}));
    assert_eq!(replies[1], json!({"ok": true}));
    assert_eq!(
        replies[2],
        json!({"error": "no job `nope` (available: `slow`)"})
    );
    assert!(
        replies[3]["error"]
            .as_str()
            .unwrap()
            .starts_with("invalid request:"),
        "{}",
        replies[3]
    );
    let lines = lines_until(child, |lines| {
        lines.iter().filter(|line| line["event"] == "done").count() == 2
    });
    let events: Vec<_> = lines.iter().map(|line| line["event"].clone()).collect();
    assert_eq!(
        events,
        ["triggered", "triggered", "task", "done", "task", "done"]
    );
    for line in &lines {
        assert_eq!(line["job"], "slow", "{line}");
        assert_ne!(line["ok"], false, "{line}");
    }
}

#[test]
fn run_socket_replaces_a_stale_socket_but_not_a_live_one() {
    let dir = tempfile::tempdir().unwrap();
    let project = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  a:\n    tasks: {e: {cmd: [echo]}}\n",
    );
    let socket = dir.path().join("rocky.sock");
    let args = ["run", path_str(&project), "--socket", path_str(&socket)];
    let mut first = spawn_rocky(&args, dir.path());
    connect(&socket);

    let second = wait_with_deadline(spawn_rocky(&args, dir.path()));
    assert_eq!(second.status.code(), Some(1));
    assert_stderr_contains(&second, "is in use by another process");

    // A stopped `run` leaves its socket file behind.
    kill(
        Pid::from_raw(i32::try_from(first.id()).unwrap()),
        Signal::SIGTERM,
    )
    .unwrap();
    first.wait().unwrap();
    assert!(socket.exists());
    let third = spawn_rocky(&args, dir.path());
    let replies = requests(&connect(&socket), &[r#"{"job": "a"}"#]);
    assert_eq!(replies, [json!({"ok": true})]);
    lines_until(third, |lines| {
        lines.iter().any(|line| line["event"] == "done")
    });
}

#[test]
fn trigger_queues_a_job_and_reports_errors() {
    let dir = tempfile::tempdir().unwrap();
    let project = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  a:\n    tasks: {e: {cmd: [echo]}}\n",
    );
    let socket = dir.path().join("rocky.sock");
    let trigger = |job: &str| rocky(&["trigger", path_str(&socket), job], dir.path());

    let missing = trigger("a");
    assert_eq!(missing.status.code(), Some(1));
    assert_stderr_contains(&missing, "socket");

    let child = spawn_rocky(
        &["run", path_str(&project), "--socket", path_str(&socket)],
        dir.path(),
    );
    connect(&socket);
    let queued = trigger("a");
    assert!(queued.status.success(), "{}", stderr(&queued));
    assert_eq!(String::from_utf8_lossy(&queued.stdout), "queued `a`\n");

    let unknown = trigger("nope");
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(stderr(&unknown), "Error: no job `nope` (available: `a`)\n");
    assert!(unknown.stdout.is_empty());

    lines_until(child, |lines| {
        lines.iter().any(|line| line["event"] == "done")
    });
}
