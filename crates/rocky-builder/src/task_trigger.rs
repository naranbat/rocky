//! The rows of a trigger task: which job to queue.

use ratatui::text::Line;
use serde_yaml::Value;

use crate::edit::Edit;
use crate::form::{Action, Choice, Row};
use crate::node::{self, Path};
use crate::theme;

pub fn rows(ed: &mut Edit, root: &Value, path: &Path) -> Vec<Row> {
    let key = path.key("trigger");
    let current = node::get(root, &key)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let choices = ed
        .jobs
        .iter()
        .map(|job| {
            Choice::new(
                job,
                "",
                Action::Set(key.clone(), Some(Value::from(job.as_str()))),
            )
        })
        .collect();
    vec![
        Row::field(
            "queue a run of",
            Line::styled(current.to_string(), theme::bold()),
            Action::Pick("Trigger job".to_string(), choices),
        ),
        Row::hint("doesn't wait · saves no result · can't fail; takes no other fields but when"),
    ]
}
