//! Which variables a `${…}` can read: the project's and job's values and the env file's
//! names for every task, plus what each earlier task left in `tasks.NAME`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

use crate::complete::{self, Var};
use crate::doc;
use crate::task::Kind;

const VALUES: &str = "VALUES · ENV FILE";

/// The variables every task of `job` can read: project and job values, and the names in the
/// env file (read through `files`, the editor's file cache). Env values are never shown.
pub fn base(root: &Value, job: &str, dir: &Path, files: &mut HashMap<PathBuf, String>) -> Vec<Var> {
    let mut vars = Vec::new();
    let job_values = root
        .get("jobs")
        .and_then(|jobs| jobs.get(job))
        .and_then(|job| job.get("values"));
    for (values, source) in [
        (root.get("values"), "project value"),
        (job_values, "job value"),
    ] {
        for (name, value) in values.and_then(Value::as_mapping).into_iter().flatten() {
            vars.push(Var {
                path: doc::key_text(name),
                detail: format!("{source} {}", doc::flow(value)),
                group: VALUES.to_string(),
            });
        }
    }
    if let Some(file) = root.get("env_file").and_then(Value::as_str) {
        let path = dir.join(file);
        let text = files
            .entry(path.clone())
            .or_insert_with(|| std::fs::read_to_string(&path).unwrap_or_default());
        for name in env_names(text) {
            vars.push(Var {
                path: name,
                detail: ".env".to_string(),
                group: VALUES.to_string(),
            });
        }
    }
    vars
}

/// The names a `.env` file sets: `NAME=…` lines, with an optional `export `.
fn env_names(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.trim_start_matches("export ").split_once('='))
        .map(|(name, _)| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// What task `name` leaves for later tasks: `tasks.NAME.ok`, `exit_code`, `stdout`,
/// `stderr`, and, after a run whose stdout was JSON, the fields inside it with their values.
/// Loops and triggers leave nothing.
pub fn task(name: &str, task: &Value, result: Option<&serde_json::Value>) -> Vec<Var> {
    if !matches!(Kind::of(task), Some(Kind::Cmd)) {
        return Vec::new();
    }
    let group = format!("EARLIER TASK · {name}");
    let shown = |field: &str, otherwise: &str| {
        result
            .and_then(|result| result.get(field))
            .map(|value| match value {
                serde_json::Value::String(text) => {
                    format!("{:?}", text.lines().next().unwrap_or_default())
                }
                other => other.to_string(),
            })
            .unwrap_or_else(|| otherwise.to_string())
    };
    let mut vars: Vec<Var> = [
        ("ok", "true / false"),
        ("exit_code", "number"),
        ("stderr", "text"),
    ]
    .into_iter()
    .map(|(field, otherwise)| Var {
        path: format!("tasks.{name}.{field}"),
        detail: shown(field, otherwise),
        group: group.clone(),
    })
    .collect();
    match result.and_then(|result| result.get("stdout")) {
        Some(stdout) => complete::json_paths(
            &format!("tasks.{name}.stdout"),
            stdout,
            &group,
            3,
            &mut vars,
        ),
        None => vars.push(Var {
            path: format!("tasks.{name}.stdout"),
            detail: "text, or JSON".to_string(),
            group,
        }),
    }
    vars
}

/// A loop's item variable, while its `do` tasks are drawn.
pub fn loop_item(name: &str) -> Var {
    Var {
        path: name.to_string(),
        detail: "loop item".to_string(),
        group: "LOOP".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_values_and_env_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "# secret\nexport API_TOKEN=abc\nUSER = bob\n",
        )
        .unwrap();
        let root: Value = serde_yaml::from_str(
            "values: {BASE: x}\nenv_file: .env\njobs:\n  j: {values: {N: 1}, tasks: {}}\n",
        )
        .unwrap();
        let vars = base(&root, "j", dir.path(), &mut HashMap::new());
        let paths: Vec<&str> = vars.iter().map(|var| var.path.as_str()).collect();
        assert_eq!(paths, ["BASE", "N", "API_TOKEN", "USER"]);
        assert!(!vars.iter().any(|var| var.detail.contains("abc")));
    }

    #[test]
    fn task_fields_use_the_last_run() {
        let cmd: Value = serde_yaml::from_str("{cmd: [echo]}").unwrap();
        let result =
            serde_json::json!({"ok": true, "exit_code": 0, "stdout": {"token": "t"}, "stderr": ""});
        let paths: Vec<String> = task("login", &cmd, Some(&result))
            .into_iter()
            .map(|var| var.path)
            .collect();
        assert!(paths.contains(&"tasks.login.stdout.token".to_string()));
        assert_eq!(task("login", &cmd, None).len(), 4);
        let trigger: Value = serde_yaml::from_str("{trigger: x}").unwrap();
        assert!(task("t", &trigger, None).is_empty());
    }
}
