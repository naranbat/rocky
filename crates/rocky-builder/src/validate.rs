//! Checks the unsaved document with rocky-core, exactly as `rocky validate` would, shortly
//! after the last edit.

use std::path::Path;
use std::time::{Duration, Instant};

/// How long after the last edit to validate again.
const DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct Validation {
    /// `None` until the first check; then `Ok` or core's error chain.
    pub result: Option<Result<(), String>>,
    /// The text last checked, so unchanged text isn't checked again.
    checked: String,
    /// When the text last changed without being checked yet.
    pending_since: Option<Instant>,
}

impl Validation {
    /// Validates `text` if it changed at least [`DEBOUNCE`] ago, or right away with `now`.
    /// Returns how long to wait before calling again, while a check is pending.
    pub fn update(&mut self, path: &Path, text: &str, now: bool) -> Option<Duration> {
        if text != self.checked && self.pending_since.is_none() {
            self.pending_since = Some(Instant::now());
        }
        let due = self
            .pending_since
            .map(|since| since.elapsed() >= DEBOUNCE || self.result.is_none())
            .unwrap_or(false);
        if now || due {
            self.result = Some(check(path, text));
            self.checked = text.to_string();
            self.pending_since = None;
            return None;
        }
        self.pending_since
            .map(|since| DEBOUNCE.saturating_sub(since.elapsed()))
    }

    pub fn is_valid(&self) -> bool {
        matches!(self.result, Some(Ok(())))
    }
}

/// Core's verdict on `text` as the contents of the project file at `path`.
pub fn check(path: &Path, text: &str) -> Result<(), String> {
    rocky_core::project::load_str(path, text)
        .map(|_| ())
        .map_err(|error| error.chain())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_core_errors() {
        let path = Path::new("project.yaml");
        assert!(check(path, "jobs:\n  a:\n    tasks:\n      t: {cmd: [echo]}\n").is_ok());
        let error = check(path, "jobs:\n  a:\n    tasks:\n      t: {cmd: []}\n").unwrap_err();
        assert!(error.contains('a'), "{error}");
    }

    #[test]
    fn first_check_is_immediate_then_debounced() {
        let path = Path::new("project.yaml");
        let mut validation = Validation::default();
        validation.update(path, "jobs: {}\n", false);
        assert!(matches!(validation.result, Some(Err(_))));
        assert!(validation.update(path, "jobs: {a: 1}\n", false).is_some());
        assert!(validation.update(path, "jobs: {a: 1}\n", true).is_none());
    }
}
