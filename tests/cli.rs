use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Runs `request`, the fields of an `http` step, as a one-step job `w` in `dir`.
fn exec(dir: &Path, request: &str) -> Output {
    let indented: String = request
        .lines()
        .map(|line| format!("          {line}\n"))
        .collect();
    let file = write(
        dir,
        "project.yaml",
        &format!("jobs:\n  w:\n    steps:\n      request:\n        http:\n{indented}"),
    );
    exec_project(&file, &[], dir)
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
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
    Command::new(env!("CARGO_BIN_EXE_rocky"))
        .arg("run")
        .arg(project)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
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

/// Reads `child`'s stdout JSON lines until `enough` holds for the lines so far, then kills
/// it; kills it and panics if that takes longer than `DEADLINE`.
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
    child.kill().unwrap();
    child.wait().unwrap();
    lines
}

/// Waits for the pid a cmd step wrote to `pid_file`; kills `child` and panics if it
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

fn assert_gone(pid: i32) {
    let gone = (0..40).any(|_| {
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if alive {
            std::thread::sleep(Duration::from_millis(50));
        }
        !alive
    });
    assert!(gone, "`sleep` ({pid}) still runs");
}

#[tokio::test]
async fn get_prints_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users"))
        .and(query_param("page", "2"))
        .and(header("authorization", "Bearer abc"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-request-id", "r1")
                .set_body_json(json!({"users": [1]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let request = format!(
        "method: GET\nurl: {}/users\nquery:\n  page: \"2\"\nheaders:\n  Authorization: Bearer abc\n",
        server.uri()
    );

    let out = stdout_json(&exec(dir.path(), &request));
    assert_eq!(out["status"], 200);
    assert_eq!(out["headers"]["x-request-id"], "r1");
    assert_eq!(out["body"], json!({"users": [1]}));
}

#[tokio::test]
async fn method_keys_send_requests() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 7})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/notify"))
        .and(body_json(json!({"id": 7})))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let book = write(
        dir.path(),
        "project.yaml",
        &format!(
            "vars: {{BASE: \"{}\"}}\njobs:\n  w:\n    steps:\n\
             \x20     user:\n        get: ${{BASE}}/users\n\
             \x20     notify:\n        post: {{url: \"${{BASE}}/notify\", json: {{id: \"${{steps.user.body.id}}\"}}}}\n",
            server.uri()
        ),
    );

    let out = stdout_json(&exec_project(&book, &[], dir.path()));
    assert_eq!(out["status"], 201);
}

#[tokio::test]
async fn step_vars_and_project_env_file_from_other_cwd() {
    let server = MockServer::start().await;
    Mock::given(path("/api"))
        .and(header("x-token", "from_file"))
        .and(header("x-shared", "from_vars"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".env", "TOKEN=from_file\nSHARED=from_file\n");
    let book = write(
        dir.path(),
        "project.yaml",
        &format!(
            "env_file: .env\njobs:\n  w:\n    steps:\n      request:\n        http: {{method: GET, url: \"${{BASE}}/api\", headers: {{X-Token: \"${{TOKEN}}\", X-Shared: \"${{SHARED}}\"}}}}\n        vars: {{BASE: \"{}\", SHARED: from_vars}}\n",
            server.uri()
        ),
    );
    let other_cwd = tempfile::tempdir().unwrap();

    let out = stdout_json(&exec_project(&book, &[], other_cwd.path()));
    assert_eq!(out["status"], 200);
}

#[tokio::test]
async fn undefined_var_fails_before_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let request = format!(
        "method: GET\nurl: {}/${{ROCKY_TEST_UNDEFINED}}\n",
        server.uri()
    );

    let out = exec(dir.path(), &request);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"");
    assert!(stderr(&out).contains("variable `ROCKY_TEST_UNDEFINED` is not defined"));
}

#[test]
fn missing_env_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "env_file: missing.env\njobs:\n  w:\n    steps:\n      e:\n        cmd: [echo]\n",
    );
    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("failed to load env file"), "{err}");
    assert_eq!(err.matches("No such file or directory").count(), 1, "{err}");
}

