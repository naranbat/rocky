//! The schedule rows: its kind, that kind's fields, and when it runs next.

use jiff::Zoned;
use ratatui::text::Line;
use rocky_core::schedule::Schedule;
use serde_yaml::Value;

use crate::doc;
use crate::form::{Action, Choice, Field, Row};
use crate::node::{self, Path};
use crate::theme;

const KINDS: [&str; 6] = ["every", "hourly", "daily", "weekly", "at", "none"];
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// The schedule's kind: the key that names it, or `none`.
fn kind(schedule: Option<&Value>) -> &'static str {
    let Some(map) = schedule.and_then(Value::as_mapping) else {
        return "none";
    };
    KINDS[..5]
        .iter()
        .find(|kind| map.contains_key(**kind))
        .copied()
        .unwrap_or("none")
}

/// A one-line summary for the job list: `every 5m`, `daily 02:30`, `no schedule`.
pub fn summary(schedule: Option<&Value>) -> String {
    let Some(map) = schedule.and_then(Value::as_mapping) else {
        return "no schedule".to_string();
    };
    match kind(schedule) {
        "weekly" => {
            let weekly = map.get("weekly");
            let days = weekly
                .and_then(|w| w.get("days"))
                .and_then(Value::as_sequence)
                .map(|days| {
                    days.iter()
                        .map(doc::scalar_text)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let time = weekly
                .and_then(|w| w.get("time"))
                .map(doc::scalar_text)
                .unwrap_or_default();
            format!("weekly {days} {time}")
        }
        "hourly" => format!(
            "hourly :{:0>2}",
            map.get("hourly").map(doc::scalar_text).unwrap_or_default()
        ),
        "none" => "on start".to_string(),
        kind => format!(
            "{kind} {}",
            map.get(kind).map(doc::scalar_text).unwrap_or_default()
        ),
    }
}

/// A new schedule of `kind`, keeping `run_on_start` from the old one where it applies.
fn template(kind: &str, run_on_start: bool) -> Option<Value> {
    let mut map = match kind {
        "every" => return Some(doc::map_of([("every", Value::from("5m"))])),
        "hourly" => doc::map_of([("hourly", Value::from(0))]),
        "daily" => doc::map_of([("daily", Value::from("09:00"))]),
        "weekly" => doc::map_of([(
            "weekly",
            doc::map_of([
                ("days", Value::Sequence(vec![Value::from("mon")])),
                ("time", Value::from("09:00")),
            ]),
        )]),
        "at" => doc::map_of([("at", Value::from("2030-01-01T09:00:00"))]),
        _ if run_on_start => doc::map_of([]),
        _ => return None,
    };
    if run_on_start {
        doc::as_map(&mut map).insert(Value::from("run_on_start"), Value::from(true));
    }
    Some(map)
}

/// The rows of the `schedule` at `path`: its kind (Enter picks another), that kind's
/// fields, `run_on_start`, and when it runs next.
pub fn rows(root: &Value, path: &Path) -> Vec<Row> {
    let schedule = node::get(root, path);
    let current = kind(schedule);
    let run_on_start = schedule
        .and_then(|s| s.get("run_on_start"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let choices = KINDS
        .iter()
        .map(|kind| {
            Choice::new(
                kind,
                "",
                Action::Set(path.clone(), template(kind, run_on_start)),
            )
        })
        .collect();
    let mut rows = vec![Row::field(
        "schedule",
        Line::styled(summary(schedule), theme::accent()),
        Action::Pick("Schedule".to_string(), choices),
    )];
    let Some(schedule) = schedule else {
        rows.push(Row::hint(
            "No schedule: runs only with `rocky exec` or a trigger.",
        ));
        return rows;
    };
    match current {
        "every" => {
            rows.push(Row::edit("every", root, path.key("every"), Field::Text));
            rows.push(match schedule.get("delay") {
                Some(_) => Row::edit("delay", root, path.key("delay"), Field::Optional)
                    .removable(path.key("delay")),
                None => Row::add(
                    "delay · first run right away",
                    Action::Add(path.key("delay"), Value::from("30s"), Some(Field::Optional)),
                ),
            });
        }
        "hourly" => rows.push(Row::edit("minute", root, path.key("hourly"), Field::Number)),
        "daily" => rows.push(Row::edit("time", root, path.key("daily"), Field::Text)),
        "weekly" => {
            let weekly = path.key("weekly");
            let days: Vec<&str> = schedule
                .get("weekly")
                .and_then(|weekly| weekly.get("days"))
                .and_then(Value::as_sequence)
                .map(|days| days.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            for day in DAYS {
                let on = days.contains(&day);
                let toggled: Vec<Value> = DAYS
                    .iter()
                    .filter(|d| (**d == day) != days.contains(*d))
                    .map(|d| Value::from(*d))
                    .collect();
                let (mark, style) = if on {
                    ("[x]", theme::accent())
                } else {
                    ("[ ]", theme::dim())
                };
                let label = if day == "mon" { "days" } else { "" };
                rows.push(Row::field(
                    label,
                    Line::styled(format!("{mark} {day}"), style),
                    Action::Set(weekly.key("days"), Some(Value::Sequence(toggled))),
                ));
            }
            rows.push(Row::edit("time", root, weekly.key("time"), Field::Text));
        }
        "at" => rows.push(Row::edit("at", root, path.key("at"), Field::Text)),
        _ => rows.push(Row::hint("Runs once when `rocky run` starts.")),
    }
    if current != "every" {
        let mark = if run_on_start { "[x]" } else { "[ ]" };
        let toggled = if current == "none" {
            // `{run_on_start: true}` alone is the whole schedule.
            Action::Set(path.clone(), None)
        } else {
            Action::Set(
                path.key("run_on_start"),
                (!run_on_start).then(|| Value::from(true)),
            )
        };
        rows.push(Row::field(
            "run_on_start",
            Line::styled(
                format!("{mark} also run once when `rocky run` starts"),
                theme::text(),
            ),
            toggled,
        ));
    }
    rows.push(Row::hint(&next_runs(schedule, &Zoned::now())));
    rows
}

/// When `rocky run` would next run a job with `schedule`, counting from `now`, as
/// `Next runs · Fri 6 Nov 09:30 · Mon 9 Nov 09:30`, or why the schedule is invalid.
fn next_runs(schedule: &Value, now: &Zoned) -> String {
    let parsed = match serde_yaml::from_value::<Schedule>(schedule.clone()) {
        Ok(parsed) => parsed,
        Err(error) => return format!("Invalid schedule: {error}"),
    };
    let format = |time: &Zoned| time.strftime("%a %-d %b %H:%M").to_string();
    let mut runs = Vec::new();
    match parsed {
        Schedule::OnStart => return "Runs once, when `rocky run` starts".to_string(),
        Schedule::Every { interval, delay } => {
            let mut next = now.checked_add(delay).ok();
            for _ in 0..2 {
                let Some(time) = next else { break };
                runs.push(format(&time));
                next = time.checked_add(interval).ok();
            }
            if delay.is_zero()
                && let Some(first) = runs.first_mut()
            {
                *first = "at start".to_string();
            }
        }
        Schedule::Clock {
            clock,
            run_on_start,
        } => {
            if run_on_start {
                runs.push("at start".to_string());
            }
            let mut from = now.clone();
            while runs.len() < 3 {
                match clock.next_after(&from) {
                    Ok(Some(time)) => {
                        runs.push(format(&time));
                        from = time;
                    }
                    Ok(None) => break,
                    Err(error) => return format!("Invalid schedule: {}", error.chain()),
                }
            }
        }
    }
    if runs.is_empty() {
        return "No more runs: the time has passed".to_string();
    }
    format!("Next runs · {}", runs.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_next_runs() {
        let now: Zoned = "2026-11-06T10:00:00[UTC]".parse().unwrap();
        let weekly: Value =
            serde_yaml::from_str("{weekly: {days: [mon, fri], time: '09:30'}}").unwrap();
        assert_eq!(
            next_runs(&weekly, &now),
            "Next runs · Mon 9 Nov 09:30 · Fri 13 Nov 09:30 · Mon 16 Nov 09:30"
        );
        let every: Value = serde_yaml::from_str("{every: 5m, delay: 1m}").unwrap();
        assert_eq!(
            next_runs(&every, &now),
            "Next runs · Fri 6 Nov 10:01 · Fri 6 Nov 10:06"
        );
        let past: Value = serde_yaml::from_str("{at: '2020-01-01T00:00:00'}").unwrap();
        assert_eq!(next_runs(&past, &now), "No more runs: the time has passed");
        let bad: Value = serde_yaml::from_str("{daily: '25:00'}").unwrap();
        assert!(next_runs(&bad, &now).starts_with("Invalid schedule"));
    }

    #[test]
    fn templates_are_valid_schedules() {
        for kind in KINDS {
            for run_on_start in [false, true] {
                let Some(schedule) = template(kind, run_on_start) else {
                    continue;
                };
                assert_eq!(super::kind(Some(&schedule)), kind);
                let text = format!(
                    "jobs:\n  a:\n    schedule: {}\n    tasks:\n      t: {{cmd: [echo]}}\n",
                    doc::flow(&schedule)
                );
                crate::validate::check(std::path::Path::new("project"), &text)
                    .unwrap_or_else(|error| panic!("{kind}: {error:?}\n{text}"));
            }
        }
    }
}
