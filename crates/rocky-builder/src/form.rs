//! The main pane as a list of rows: headings, notes, and fields that do something on Enter.
//! Views build the rows from the document each frame; the app applies the focused row's
//! action back through its [`Path`].

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use serde_yaml::Value;

use crate::complete::Mode;
use crate::doc;
use crate::input::Input;
use crate::node::Path;
use crate::theme;

/// Width of the label column.
const LABEL: usize = 16;

/// How typed text becomes the value at a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    /// A string, kept even when empty.
    Text,
    /// A string; empty text removes it.
    Optional,
    /// Numbers and booleans as typed, anything else a string.
    Scalar,
    /// A whole number; empty text removes it.
    Number,
    /// Any YAML on one line, in flow style: `{every: 5m}`, `[mon, fri]`.
    Flow,
    /// A string that is a whole variable path (`for_each`), where `${}` is optional.
    Var,
}

impl Field {
    /// The text a field over `value` starts with.
    pub fn text(self, value: Option<&Value>) -> String {
        match (self, value) {
            (_, None) => String::new(),
            (Field::Flow, Some(value)) => match value {
                Value::Mapping(_) | Value::Sequence(_) => doc::flow(value),
                other => doc::scalar_text(other),
            },
            (_, Some(value)) => doc::scalar_text(value),
        }
    }

    /// How the field suggests variables, if it does.
    pub fn completion(self) -> Option<Mode> {
        match self {
            Field::Number => None,
            Field::Var => Some(Mode::Whole),
            _ => Some(Mode::Inline),
        }
    }

    /// The value `text` stands for; `Ok(None)` removes it.
    pub fn parse(self, text: &str) -> Result<Option<Value>, String> {
        Ok(match self {
            Field::Text | Field::Var => Some(Value::from(text)),
            Field::Optional => (!text.is_empty()).then(|| Value::from(text)),
            Field::Scalar => Some(doc::parse_scalar(text)),
            Field::Number => match text.trim() {
                "" => None,
                trimmed => Some(
                    trimmed
                        .parse::<u64>()
                        .map_or_else(|_| Value::from(text), Value::from),
                ),
            },
            Field::Flow if text.trim().is_empty() => None,
            Field::Flow => Some(serde_yaml::from_str(text).map_err(|error| error.to_string())?),
        })
    }
}

/// A choice in a picker, and what picking it does.
#[derive(Clone, Debug)]
pub struct Choice {
    pub label: String,
    pub detail: String,
    pub action: Action,
}

impl Choice {
    pub fn new(label: &str, detail: &str, action: Action) -> Choice {
        Choice {
            label: label.to_string(),
            detail: detail.to_string(),
            action,
        }
    }
}

/// What Enter on a row does.
#[derive(Clone, Debug)]
pub enum Action {
    None,
    /// Edit the value at the path as text.
    Edit(Path, Field),
    /// Edit the key the path ends in.
    Rename(Path),
    /// Set the value at the path, or remove it with `None`.
    Set(Path, Option<Value>),
    /// Set the value at the path, then edit it with the field, or rename its key with `None`.
    Add(Path, Value, Option<Field>),
    /// Choose one of several actions.
    Pick(String, Vec<Choice>),
    /// Open a task of the list shown, by name.
    Open(String),
    /// Show the output of a task's last run.
    Output(String),
    /// Replace the old-syntax task `name` of the task list at the path with plain tasks.
    Convert(Path, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Heading,
    Note,
    Field,
}

pub struct Row {
    pub kind: Kind,
    pub label: Span<'static>,
    pub value: Line<'static>,
    pub action: Action,
    /// What `x` removes, `J` / `K` move, and `r` renames.
    pub remove: Option<Path>,
    pub shift: Option<Path>,
    pub rename: Option<Path>,
    /// The task this row stands for, for run-until and output.
    pub task: Option<String>,
}

impl Row {
    fn new(kind: Kind, label: Span<'static>, value: Line<'static>, action: Action) -> Row {
        Row {
            kind,
            label,
            value,
            action,
            remove: None,
            shift: None,
            rename: None,
            task: None,
        }
    }

    /// `TASKS · 3   run in order`.
    pub fn heading(title: &str, hint: &str) -> Row {
        Row::new(
            Kind::Heading,
            Span::styled(title.to_string(), theme::heading()),
            Line::styled(hint.to_string(), theme::faint()),
            Action::None,
        )
    }

    /// A line of text that can't be focused.
    pub fn note(text: impl Into<Line<'static>>) -> Row {
        Row::new(Kind::Note, Span::raw(""), text.into(), Action::None)
    }

    /// A dim note.
    pub fn hint(text: &str) -> Row {
        Row::note(Line::styled(text.to_string(), theme::dim()))
    }