#[test]
fn request_env_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let out = exec(
        dir.path(),
        "env_file: .env\nmethod: GET\nurl: http://localhost\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("unknown field `env_file`"));
}

#[tokio::test]
async fn step_timeout_limits_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(3)))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "jobs:\n  w:\n    steps:\n      slow:\n        http: {{method: GET, url: \"{}\"}}\n        timeout: 1\n",
            server.uri()
        ),
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"");
    assert!(
        stderr(&out).contains("request timed out after 1s"),
        "{}",
        stderr(&out)
    );
}

fn exec_project(file: &Path, extra: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rocky"))
        .arg("exec")
        .arg(file)
        .arg("w")
        .args(extra)
        .current_dir(cwd)
        .output()
        .unwrap()
}

#[tokio::test]
async fn job_passes_results_to_next_step() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"token": "abc", "user": {"id": 7}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/7"))
        .and(header("authorization", "Bearer abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "alice"})))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "vars:\n  BASE: {}\njobs:\n  w:\n    steps:\n\
             \x20     login:\n        http:\n          method: POST\n          url: ${{BASE}}/login\n\
             \x20     profile:\n        http:\n          method: GET\n          url: ${{BASE}}/users/${{steps.login.body.user.id}}\n          headers:\n            Authorization: Bearer ${{steps.login.body.token}}\n",
            server.uri()
        ),
    );

    let out = stdout_json(&exec_project(&file, &[], dir.path()));
    assert_eq!(out["status"], 200);
    assert_eq!(out["body"], json!({"name": "alice"}));
}

#[tokio::test]
async fn job_injects_whole_body_and_status_with_types() {
    let server = MockServer::start().await;
    Mock::given(path("/profile"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "alice", "tags": ["a"]})),
        )
        .mount(&server)
        .await;
    Mock::given(path("/audit"))
        .and(body_json(json!({
            "profile": {"name": "alice", "tags": ["a"]},
            "prev_status": 200,
        })))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "vars:\n  BASE: {}\njobs:\n  w:\n    steps:\n\
             \x20     profile:\n        http:\n          method: GET\n          url: ${{BASE}}/profile\n\
             \x20     audit:\n        http:\n          method: POST\n          url: ${{BASE}}/audit\n          json:\n            profile: ${{steps.profile.body}}\n            prev_status: ${{steps.profile.status}}\n",
            server.uri()
        ),
    );

    let out = stdout_json(&exec_project(&file, &[], dir.path()));
    assert_eq!(out["status"], 201);
}

#[tokio::test]
async fn job_allow_failure_continues() {
    let server = MockServer::start().await;
    Mock::given(path("/flaky"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/next"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "jobs:\n  w:\n    steps:\n\
             \x20     flaky:\n        allow_failure: true\n        http:\n          method: GET\n          url: {0}/flaky\n\
             \x20     next:\n        http:\n          method: GET\n          url: {0}/next\n",
            server.uri()
        ),
    );

    let out = stdout_json(&exec_project(&file, &[], dir.path()));
    assert_eq!(out["status"], 200);
}

#[tokio::test]
async fn job_branch_picks_then_or_else_then_continues() {
    for (count, big_runs) in [(15, true), (5, false)] {
        let server = MockServer::start().await;
        Mock::given(path("/stats"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": count})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200))
            .expect(u64::from(big_runs))
            .mount(&server)
            .await;
        Mock::given(path("/small"))
            .respond_with(ResponseTemplate::new(200))
            .expect(u64::from(!big_runs))
            .mount(&server)
            .await;
        Mock::given(path("/done"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"done": true})))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "project.yaml",
            &format!(
                "vars:\n  BASE: {}\njobs:\n  w:\n    steps:\n\
                 \x20     A:\n        http: {{method: GET, url: \"${{BASE}}/stats\"}}\n\
                 \x20     route:\n        if: {{this: steps.A.body.count, gt: 10}}\n        then:\n          big:\n            http: {{method: POST, url: \"${{BASE}}/big\"}}\n        else:\n          small:\n            http: {{method: POST, url: \"${{BASE}}/small\"}}\n\
                 \x20     D:\n        http: {{method: GET, url: \"${{BASE}}/done\"}}\n",
                server.uri()
            ),
        );

        let out = stdout_json(&exec_project(&file, &[], dir.path()));
        assert_eq!(out["body"], json!({"done": true}), "count {count}");
    }
}

