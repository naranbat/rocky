//! Where the parts of a YAML file are, found from its lines by indentation: a job file's
//! settings and each of its tasks, or one top-level key's block. Also the text edits that
//! move these parts around. Works on block-style YAML, as rocky's files are written.

use std::ops::Range;

/// One task of a job file: its name and its lines, comments right above it included.
pub struct Part {
    pub name: String,
    pub lines: Range<usize>,
}

pub struct Outline {
    /// The lines before `tasks:`: `schedule`, `values` and `defaults`.
    pub settings: Range<usize>,
    pub tasks: Vec<Part>,
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank_or_comment(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

/// The key a `key: value` or `key:` line starts with.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let end = trimmed.find(':')?;
    let key = &trimmed[..end];
    let after = &trimmed[end + 1..];
    let plain = !key.is_empty() && !key.contains([' ', '#', '-', '{', '[', '"', '\'']);
    (plain && (after.is_empty() || after.starts_with(' '))).then_some(key)
}

/// `range` without blank lines at its end.
fn trim_end(lines: &[String], mut range: Range<usize>) -> Range<usize> {
    while range.end > range.start && lines[range.end - 1].trim().is_empty() {
        range.end -= 1;
    }
    range
}

/// The lines of top-level `key`'s block, its own line first.
pub fn block(lines: &[String], key: &str) -> Option<Range<usize>> {
    let start = lines
        .iter()
        .position(|line| indent(line) == 0 && key_of(line) == Some(key))?;
    let end = (start + 1..lines.len())
        .find(|&i| indent(&lines[i]) == 0 && !is_blank_or_comment(&lines[i]))
        .unwrap_or(lines.len());
    Some(trim_end(lines, start..end))
}

/// The settings and tasks of a job file.
pub fn job(lines: &[String]) -> Outline {
    let Some(block) = block(lines, "tasks") else {
        return Outline {
            settings: trim_end(lines, 0..lines.len()),
            tasks: Vec::new(),
        };
    };
    let settings = trim_end(lines, 0..block.start);
    let body = block.start + 1..block.end;
    let Some(depth) = body
        .clone()
        .find(|&i| !is_blank_or_comment(&lines[i]))
        .map(|i| indent(&lines[i]))
    else {
        return Outline {
            settings,
            tasks: Vec::new(),
        };
    };
    let keys: Vec<usize> = body
        .clone()
        .filter(|&i| indent(&lines[i]) == depth && key_of(&lines[i]).is_some())
        .collect();
    // Each task starts at the comments right above its key.
    let starts: Vec<usize> = keys
        .iter()
        .map(|&key| {
            let mut start = key;
            while start > body.start && lines[start - 1].trim_start().starts_with('#') {
                start -= 1;
            }
            start
        })
        .collect();
    let tasks = keys
        .iter()
        .enumerate()
        .map(|(n, &key)| {
            let end = starts.get(n + 1).copied().unwrap_or(body.end);
            Part {
                name: key_of(&lines[key]).unwrap_or_default().to_string(),
                lines: trim_end(lines, starts[n]..end),
            }
        })
        .collect();
    Outline { settings, tasks }
}

/// Swaps task `index` with the one above (`up`) or below it. Returns the moved task's new
/// index, or `None` when it can't move that way.
pub fn move_task(lines: &mut Vec<String>, index: usize, up: bool) -> Option<usize> {
    let tasks = job(lines).tasks;
    let first = if up { index.checked_sub(1)? } else { index };
    let (a, b) = (tasks.get(first)?, tasks.get(first + 1)?);
    let mut moved = lines[b.lines.clone()].to_vec();
    moved.extend_from_slice(&lines[a.lines.end..b.lines.start]);
    moved.extend_from_slice(&lines[a.lines.clone()]);
    lines.splice(a.lines.start..b.lines.end, moved);
    Some(if up { first } else { first + 1 })
}

/// Adds a starter cmd task named `name` after task `after`, or last. Returns its index, or
/// `None` when the file's `tasks` is written inline and can't take a new line.
pub fn add_task(lines: &mut Vec<String>, after: Option<usize>, name: &str) -> Option<usize> {
    let outline = job(lines);
    let depth = outline
        .tasks
        .first()
        .map_or(2, |task| indent(&lines[key_line(lines, task)]));
    let pad = " ".repeat(depth);
    let new = [
        format!("{pad}{name}:"),
        format!("{pad}{pad}cmd: [echo, {name}]"),
    ];
    let (at, index) = match after.and_then(|n| outline.tasks.get(n).map(|t| (n, t))) {
        Some((n, task)) => (task.lines.end, n + 1),
        None => match outline.tasks.last() {
            Some(task) => (task.lines.end, outline.tasks.len()),
            None => match block(lines, "tasks") {
                Some(tasks) if lines[tasks.start].trim_end() == "tasks:" => (tasks.start + 1, 0),
                Some(_) => return None,
                None => {
                    lines.push("tasks:".to_string());
                    (lines.len(), 0)
                }
            },
        },
    };
    lines.splice(at..at, new);
    Some(index)
}

/// The line of `task`'s key: its first line that isn't a comment.
fn key_line(lines: &[String], task: &Part) -> usize {
    task.lines
        .clone()
        .find(|&i| !lines[i].trim_start().starts_with('#'))
        .unwrap_or(task.lines.start)
}

/// Where to start typing in `range`: the line after its first key, or that key's line.
pub fn start_line(lines: &[String], range: Range<usize>) -> usize {
    let key = range
        .clone()
        .find(|&i| !is_blank_or_comment(&lines[i]))
        .unwrap_or(range.start);
    if key + 1 < range.end { key + 1 } else { key }
}

/// A task name not in `outline`: `task`, `task2`, ...
pub fn free_name(outline: &Outline) -> String {
    (1..)
        .map(|n| match n {
            1 => "task".to_string(),
            n => format!("task{n}"),
        })
        .find(|name| outline.tasks.iter().all(|task| &task.name != name))
        .unwrap_or_default()
}

/// Adds `line` at the end of top-level `key`'s block, creating the block if there is none.
/// False when the block is written inline.
pub fn add_to_block(lines: &mut Vec<String>, key: &str, line: String) -> bool {
    match block(lines, key) {
        Some(range) if lines[range.start].trim_end() == format!("{key}:") => {
            // After the block's last entry, before any comments that end it.
            let end = range
                .rev()
                .find(|&i| !is_blank_or_comment(&lines[i]))
                .map_or(0, |i| i + 1);
            lines.insert(end, line);
            true
        }
        Some(_) => false,
        None => {
            if lines.len() == 1 && lines[0].is_empty() {
                lines.clear();
            }
            lines.push(format!("{key}:"));
            lines.push(line);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    const JOB: &str = "\
# a job
schedule:
  hourly: 15
tasks:
  one:
    cmd: [echo, 1]

  # the second
  two:
    when: { this: x, eq: 1 }
    cmd: [echo, 2]
  three: {cmd: [echo, 3]}
";

    #[test]
    fn finds_settings_and_tasks() {
        let lines = lines(JOB);
        let outline = job(&lines);
        assert_eq!(outline.settings, 0..3);
        let tasks: Vec<_> = outline
            .tasks
            .iter()
            .map(|t| (t.name.as_str(), t.lines.clone()))
            .collect();
        assert_eq!(tasks, [("one", 4..6), ("two", 7..11), ("three", 11..12)]);
        assert_eq!(block(&lines, "schedule"), Some(1..3));
        assert_eq!(block(&lines, "values"), None);
        assert_eq!(start_line(&lines, outline.settings.clone()), 2);
        assert_eq!(start_line(&lines, 11..12), 11);
    }

    #[test]
    fn moves_a_task_with_its_comments() {
        let mut text = lines(JOB);
        assert_eq!(move_task(&mut text, 1, true), Some(0));
        let names: Vec<_> = job(&text).tasks.into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["two", "one", "three"]);
        assert_eq!(text[4], "  # the second");
        assert_eq!(move_task(&mut text, 2, false), None);
        assert_eq!(move_task(&mut text, 0, true), None);
    }

    #[test]
    fn adds_tasks() {
        let mut text = lines(JOB);
        assert_eq!(add_task(&mut text, Some(0), "new"), Some(1));
        let names: Vec<_> = job(&text).tasks.into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["one", "new", "two", "three"]);
        assert_eq!(text[6..8], ["  new:", "    cmd: [echo, new]"]);

        let mut empty = lines("schedule:\n  every: 5m\n");
        assert_eq!(add_task(&mut empty, None, "task"), Some(0));
        assert_eq!(job(&empty).tasks[0].name, "task");
        assert_eq!(free_name(&job(&empty)), "task2");

        let mut inline = lines("tasks: {a: {cmd: [x]}}\n");
        assert_eq!(add_task(&mut inline, None, "b"), None);
    }

    #[test]
    fn adds_to_a_block() {
        let mut text = lines("env_file: x.env\njobs:\n  a: jobs/a.yaml\n# end\n");
        assert!(add_to_block(
            &mut text,
            "jobs",
            "  b: jobs/b.yaml".to_string()
        ));
        assert_eq!(text[3], "  b: jobs/b.yaml");
        let mut inline = lines("jobs: {a: jobs/a.yaml}\n");
        assert!(!add_to_block(&mut inline, "jobs", "  b: x".to_string()));
    }
}
