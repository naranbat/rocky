//! A job's schedule in a few words, for the job list.

use serde_yaml::Value;

use crate::summary::scalar;

/// `every 5m`, `daily 09:30`, `weekly mon,fri 09:30`, `hourly :15`, `on start`; `None`
/// without a schedule.
pub fn summary(schedule: Option<&Value>) -> Option<String> {
    let map = schedule?.as_mapping()?;
    let field = |key: &str| map.get(key);
    Some(if let Some(weekly) = field("weekly") {
        let days = weekly
            .get("days")
            .and_then(Value::as_sequence)
            .map(|days| days.iter().map(scalar).collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        let time = weekly.get("time").map(scalar).unwrap_or_default();
        format!("weekly {days} {time}")
    } else if let Some(minute) = field("hourly") {
        format!("hourly :{:0>2}", scalar(minute))
    } else if let Some((kind, value)) = ["every", "daily", "at"]
        .iter()
        .find_map(|kind| field(kind).map(|value| (kind, value)))
    {
        format!("{kind} {}", scalar(value))
    } else {
        "on start".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(yaml: &str) -> Option<String> {
        summary(Some(&serde_yaml::from_str(yaml).unwrap()))
    }

    #[test]
    fn summarizes_each_kind() {
        assert_eq!(of("every: 5m").as_deref(), Some("every 5m"));
        assert_eq!(of("daily: '09:30'").as_deref(), Some("daily 09:30"));
        assert_eq!(of("hourly: 5").as_deref(), Some("hourly :05"));
        assert_eq!(
            of("weekly: {days: [mon, fri], time: '09:30'}").as_deref(),
            Some("weekly mon,fri 09:30")
        );
        assert_eq!(of("run_on_start: true").as_deref(), Some("on start"));
        assert_eq!(summary(None), None);
    }
}