#[tokio::test]
async fn job_for_each_posts_each_item() {
    let server = MockServer::start().await;
    Mock::given(path("/users"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"items": [{"id": 1}, {"id": 2}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    for id in [1, 2] {
        Mock::given(method("POST"))
            .and(path(format!("/notify/{id}")))
            .and(body_json(json!({"id": id})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sent": id})))
            .expect(1)
            .mount(&server)
            .await;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "vars:\n  BASE: {}\njobs:\n  w:\n    steps:\n\
             \x20     users:\n        http: {{method: GET, url: \"${{BASE}}/users\"}}\n\
             \x20     each:\n        for_each: steps.users.body.items\n        as: USER\n        do:\n\
             \x20         notify:\n            http:\n              method: POST\n              url: ${{BASE}}/notify/${{USER.id}}\n              json: {{id: \"${{USER.id}}\"}}\n",
            server.uri()
        ),
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(stdout_json(&out)["body"], json!({"sent": 2}));
}

#[tokio::test]
async fn job_invalid_condition_fails_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "jobs:\n  w:\n    steps:\n\
             \x20     get:\n        http: {{method: GET, url: \"{}\"}}\n\
             \x20     route:\n        if: {{this: steps.get.status, gt: ten}}\n        then:\n          again:\n            http: {{method: GET, url: \"{}\"}}\n",
            server.uri(),
            server.uri()
        ),
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("invalid step: `route`: invalid condition: `gt` needs a number"),
        "{err}"
    );
    assert!(!err.contains("failed"), "{err}");
}

#[test]
fn job_invalid_regex_fails_at_load() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  w:\n    steps:\n\
         \x20     route:\n        if: {this: X, matches: '('}\n        then:\n          touch:\n            cmd: [touch, ran]\n",
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("invalid regex `(`"), "{err}");
    assert!(!dir.path().join("ran").exists());
}

#[tokio::test]
async fn job_condition_type_error_reports_step() {
    let server = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"v": "abc"})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "jobs:\n  w:\n    steps:\n\
             \x20     get:\n        http: {{method: GET, url: \"{}/a\"}}\n\
             \x20     route:\n        if: {{this: steps.get.body.v, gt: 1}}\n        then:\n          again:\n            http: {{method: GET, url: \"{}/a\"}}\n",
            server.uri(),
            server.uri()
        ),
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"");
    let err = stderr(&out);
    assert!(err.contains("step `route` failed"), "{err}");
    assert!(err.contains("`steps.get.body.v` is not a number"), "{err}");
}

#[tokio::test]
async fn job_prints_failing_step_result() {
    let server = MockServer::start().await;
    Mock::given(path("/stats"))
        .respond_with(ResponseTemplate::new(404).set_body_string("gone"))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "jobs:\n  w:\n    steps:\n      stats:\n        http: {{method: GET, url: \"{}/stats\"}}\n",
            server.uri()
        ),
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(envelope["status"], 404);
    assert_eq!(envelope["body"], "gone");
    let err = stderr(&out);
    assert!(err.contains("step `stats` returned HTTP 404"), "{err}");
    assert!(!err.contains("step `stats`:"), "{err}");
}

