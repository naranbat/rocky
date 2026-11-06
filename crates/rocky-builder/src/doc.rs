//! The project file being edited: its YAML tree, kept in file order, plus helpers to edit
//! mappings in place without reordering them.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde_yaml::{Mapping, Value};

/// An open project file.
pub struct Doc {
    pub path: PathBuf,
    pub root: Value,
    /// The text last read from or written to disk, to tell whether there are unsaved edits.
    saved: String,
}

impl Doc {
    pub fn open(path: &Path) -> anyhow::Result<Doc> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let root: Value = serde_yaml::from_str(&text)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        let mut doc = Doc {
            path: path.to_path_buf(),
            root,
            saved: String::new(),
        };
        // Compare against our own serialization, so opening a file isn't an unsaved edit.
        doc.saved = doc.text()?;
        Ok(doc)
    }

    /// The document as YAML.
    pub fn text(&self) -> anyhow::Result<String> {
        Ok(serde_yaml::to_string(&self.root)?)
    }

    pub fn is_dirty(&self, text: &str) -> bool {
        text != self.saved
    }

    pub fn save(&mut self, text: String) -> anyhow::Result<()> {
        std::fs::write(&self.path, &text)
            .with_context(|| format!("cannot write {}", self.path.display()))?;
        self.saved = text;
        Ok(())
    }

    /// The directory relative paths in the project resolve against.
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new(""))
    }

    /// The `jobs` mapping, created if missing.
    pub fn jobs_mut(&mut self) -> &mut Mapping {
        child_map(&mut self.root, "jobs")
    }

    pub fn job_names(&self) -> Vec<String> {
        self.root
            .get("jobs")
            .and_then(Value::as_mapping)
            .map(keys)
            .unwrap_or_default()
    }
}

/// `value` as a mapping, replacing it with an empty one if it is anything else (null included).
pub fn as_map(value: &mut Value) -> &mut Mapping {
    match value {
        Value::Mapping(map) => map,
        other => {
            *other = Value::Mapping(Mapping::new());
            as_map(other)
        }
    }
}

/// The mapping under `key` of `value`, created (at the end) if missing.
pub fn child_map<'a>(value: &'a mut Value, key: &str) -> &'a mut Mapping {
    let map = as_map(value);
    as_map(
        map.entry(Value::from(key))
            .or_insert_with(|| Value::Mapping(Mapping::new())),
    )
}

/// The string keys of `map`, in order.
pub fn keys(map: &Mapping) -> Vec<String> {
    map.keys().map(key_text).collect()
}

/// A key as text: strings as is, anything else as YAML.
pub fn key_text(key: &Value) -> String {
    match key {
        Value::String(text) => text.clone(),
        other => scalar_text(other),
    }
}

/// A scalar as its YAML text, without quotes or the trailing newline. Collections render
/// in YAML flow-ish form via serde_yaml.
pub fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => serde_yaml::to_string(other)
            .map(|text| text.trim_end().to_string())
            .unwrap_or_default(),
    }
}

/// Parses typed text as a YAML scalar (`200` → number, `true` → bool), keeping it as a
/// string when it is anything else, so `a: b` typed into a value field stays text.
pub fn parse_scalar(text: &str) -> Value {
    match serde_yaml::from_str::<Value>(text) {
        Ok(value @ (Value::Bool(_) | Value::Number(_))) => value,
        _ => Value::String(text.to_string()),
    }
}

/// The string under `key`, if it is one.
pub fn get_str<'a>(map: &'a Mapping, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

/// Sets `key` to `value`, or removes it (keeping the order of the rest) when `None`.
pub fn set(map: &mut Mapping, key: &str, value: Option<Value>) {
    match value {
        Some(value) => {
            map.insert(Value::from(key), value);
        }
        None => {
            map.shift_remove(key);
        }
    }
}