    pub fn field(label: &str, value: impl Into<Line<'static>>, action: Action) -> Row {
        Row::new(
            Kind::Field,
            Span::styled(label.to_string(), theme::dim()),
            value.into(),
            action,
        )
    }

    /// A text field over the value at `path`.
    pub fn edit(label: &str, root: &Value, path: Path, field: Field) -> Row {
        let value = value_line(crate::node::get(root, &path));
        Row::field(label, value, Action::Edit(path, field))
    }

    /// A `+ label` row that runs `action`.
    pub fn add(label: &str, action: Action) -> Row {
        Row::field("", Line::styled(format!("+ {label}"), theme::dim()), action)
    }

    /// Like [`Row::add`], but in the label column, starting where field labels do.
    pub fn add_in_label(label: &str, action: Action) -> Row {
        Row::field(&format!("+ {label}"), Line::default(), action)
    }

    pub fn label_style(mut self, style: Style) -> Row {
        self.label = self.label.style(style);
        self
    }

    pub fn removable(mut self, path: Path) -> Row {
        self.remove = Some(path);
        self
    }

    pub fn movable(mut self, path: Path) -> Row {
        self.shift = Some(path);
        self
    }

    pub fn renamable(mut self, path: Path) -> Row {
        self.rename = Some(path);
        self
    }

    pub fn for_task(mut self, name: &str) -> Row {
        self.task = Some(name.to_string());
        self
    }

    pub fn focusable(&self) -> bool {
        self.kind == Kind::Field
    }
}

/// A value as shown in a row: numbers and booleans colored, `${…}` highlighted,
/// collections in flow style, a missing value as `—`.
pub fn value_line(value: Option<&Value>) -> Line<'static> {
    match value {
        None | Some(Value::Null) => Line::styled("—", theme::dim()),
        Some(value @ (Value::Number(_) | Value::Bool(_))) => {
            Line::styled(doc::scalar_text(value), theme::NUMBER)
        }
        Some(Value::String(text)) => highlight(text),
        Some(other) => highlight(&doc::flow(other)),
    }
}

/// `text` with each `${…}` reference in the accent color.
pub fn highlight(text: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let end = rest[start..]
            .find('}')
            .map_or(rest.len(), |end| start + end + 1);
        spans.push(Span::styled(rest[..start].to_string(), theme::value()));
        spans.push(Span::styled(rest[start..end].to_string(), theme::accent()));
        rest = &rest[end..];
    }
    spans.push(Span::styled(rest.to_string(), theme::value()));
    Line::from(spans)
}

/// The next focusable row from `from` in direction `by`, or `from` if there is none.
pub fn step(rows: &[Row], from: usize, by: isize) -> usize {
    let mut at = from;
    loop {
        match at.checked_add_signed(by) {
            Some(next) if next < rows.len() => {
                at = next;
                if rows[at].focusable() {
                    return at;
                }
            }
            _ => return from,
        }
    }
}

/// `focus` if that row can be focused, else the nearest one that can, if any.
pub fn settle(rows: &[Row], focus: usize) -> usize {
    let focus = focus.min(rows.len().saturating_sub(1));
    if rows.get(focus).is_some_and(Row::focusable) {
        return focus;
    }
    let down = step(rows, focus, 1);
    if down != focus {
        return down;
    }
    step(rows, focus, -1)
}

/// How many lines rows `from..=to` take when `from` is the first one shown: one each, plus a
/// blank line above each heading but the first shown, to set the groups apart.
fn span(rows: &[Row], from: usize, to: usize) -> usize {
    (from..=to)
        .map(|index| 1 + usize::from(index > from && rows[index].kind == Kind::Heading))
        .sum()
}