#[test]
fn cmd_output_branch_and_vars() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "env_file: .env\n\
         vars: {USER_NAME: alice}\n\
         jobs:\n  w:\n    steps:\n\
         \x20     count:\n        cmd: [echo, '{\"n\": 5}']\n\
         \x20     size:\n        if: {this: steps.count.stdout.n, gt: 3}\n        then: {big: {cmd: [echo, '${GREETING} ${USER_NAME} n=${steps.count.stdout.n}']}}\n        else: {small: {cmd: [echo, small]}}\n",
    );
    write(dir.path(), ".env", "GREETING=hello\n");

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(
        stdout_json(&out),
        json!({"exit_code": 0, "stdout": "hello alice n=5", "stderr": ""})
    );
}

#[test]
fn set_vars_applies_to_later_steps_and_writes_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "vars: {N: '0'}\n\
         jobs:\n  w:\n    steps:\n\
         \x20     keep:\n        set: {vars: {N: '${N}+1'}}\n\
         \x20     print:\n        cmd: [echo, 'n=${N}']\n",
    );
    let out = exec_project(&file, &[], dir.path());
    assert_eq!(stdout_json(&out)["stdout"], json!("n=0+1"));
    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(files, ["project.yaml"]);
}

#[test]
fn cmd_stops_on_non_zero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  w:\n    steps:\n      boom:\n        cmd: [sh, -c, 'echo bad >&2; exit 3']\n      never:\n        cmd: [echo, never]\n",
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    let printed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(printed["exit_code"], 3);
    assert_eq!(printed["stderr"], "bad");
    assert!(stderr(&out).contains("step `boom` exited with code 3"));
}

#[test]
fn cmd_missing_program_fails_with_step_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  w:\n    steps:\n      nope:\n        cmd: [rocky-no-such-program]\n",
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    let log = stderr(&out);
    assert!(log.contains("step `nope` failed"), "{log}");
    assert!(
        log.contains("failed to start command `rocky-no-such-program`"),
        "{log}"
    );
}

#[tokio::test]
async fn job_steps_read_earlier_results_via_args_and_stdin() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-token", "t1")
                .set_body_json(json!({"users": ["a", "b", "c"]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/report"))
        .and(header("authorization", "Bearer t1"))
        .and(body_json(
            json!({"users": {"users": ["a", "b", "c"]}, "bytes": 13}),
        ))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        &format!(
            "vars:\n  BASE: {}\n\
             jobs:\n  w:\n    steps:\n\
             \x20     fetch:\n        http: {{method: GET, url: \"${{BASE}}/users\"}}\n\
             \x20     count:\n        cmd: [wc, -c]\n        stdin: ${{steps.fetch.body.users}}\n\
             \x20     report:\n        http:\n          method: POST\n          url: ${{BASE}}/report\n          headers:\n            Authorization: Bearer ${{steps.fetch.headers.x-token}}\n          json:\n            users: ${{steps.fetch.body}}\n            bytes: ${{steps.count.stdout}}\n\
             \x20     check:\n        if: {{this: steps.report.status, eq: 201}}\n        then:\n          print:\n            cmd: [echo, 'reported ${{steps.count.stdout}}']\n",
            server.uri()
        ),
    );

    let out = exec_project(&file, &[], dir.path());
    assert_eq!(stdout_json(&out)["stdout"], "reported 13");
}

#[test]
fn run_runs_at_job_once_and_exits() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "vars: {{WHO: hi}}\n\
             schedules:\n  hello:\n    at: \"{at}\"\n\
             jobs:\n\
             \x20 hello:\n    steps:\n      greet:\n        cmd: [echo, '${{WHO}}']\n\
             \x20 manual:\n    steps:\n      never:\n        cmd: [echo, never]\n"
        ),
    );

    let output = run(&project, dir.path());
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    let lines = stdout_lines(&output);
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in &lines {
        assert_eq!(line["job"], "hello", "{line}");
        assert!(line["time"].is_string(), "{line}");
    }
    assert_eq!(lines[0]["event"], "step");
    assert_eq!(lines[0]["step"], "greet");
    assert_eq!(lines[0]["exit_code"], 0);
    assert_eq!(lines[0]["ok"], true);
    assert_eq!(lines[1]["event"], "done");
    assert_eq!(lines[1].get("result"), None);
    assert_eq!(lines[1]["failed_step"], Value::Null);
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
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "schedules:\n  hello:\n    at: \"{at}\"\n\
             jobs:\n  hello:\n    steps:\n      greet:\n        cmd: [echo, hi]\n"
        ),
    );

    let output = run(&project, dir.path());
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("cannot write log file"),
        "{}",
        stderr(&output)
    );
    let events: Vec<_> = stdout_lines(&output)
        .iter()
        .map(|line| line["event"].clone())
        .collect();
    assert_eq!(events, ["step", "done"]);
}

