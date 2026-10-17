use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Map, Value};

use crate::error::Error;
use crate::state;
use crate::step::Step;

/// Project `vars` set by `set` steps, in memory only: every job run given the
/// same `ProjectVars` reads and updates one map. Names never updated aren't in it.
#[derive(Clone, Debug, Default)]
pub struct ProjectVars(Arc<Mutex<Map<String, Value>>>);

impl ProjectVars {
    pub fn snapshot(&self) -> Map<String, Value> {
        self.lock().clone()
    }

    #[cfg(test)]
    pub fn update(&self, set: Map<String, Value>) {
        self.lock().extend(set);
    }

    /// Calls `f` with the current values and applies the updates it returns, holding the
    /// lock throughout, so concurrent runs can't lose each other's updates.
    pub fn update_with<T>(
        &self,
        f: impl FnOnce(&Map<String, Value>) -> Result<(T, Map<String, Value>), Error>,
    ) -> Result<T, Error> {
        let mut vars = self.lock();
        let (result, set) = f(&vars)?;
        vars.extend(set);
        Ok(result)
    }

    /// A run that panicked can't have left the map half-updated (each change is one
    /// `extend`), so a poisoned lock is still safe to use.
    fn lock(&self) -> MutexGuard<'_, Map<String, Value>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Checks that every `set.vars` in `steps`, nested ones included, only sets names
/// declared in the project's `vars`.
pub fn check(steps: &[Step], vars: &BTreeMap<String, String>) -> Result<(), Error> {
    match state::undeclared(steps, vars, |step| step.set.as_ref().map(|set| &set.vars)) {
        Some(name) => Err(Error::InvalidStep(format!(
            "`set.vars` sets `{name}`, which the project's `vars` doesn't declare"
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn update_overwrites_and_keeps_other_names() {
        let vars = ProjectVars::default();
        vars.update(Map::from_iter([
            ("A".to_string(), json!(1)),
            ("B".to_string(), json!("b")),
        ]));
        vars.update(Map::from_iter([("A".to_string(), json!([2]))]));
        assert_eq!(Value::Object(vars.snapshot()), json!({"A": [2], "B": "b"}));
    }

    #[test]
    fn check_finds_undeclared_names_in_nested_steps() {
        let steps = crate::step::named(serde_yaml::Deserializer::from_str(
            "a: {set: {vars: {A: 1}}}\n\
             b:\n  for_each: L\n  do: {c: {set: {vars: {C: 1}}}}\n",
        ))
        .unwrap();
        let vars = BTreeMap::from([("A".to_string(), "a".to_string())]);
        let err = check(&steps, &vars).unwrap_err();
        assert!(err.to_string().contains("`C`"), "{err}");
        check(&steps[..1], &vars).unwrap();
    }
}
