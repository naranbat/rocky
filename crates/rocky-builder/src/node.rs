//! Paths into the document's YAML tree (`jobs.main.tasks.fetch.cmd[0]`), and edits made
//! through them: what a form row reads and writes.

use serde_yaml::{Mapping, Value};

use crate::doc;

/// One step of a path: a mapping key or a list index.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// Where a node sits in the document, from the root.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path(pub Vec<Seg>);

impl Path {
    pub fn key(&self, key: &str) -> Path {
        let mut path = self.clone();
        path.0.push(Seg::Key(key.to_string()));
        path
    }

    pub fn index(&self, index: usize) -> Path {
        let mut path = self.clone();
        path.0.push(Seg::Index(index));
        path
    }

    /// The path without its last step, and that step.
    fn split(&self) -> Option<(Path, &Seg)> {
        let (last, parent) = self.0.split_last()?;
        Some((Path(parent.to_vec()), last))
    }

    /// The last key, if the path ends in one.
    pub fn last_key(&self) -> Option<&str> {
        match self.0.last() {
            Some(Seg::Key(key)) => Some(key),
            _ => None,
        }
    }
}

pub fn get<'a>(root: &'a Value, path: &Path) -> Option<&'a Value> {
    path.0.iter().try_fold(root, |value, seg| match seg {
        Seg::Key(key) => value.as_mapping()?.get(key.as_str()),
        Seg::Index(index) => value.as_sequence()?.get(*index),
    })
}

pub fn get_mut<'a>(root: &'a mut Value, path: &Path) -> Option<&'a mut Value> {
    path.0.iter().try_fold(root, |value, seg| match seg {
        Seg::Key(key) => value.as_mapping_mut()?.get_mut(key.as_str()),
        Seg::Index(index) => value.as_sequence_mut()?.get_mut(*index),
    })
}

/// The mapping at `path`, created along the way where keys are missing.
pub fn map_mut<'a>(root: &'a mut Value, path: &Path) -> Option<&'a mut Mapping> {
    let mut value = root;
    for seg in &path.0 {
        value = match seg {
            Seg::Key(key) => doc::as_map(value)
                .entry(Value::from(key.as_str()))
                .or_insert(Value::Null),
            Seg::Index(index) => value.as_sequence_mut()?.get_mut(*index)?,
        };
    }
    Some(doc::as_map(value))
}

/// Sets the node at `path`, or removes it with `None`. A missing parent mapping is created.
pub fn set(root: &mut Value, path: &Path, value: Option<Value>) -> bool {
    let Some((parent, last)) = path.split() else {
        return false;
    };
    match last {
        Seg::Key(key) => match value {
            Some(value) => match map_mut(root, &parent) {
                Some(map) => {
                    map.insert(Value::from(key.as_str()), value);
                    true
                }
                None => false,
            },
            None => get_mut(root, &parent)
                .and_then(Value::as_mapping_mut)
                .and_then(|map| map.shift_remove(key.as_str()))
                .is_some(),
        },
        Seg::Index(index) => {
            let Some(items) = get_mut(root, &parent).and_then(Value::as_sequence_mut) else {
                return false;
            };
            match value {
                Some(value) if *index < items.len() => items[*index] = value,
                Some(value) => items.push(value),
                None if *index < items.len() => {
                    items.remove(*index);
                }
                None => return false,
            }
            true
        }
    }
}

/// Moves the entry at `path` one place up (`-1`) or down (`1`) among its siblings.
/// Returns its new path.
pub fn shift(root: &mut Value, path: &Path, by: isize) -> Option<Path> {
    let (parent, last) = path.split()?;
    let node = get_mut(root, &parent)?;
    match (last, node) {
        (Seg::Key(key), Value::Mapping(map)) => {
            let from = map.keys().position(|k| k.as_str() == Some(key))?;
            let to = from.checked_add_signed(by).filter(|to| *to < map.len())?;
            doc::move_entry(map, from, to);
            Some(path.clone())
        }
        (Seg::Index(from), Value::Sequence(items)) => {
            let to = from.checked_add_signed(by).filter(|to| *to < items.len())?;
            items.swap(*from, to);
            Some(parent.index(to))
        }
        _ => None,
    }
}

/// Renames the mapping key `path` ends in. Returns the new path, or `None` if the name is
/// empty or taken.
pub fn rename(root: &mut Value, path: &Path, new: &str) -> Option<Path> {
    let (parent, Seg::Key(old)) = path.split()? else {
        return None;
    };
    let map = get_mut(root, &parent)?.as_mapping_mut()?;
    doc::rename(map, old, new).then(|| parent.key(new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> Value {
        serde_yaml::from_str("jobs: {a: {tasks: {t: {cmd: [echo, hi]}, u: {cmd: [x]}}}}").unwrap()
    }

    #[test]
    fn reads_and_writes_through_paths() {
        let mut root = root();
        let tasks = Path::default().key("jobs").key("a").key("tasks");
        let cmd = tasks.key("t").key("cmd");
        assert_eq!(get(&root, &cmd.index(1)), Some(&Value::from("hi")));
        assert!(set(&mut root, &cmd.index(2), Some(Value::from("there"))));
        assert!(set(&mut root, &cmd.index(0), None));
        assert_eq!(doc::flow(get(&root, &cmd).unwrap()), "[hi, there]");
        assert!(set(
            &mut root,
            &tasks.key("t").key("env").key("A"),
            Some(Value::from(1))
        ));
        assert_eq!(
            get(&root, &tasks.key("t").key("env").key("A")),
            Some(&Value::from(1))
        );
        assert_eq!(shift(&mut root, &cmd.index(0), 1), Some(cmd.index(1)));
        assert_eq!(shift(&mut root, &cmd.index(1), 1), None);
        assert_eq!(shift(&mut root, &tasks.key("u"), -1), Some(tasks.key("u")));
        let names = doc::keys(get(&root, &tasks).unwrap().as_mapping().unwrap());
        assert_eq!(names, ["u", "t"]);
        assert_eq!(rename(&mut root, &tasks.key("u"), "t"), None);
        assert_eq!(
            rename(&mut root, &tasks.key("u"), "v"),
            Some(tasks.key("v"))
        );
        assert!(set(&mut root, &tasks.key("v"), None));
        assert!(!set(&mut root, &tasks.key("v"), None));
    }
}