#[test]
fn stop_signals_stop_job_and_kill_running_commands() {
    // SIGHUP comes from closing the terminal, which no longer reaches the commands' groups.
    for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "project.yaml",
            "jobs:\n  w:\n    steps:\n      wait:\n        cmd: [sh, -c, 'sleep 30 & echo $! > pid; wait']\n        cwd: .\n",
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_rocky"))
            .args(["exec".as_ref(), file.as_os_str(), "w".as_ref()])
            .current_dir(dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = wait_for_pid(&mut child, &dir.path().join("pid"));
        let rocky = i32::try_from(child.id()).unwrap();
        assert_eq!(unsafe { libc::kill(rocky, signal) }, 0);
        let out = wait_with_deadline(child);
        assert_eq!(out.status.code(), Some(128 + signal), "{}", stderr(&out));
        assert_gone(pid);
    }
}

#[test]
fn sigint_stops_run_and_kills_running_commands() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "schedules:\n  w:\n    at: \"{at}\"\n\
             jobs:\n  w:\n    steps:\n      wait:\n        cmd: [sh, -c, 'sleep 30 & echo $! > pid; wait']\n        cwd: .\n"
        ),
    );
    let mut child = spawn_run(&project, dir.path());
    let pid = wait_for_pid(&mut child, &dir.path().join("pid"));
    let rocky = i32::try_from(child.id()).unwrap();
    assert_eq!(unsafe { libc::kill(rocky, libc::SIGINT) }, 0);
    let out = wait_with_deadline(child);
    assert_eq!(out.status.code(), Some(130), "{}", stderr(&out));
    assert_gone(pid);
}

#[test]
fn run_keeps_logging_when_stdout_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "schedules:\n  hello:\n    at: \"{at}\"\n\
             jobs:\n  hello:\n    steps:\n      greet:\n        cmd: [echo, hi]\n"
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
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "schedules:\n  slow:\n    at: \"{at}\"\n    run_on_start: true\n\
             jobs:\n  slow:\n    steps:\n      wait:\n        cmd: [sleep, '1']\n"
        ),
    );

    let output = run(&project, dir.path());
    let done = stdout_lines(&output)
        .iter()
        .filter(|line| line["event"] == "done")
        .count();
    assert_eq!(done, 2, "{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn run_run_on_start_runs_before_first_slot_and_logs_control_flow() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "schedules:\n  hello:\n    at: \"{at}\"\n    run_on_start: true\n\
             jobs:\n\
             \x20 hello:\n    states: {{N: 0}}\n    steps:\n\
             \x20     save:\n        set: {{states: {{N: 1}}}}\n\
             \x20     check:\n        if: {{this: states.N, eq: 1}}\n        then: {{hi: {{cmd: [echo, hi]}}}}\n\
             \x20     route:\n        switch: states.N\n        cases: {{1: {{one: {{cmd: [echo, one]}}}}}}\n\
             \x20     list:\n        cmd: [echo, '[1]']\n\
             \x20     each:\n        for_each: steps.list.stdout\n        do: {{print: {{cmd: [echo, '${{ITEM}}']}}}}\n"
        ),
    );

    let output = run(&project, dir.path());
    let events: Vec<_> = stdout_lines(&output)
        .iter()
        .map(|line| line["event"].clone())
        .collect();
    let run = [
        "set", "branch", "step", "switch", "step", "step", "for_each", "step", "done",
    ];
    assert_eq!(events, [run, run].concat());
}