/// Draws `rows` in `area`, scrolled (through `offset`) so the focused row shows, with a blank
/// line above each heading but the first shown. While `editing`, the focused row's value is
/// the text field; returns where it is.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    rows: &[Row],
    focus: Option<usize>,
    offset: &mut usize,
    editing: Option<&Input>,
) -> Option<Rect> {
    let mut field = None;
    let height = usize::from(area.height);
    if let Some(focus) = focus {
        if focus < *offset {
            // Show the heading above the first field of a section too.
            *offset = focus.saturating_sub(usize::from(
                focus > 0 && rows[focus - 1].kind == Kind::Heading,
            ));
        } else {
            while *offset < focus && span(rows, *offset, focus) > height {
                *offset += 1;
            }
        }
    }
    // No further down than needed to show the last row.
    if let Some(last) = rows.len().checked_sub(1) {
        *offset = (*offset).min(last);
        while *offset > 0 && span(rows, *offset - 1, last) <= height {
            *offset -= 1;
        }
    }
    let mut lines = 0;
    for (index, row) in rows.iter().enumerate().skip(*offset) {
        lines += usize::from(index > *offset && row.kind == Kind::Heading);
        if lines >= height {
            break;
        }
        let y = area.y + u16::try_from(lines).unwrap_or(0);
        lines += 1;
        let line_area = Rect::new(area.x, y, area.width, 1);
        let focused = focus == Some(index);
        let marker = if focused {
            Span::styled("▌", theme::accent())
        } else {
            Span::raw(" ")
        };
        let label_text = row.label.content.to_string();
        let pad = LABEL.saturating_sub(label_text.chars().count()).max(1);
        let mut spans = vec![marker];
        match row.kind {
            Kind::Heading => {
                spans.push(row.label.clone());
                spans.push(Span::raw("  "));
                spans.extend(row.value.spans.iter().cloned());
            }
            Kind::Note => {
                spans.push(Span::raw(" ".repeat(LABEL + 1)));
                spans.extend(row.value.spans.iter().cloned());
            }
            Kind::Field => {
                spans.push(row.label.clone());
                spans.push(Span::raw(" ".repeat(pad)));
                if !(focused && editing.is_some()) {
                    spans.extend(row.value.spans.iter().cloned());
                }
            }
        }
        let mut line = Paragraph::new(Line::from(spans));
        if focused {
            line = line.bg(theme::SELECTED);
        }
        frame.render_widget(line, line_area);
        if focused && let Some(input) = editing {
            let x = 1 + u16::try_from(LABEL.max(label_text.chars().count() + 1)).unwrap_or(0);
            let input_area = Rect::new(
                area.x + x.min(area.width),
                y,
                area.width.saturating_sub(x),
                1,
            );
            input.draw(frame, input_area);
            field = Some(input_area);
        }
    }
    field
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    /// The text of each line `rows` draw in an area `height` lines high, focused on `focus`,
    /// with runs of spaces as one.
    fn drawn(rows: &[Row], height: u16, focus: usize) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(30, height)).unwrap();
        let mut offset = 0;
        terminal
            .draw(|frame| {
                draw(frame, frame.area(), rows, Some(focus), &mut offset, None);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let line: String = (0..30).map(|x| buffer[(x, y)].symbol()).collect();
                let line = line.trim_start_matches('▌');
                line.split_whitespace().collect::<Vec<_>>().join(" ")
            })
            .collect()
    }

    #[test]
    fn add_in_label_starts_where_labels_do() {
        let rows = [
            Row::field("cmd", Line::raw("echo"), Action::None),
            Row::add("arg", Action::None),
            Row::add_in_label("stdin · env", Action::None),
        ];
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        let mut offset = 0;
        terminal
            .draw(|frame| {
                draw(frame, frame.area(), &rows, None, &mut offset, None);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let column = |y: u16, text: &str| {
            (0..40u16)
                .find(|&x| buffer[(x, y)].symbol() == text)
                .unwrap()
        };
        assert_eq!(column(2, "+"), column(0, "c"));
        assert!(column(1, "+") > column(0, "c"));
    }

    #[test]
    fn groups_have_a_blank_line_above_their_heading() {
        let field = |name: &str| Row::field(name, Line::raw("x"), Action::None);
        let rows = [
            Row::heading("ONE", ""),
            field("a"),
            Row::heading("TWO", ""),
            field("b"),
        ];
        assert_eq!(drawn(&rows, 6, 1), ["ONE", "a x", "", "TWO", "b x", ""]);
        // Scrolled to the last field: `a` and the gap don't fit too, and a heading at the
        // top has no gap above it.
        assert_eq!(drawn(&rows, 3, 3), ["TWO", "b x", ""]);
    }

    #[test]
    fn parses_typed_text() {
        assert_eq!(Field::Optional.parse(""), Ok(None));
        assert_eq!(Field::Text.parse(""), Ok(Some(Value::from(""))));
        assert_eq!(Field::Number.parse(" 3 "), Ok(Some(Value::from(3))));
        assert_eq!(Field::Scalar.parse("true"), Ok(Some(Value::from(true))));
        let flow = Field::Flow.parse("{every: 5m}").unwrap().unwrap();
        assert_eq!(Field::Flow.text(Some(&flow)), "{every: 5m}");
        assert!(Field::Flow.parse("{every").is_err());
    }

    #[test]
    fn steps_over_headings_and_notes() {
        let rows = [
            Row::heading("A", ""),
            Row::field("a", "1", Action::None),
            Row::hint("note"),
            Row::field("b", "2", Action::None),
        ];
        assert_eq!(settle(&rows, 0), 1);
        assert_eq!(step(&rows, 1, 1), 3);
        assert_eq!(step(&rows, 3, 1), 3);
        assert_eq!(step(&rows, 3, -1), 1);
        assert_eq!(settle(&rows, 9), 3);
    }
}
