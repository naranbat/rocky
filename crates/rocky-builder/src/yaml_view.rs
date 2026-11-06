//! The read-only YAML view of the document, as Save would write it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use crate::theme;

/// Draws `text` in `area` from line `scroll`, keeping `scroll` within the text.
pub fn draw(frame: &mut Frame, area: Rect, text: &str, scroll: &mut u16) {
    let lines = u16::try_from(text.lines().count()).unwrap_or(u16::MAX);
    *scroll = (*scroll).min(lines.saturating_sub(area.height));
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(theme::text())
            .scroll((*scroll, 0)),
        area,
    );
}