#[test]
fn run_rejects_a_project_without_schedules() {
    let dir = tempfile::tempdir().unwrap();
    let project = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  a:\n    steps: {e: {cmd: [echo]}}\n",
    );
    let output = run(&project, dir.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("project has no `schedules`"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn network_error_cause_is_printed_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "p.yaml",
        "jobs:\n  w:\n    steps:\n      down:\n        http: {method: GET, url: \"http://127.0.0.1:1/\"}\n",
    );
    let out = exec_project(&file, &[], dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"");
    let log = stderr(&out);
    assert!(log.contains("request failed"), "{log}");
    assert_eq!(log.matches("error sending request").count(), 1, "{log}");
}

#[test]
fn allow_failure_continues_after_network_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "p.yaml",
        "jobs:\n  w:\n    steps:\n\
         \x20     down:\n        http: {method: GET, url: \"http://127.0.0.1:1/\"}\n        allow_failure: true\n\
         \x20     print:\n        cmd: [echo, '${steps.down.error}']\n",
    );
    let out = exec_project(&file, &[], dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    let error = stdout_json(&out)["stdout"].as_str().unwrap().to_string();
    assert!(
        error.starts_with("request failed: error sending request"),
        "{error}"
    );
}

#[test]
fn run_reports_errored_run_on_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let at = at_in(500);
    let project = write(
        dir.path(),
        "project.yaml",
        &format!(
            "schedules:\n  broken:\n    at: \"{at}\"\njobs:\n  broken:\n    steps:\n      e:\n        cmd: [echo, '${{ROCKY_TEST_UNDEFINED}}']\n"
        ),
    );

    let output = run(&project, dir.path());
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
fn run_every_jobs_run_independently_and_survive_errors() {
    let dir = tempfile::tempdir().unwrap();
    let project = write(
        dir.path(),
        "project.yaml",
        "schedules: {ok: {every: 1s}, broken: {every: 1s}}\n\
         jobs:\n\
         \x20 ok:\n    steps: {ok: {cmd: [echo, ok]}}\n\
         \x20 broken:\n    steps: {e: {cmd: [echo, '${ROCKY_TEST_UNDEFINED}']}}\n",
    );

    let runs = |lines: &[Value], job: &str, event: &str| {
        lines
            .iter()
            .filter(|line| line["job"] == job && line["event"] == event)
            .count()
    };
    let lines = lines_until(spawn_run(&project, dir.path()), |lines| {
        runs(lines, "ok", "done") >= 2 && runs(lines, "broken", "error") >= 2
    });
    for line in &lines {
        match line["event"].as_str().unwrap() {
            "done" => assert_eq!(line["failed_step"], Value::Null, "{line}"),
            "error" => assert!(
                line["error"].as_str().unwrap().contains("is not defined"),
                "{line}"
            ),
            "step" => assert_eq!(line["job"], "ok", "{line}"),
            other => panic!("unexpected event {other}: {line}"),
        }
    }
}

#[test]
fn run_failed_step_is_logged_and_next_attempts_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let project = write(
        dir.path(),
        "project.yaml",
        "schedules: {ok: {every: 1s}, failing: {every: 1s}}\n\
         jobs:\n\
         \x20 ok:\n    steps: {ok: {cmd: [echo, ok]}}\n\
         \x20 failing:\n    steps: {check: {cmd: [sh, -c, 'exit 3']}, never: {cmd: [echo, never]}}\n",
    );

    let events = |lines: &[Value], job: &str, event: &str| {
        lines
            .iter()
            .filter(|line| line["job"] == job && line["event"] == event)
            .cloned()
            .collect::<Vec<_>>()
    };
    // Stops between runs of `failing`, so each of its `step` lines has its `done` line.
    let lines = lines_until(spawn_run(&project, dir.path()), |lines| {
        let failing = events(lines, "failing", "done").len();
        failing >= 2
            && events(lines, "ok", "done").len() >= 2
            && events(lines, "failing", "step").len() == failing
    });
    let failing = events(&lines, "failing", "done");
    for line in &failing {
        assert_eq!(line["failed_step"], "check", "{line}");
        assert_eq!(line["reason"], "exited with code 3", "{line}");
    }
    for line in events(&lines, "ok", "done") {
        assert_eq!(line["failed_step"], Value::Null, "{line}");
        assert_eq!(line["reason"], Value::Null, "{line}");
    }
    let steps = events(&lines, "failing", "step");
    assert_eq!(steps.len(), failing.len(), "{lines:?}");
    for line in &steps {
        assert_eq!(line["step"], "check", "{line}");
        assert_eq!(line["ok"], false, "{line}");
    }
}