/// Renames key `old` to `new` in place. Does nothing if `new` is empty or already used.
pub fn rename(map: &mut Mapping, old: &str, new: &str) -> bool {
    if new.is_empty() || old == new || map.contains_key(new) {
        return false;
    }
    let entries = std::mem::take(map);
    *map = entries
        .into_iter()
        .map(|(key, value)| {
            if key.as_str() == Some(old) {
                (Value::from(new), value)
            } else {
                (key, value)
            }
        })
        .collect();
    true
}

/// Moves the entry at `from` to position `to`, shifting the ones between.
pub fn move_entry(map: &mut Mapping, from: usize, to: usize) {
    if from >= map.len() || to >= map.len() {
        return;
    }
    let mut entries: Vec<(Value, Value)> = std::mem::take(map).into_iter().collect();
    let entry = entries.remove(from);
    entries.insert(to, entry);
    *map = entries.into_iter().collect();
}

/// A key for a new entry: `base`, or `base_2`, `base_3`… if taken.
pub fn fresh_key(map: &Mapping, base: &str) -> String {
    if !map.contains_key(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|key| !map.contains_key(key.as_str()))
        .unwrap_or_else(|| base.to_string())
}

/// `value` in YAML flow style on one line: `{weekly: {days: [mon, fri], time: '09:30'}}`.
pub fn flow(value: &Value) -> String {
    match value {
        Value::Mapping(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(key, value)| format!("{}: {}", key_text(key), flow(value)))
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        Value::Sequence(items) => {
            let items: Vec<String> = items.iter().map(flow).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Tagged(tagged) => flow(&tagged.value),
        scalar => serde_yaml::to_string(scalar)
            .map(|text| text.trim_end().to_string())
            .unwrap_or_default(),
    }
}

/// Builds a mapping from `(key, value)` pairs.
pub fn map_of<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Mapping(
        entries
            .into_iter()
            .map(|(key, value)| (Value::from(key), value))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = "\
jobs:
  zeta:
    legacy_field: kept
    tasks:
      second: {cmd: [echo, b]}
      first: {cmd: [echo, a]}
  alpha: jobs/a.yaml
";

    fn doc() -> (Doc, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.yaml");
        std::fs::write(&path, YAML).unwrap();
        (Doc::open(&path).unwrap(), dir)
    }

    #[test]
    fn round_trip_keeps_order_and_unknown_fields() {
        let (doc, _dir) = doc();
        assert_eq!(doc.job_names(), ["zeta", "alpha"]);
        let text = doc.text().unwrap();
        assert!(text.find("second").unwrap() < text.find("first").unwrap());
        assert!(text.contains("legacy_field: kept"));
        assert!(!doc.is_dirty(&text));
    }

    #[test]
    fn rename_and_move_keep_positions() {
        let (mut doc, _dir) = doc();
        let jobs = doc.jobs_mut();
        assert!(rename(jobs, "zeta", "beta"));
        assert!(!rename(jobs, "beta", "alpha"));
        assert_eq!(keys(jobs), ["beta", "alpha"]);
        move_entry(jobs, 0, 1);
        assert_eq!(keys(jobs), ["alpha", "beta"]);
        jobs.insert(Value::from("gamma"), Value::Null);
        move_entry(jobs, 2, 0);
        assert_eq!(keys(jobs), ["gamma", "alpha", "beta"]);
        move_entry(jobs, 0, 2);
        assert_eq!(keys(jobs), ["alpha", "beta", "gamma"]);
        set(jobs, "gamma", None);
        set(jobs, "alpha", None);
        assert_eq!(keys(jobs), ["beta"]);
        assert!(doc.is_dirty(&doc.text().unwrap()));
    }

    #[test]
    fn scalars_parse_numbers_and_bools_only() {
        assert_eq!(parse_scalar("200"), Value::from(200));
        assert_eq!(parse_scalar("true"), Value::from(true));
        assert_eq!(parse_scalar("a: b"), Value::from("a: b"));
        assert_eq!(scalar_text(&Value::from(200)), "200");
        assert_eq!(fresh_key(doc().0.jobs_mut(), "zeta"), "zeta_2");
    }
}
