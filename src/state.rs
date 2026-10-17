use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};

use crate::error::Error;
use crate::step::Step;

/// Reads a job's state file: each name in `defaults` takes its stored value, or its
/// default if the file doesn't have it; stored names no longer in `defaults` are dropped.
/// A missing file is created from the defaults.
pub fn load(path: &Path, defaults: &BTreeMap<String, Value>) -> Result<Map<String, Value>, Error> {
    let mut stored = match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str::<Map<String, Value>>(&text).map_err(|source| Error::State {
                path: path.to_path_buf(),
                source,
            })?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let state: Map<String, Value> = defaults.clone().into_iter().collect();
            save(path, &state)?;
            return Ok(state);
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    Ok(defaults
        .iter()
        .map(|(name, default)| {
            let value = stored.remove(name).unwrap_or_else(|| default.clone());
            (name.clone(), value)
        })
        .collect())
}

/// Writes `state` to `path` as JSON, creating missing parent directories. The JSON goes to a
/// temporary file next to `path` that is then renamed over it, so `path` always holds either
/// the old or the new state, never a partly written one.
pub fn save(path: &Path, state: &Map<String, Value>) -> Result<(), Error> {
    let write_err = |source| Error::Write {
        path: path.to_path_buf(),
        source,
    };
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(""))).map_err(write_err)?;
    let text = serde_json::to_string_pretty(state).map_err(|source| Error::State {
        path: path.to_path_buf(),
        source,
    })?;
    let temp = temp_path(path);
    let result = write_synced(&temp, format!("{text}\n").as_bytes())
        .and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        // Best effort: the write error is the one worth reporting.
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(write_err)
}

/// A temporary file name next to `path`, unique across processes and across saves within one.
fn temp_path(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

/// Writes `bytes` to a new file at `path` and flushes it to disk.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Checks that every `set.states` in `steps`, nested ones included, only sets names
/// declared in `defaults`.
pub fn check(steps: &[Step], defaults: &BTreeMap<String, Value>) -> Result<(), Error> {
    match undeclared(steps, defaults, |step| {
        step.set.as_ref().map(|set| &set.states)
    }) {
        Some(name) => Err(Error::InvalidStep(format!(
            "`set.states` sets `{name}`, which the job's `states` doesn't declare"
        ))),
        None => Ok(()),
    }
}

/// The first name that `updates` of a step in `steps`, nested ones included, sets
/// without it being declared in `defaults`.
pub(crate) fn undeclared<V>(
    steps: &[Step],
    defaults: &BTreeMap<String, V>,
    updates: fn(&Step) -> Option<&BTreeMap<String, Value>>,
) -> Option<String> {
    for step in steps {
        if let Some(name) =
            updates(step).and_then(|set| set.keys().find(|n| !defaults.contains_key(*n)))
        {
            return Some(name.clone());
        }
        for nested in [&step.then, &step.otherwise, &step.body, &step.default] {
            if let Some(name) = undeclared(nested, defaults, updates) {
                return Some(name);
            }
        }
        for case in &step.cases {
            if let Some(name) = undeclared(&case.body, defaults, updates) {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defaults() -> BTreeMap<String, Value> {
        BTreeMap::from([("A".to_string(), json!(0)), ("B".to_string(), json!("b"))])
    }

    #[test]
    fn missing_file_is_created_from_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("states/w.json");
        let state = load(&path, &defaults()).unwrap();
        assert_eq!(Value::Object(state), json!({"A": 0, "B": "b"}));
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written, json!({"A": 0, "B": "b"}));
    }

    #[test]
    fn stored_values_win_new_names_get_defaults_and_old_names_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.json");
        std::fs::write(&path, r#"{"A": [1], "OLD": true}"#).unwrap();
        let state = load(&path, &defaults()).unwrap();
        assert_eq!(Value::Object(state), json!({"A": [1], "B": "b"}));
    }

    #[test]
    fn invalid_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.json");
        for text in ["{", "[1]"] {
            std::fs::write(&path, text).unwrap();
            let err = load(&path, &defaults()).unwrap_err();
            assert!(matches!(err, Error::State { .. }), "{text}: {err}");
        }
    }

    #[test]
    fn save_replaces_the_file_and_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.json");
        std::fs::write(&path, r#"{"A": 0}"#).unwrap();
        for n in 1..=2 {
            let state = Map::from_iter([("A".to_string(), json!(n))]);
            save(&path, &state).unwrap();
        }
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written, json!({"A": 2}));
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["w.json"]);
    }

    #[test]
    fn failed_save_removes_its_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        // Renaming a file over a non-empty directory fails.
        let path = dir.path().join("w.json");
        std::fs::create_dir_all(path.join("inside")).unwrap();
        let err = save(&path, &Map::new()).unwrap_err();
        assert!(matches!(err, Error::Write { .. }), "{err}");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["w.json"]);
    }

    #[test]
    fn check_finds_undeclared_names_in_nested_steps() {
        for (nested, name) in [
            (
                "b:\n  if: {this: X, eq: 1}\n  then:\n    l:\n      for_each: L\n      do: {c: {set: {states: {C: 1}}}}\n",
                "C",
            ),
            (
                "b:\n  switch: X\n  cases: {1: {c: {set: {states: {B: 1}}}}, 2: {d: {set: {states: {D: 1}}}}}\n",
                "D",
            ),
            (
                "b:\n  switch: X\n  cases: {1: {c: {set: {states: {B: 1}}}}}\n  default: {d: {set: {states: {E: 1}}}}\n",
                "E",
            ),
        ] {
            let steps = crate::step::named(serde_yaml::Deserializer::from_str(&format!(
                "a: {{set: {{states: {{A: 1}}}}}}\n{nested}"
            )))
            .unwrap();
            let err = check(&steps, &defaults()).unwrap_err();
            assert!(err.to_string().contains(&format!("`{name}`")), "{err}");
            check(&steps[..1], &defaults()).unwrap();
        }
    }
}