#[test]
fn unknown_job_lists_available() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  a: {steps: {e: {cmd: [echo]}}}\n  b: {steps: {e: {cmd: [echo]}}}\n",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_rocky"))
        .args(["exec", file.to_str().unwrap(), "c"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"");
    assert!(
        stderr(&out).contains("no job `c` (available: `a`, `b`)"),
        "{}",
        stderr(&out)
    );
}

fn new(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rocky"))
        .arg("new")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

#[test]
fn new_project_runs() {
    let dir = tempfile::tempdir().unwrap();
    let out = new(&["project", "project.yaml"], dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("rocky exec project.yaml example"));
    let out = Command::new(env!("CARGO_BIN_EXE_rocky"))
        .args(["exec", "project.yaml", "example"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(stdout_json(&out)["stdout"], "hello");
}

#[test]
fn new_full_level_writes_a_project_that_validates() {
    let dir = tempfile::tempdir().unwrap();
    let out = new(&["project", "project.yaml", "--level", "full"], dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    let out = Command::new(env!("CARGO_BIN_EXE_rocky"))
        .args(["validate", "project.yaml"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn new_creates_dirs_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let out = new(&["job", "sub/dir/deploy.yaml"], dir.path());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("deploy: sub/dir/deploy.yaml"));

    assert!(new(&["step", "steps/a.yaml"], dir.path()).status.success());
    let before = std::fs::read_to_string(dir.path().join("steps/a.yaml")).unwrap();
    let out = new(&["step", "steps/a.yaml"], dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("steps/a.yaml already exists"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("steps/a.yaml")).unwrap(),
        before
    );
}

#[test]
fn new_step_asks_for_its_type() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rocky"))
        .args(["new", "step", "steps/login.yaml"])
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"http\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("[cmd/http]"), "{}", stderr(&out));
    let text = std::fs::read_to_string(dir.path().join("steps/login.yaml")).unwrap();
    assert!(
        !text.contains("\nname:") && text.contains("\nget:"),
        "{text}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("login: steps/login.yaml"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn validate_accepts_a_valid_project_without_running_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "jobs:\n  b:\n    steps:\n      t:\n        cmd: [touch, ran]\n  a:\n    steps:\n      g:\n        http: {method: GET, url: 'http://127.0.0.1:1'}\n",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_rocky"))
        .arg("validate")
        .arg(&file)
        .current_dir(dir.path())
        .output()
        .unwrap();
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
        "jobs:\n  w:\n    steps:\n      t:\n        if: {this: X, matches: '('}\n        then: {touch: {cmd: [touch, ran]}}\n",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_rocky"))
        .arg("validate")
        .arg(&file)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(
        stderr(&out).contains("invalid regex `(`"),
        "{}",
        stderr(&out)
    );
    assert!(!dir.path().join("ran").exists());
}

#[test]
fn validate_rejects_an_at_schedule_in_the_past() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(
        dir.path(),
        "project.yaml",
        "schedules:\n  w: {at: '2000-01-01T00:00:00'}\njobs:\n  w:\n    steps:\n      t:\n        cmd: [touch, ran]\n",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_rocky"))
        .arg("validate")
        .arg(&file)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("`w`: `at` is in the past"),
        "{}",
        stderr(&out)
    );
    assert!(!dir.path().join("ran").exists());
}
