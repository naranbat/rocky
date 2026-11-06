//! The read-only YAML view of the file shown, as Save would write it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use rocky_core::project::{JOBS_DIR, SETTINGS_FILE};

use crate::theme;

/// The file that the tree `text` gives for job `job`, or for `rocky.yaml` without one,
/// headed by its path.
pub fn file(text: &str, job: Option<&str>) -> String {
    let parts = match crate::parts::split(text) {
        Ok(parts) => parts,
        Err(error) => return format!("# cannot write YAML: {error}"),
    };
    match job {
        Some(job) => {
            let body = parts
                .jobs
                .into_iter()
                .find(|(name, _)| name == job)
                .map(|(_, text)| text)
                .unwrap_or_default();
            format!("# {JOBS_DIR}/{job}.yaml\n{body}")
        }
        None => format!(
            "# {SETTINGS_FILE}\n{}",
            parts
                .settings
                .unwrap_or_else(|| "# no settings: the file isn't written\n".to_string())
        ),
    }
}

/// Draws `text` in `area` from line `scroll`, keeping `scroll` within the text.
pub fn draw(frame: &mut Frame, area: Rect, text: &str, scroll: &mut u16) {
    let lines = u16::try_from(text.lines().count()).unwrap_or(u16::MAX);
    *scroll = (*scroll).min(lines.saturating_sub(area.height));
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(theme::value())
            .scroll((*scroll, 0)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_one_file_of_the_tree() {
        let text = "values: {A: 1}\njobs:\n  a: {tasks: {t: {cmd: [echo]}}}\n  b: {tasks: {}}\n";
        let a = file(text, Some("a"));
        assert!(a.starts_with("# jobs/a.yaml\ntasks:"), "{a}");
        assert!(!a.contains("jobs:") && !a.contains("values"), "{a}");
        assert_eq!(file(text, None), "# rocky.yaml\nvalues:\n  A: 1\n");
    }
}
