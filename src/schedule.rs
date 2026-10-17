use std::time::Duration;

use jiff::civil::{DateTime, Time, Weekday};
use jiff::{ToSpan, Zoned};
use serde::Deserialize;

use crate::error::Error;

/// When `run` runs a job.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawSchedule")]
pub enum Schedule {
    /// Repeatedly, `interval` apart, starting after `delay`.
    Every { interval: Duration, delay: Duration },
    /// At wall-clock times in the local time zone, and once at start if `run_on_start`.
    Clock { clock: Clock, run_on_start: bool },
}

#[derive(Debug)]
pub enum Clock {
    /// Every hour at this minute.
    Hourly(i8),
    /// Every day at this time.
    Daily(Time),
    /// On each listed day of the week at this time.
    Weekly { days: Vec<Weekday>, time: Time },
    /// Once.
    At(DateTime),
}

impl Clock {
    /// The first run time strictly after `now`, or `None` if there are no more runs.
    pub fn next_after(&self, now: &Zoned) -> Result<Option<Zoned>, Error> {
        let next = match self {
            Clock::Hourly(minute) => {
                let at_minute = |time: Zoned| {
                    time.with()
                        .minute(*minute)
                        .second(0)
                        .subsec_nanosecond(0)
                        .build()
                };
                // An hour on always lands after `now`. The earlier half-hour steps catch
                // the slot in this hour, and in zones whose clocks move by 30 minutes.
                let mut next = at_minute(now.checked_add(1.hour())?)?;
                for step in [0, 30] {
                    let candidate = at_minute(now.checked_add(step.minutes())?)?;
                    if candidate > *now && candidate < next {
                        next = candidate;
                    }
                }
                next
            }
            Clock::Daily(time) => {
                let today = now
                    .date()
                    .to_datetime(*time)
                    .to_zoned(now.time_zone().clone())?;
                if today > *now {
                    today
                } else {
                    now.date()
                        .tomorrow()?
                        .to_datetime(*time)
                        .to_zoned(now.time_zone().clone())?
                }
            }
            Clock::Weekly { days, time } => {
                let mut date = now.date();
                loop {
                    if days.contains(&date.weekday()) {
                        let candidate =
                            date.to_datetime(*time).to_zoned(now.time_zone().clone())?;
                        if candidate > *now {
                            break candidate;
                        }
                    }
                    date = date.tomorrow()?;
                }
            }
            Clock::At(datetime) => {
                let at = datetime.to_zoned(now.time_zone().clone())?;
                if at <= *now {
                    return Ok(None);
                }
                at
            }
        };
        Ok(Some(next))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchedule {
    every: Option<String>,
    delay: Option<String>,
    hourly: Option<i8>,
    daily: Option<String>,
    weekly: Option<Vec<String>>,
    time: Option<String>,
    at: Option<String>,
    #[serde(default)]
    run_on_start: bool,
}

impl TryFrom<RawSchedule> for Schedule {
    type Error = Error;

    fn try_from(raw: RawSchedule) -> Result<Self, Error> {
        let invalid = |msg: String| Error::InvalidSchedule(msg);
        let RawSchedule {
            every,
            delay,
            hourly,
            daily,
            weekly,
            time,
            at,
            run_on_start,
        } = raw;
        if delay.is_some() && every.is_none() {
            return Err(invalid("`delay` only works with `every`".to_string()));
        }
        if time.is_some() && weekly.is_none() {
            return Err(invalid("`time` only works with `weekly`".to_string()));
        }
        if run_on_start && every.is_some() {
            return Err(invalid(
                "`run_on_start` doesn't work with `every`, which runs at start unless `delay` is set"
                    .to_string(),
            ));
        }
        let parse_time = |field: &str, text: String| {
            text.parse::<Time>()
                .map_err(|_| invalid(format!("`{field}: {text}` must be a time like \"09:30\"")))
        };
        let clock = match (every, hourly, daily, weekly, at) {
            (Some(every), None, None, None, None) => {
                let interval = parse_duration(&every).map_err(invalid)?;
                if interval.is_zero() {
                    return Err(invalid("`every` must be greater than 0".to_string()));
                }
                let delay = delay
                    .as_deref()
                    .map(parse_duration)
                    .transpose()
                    .map_err(invalid)?;
                return Ok(Schedule::Every {
                    interval,
                    delay: delay.unwrap_or_default(),
                });
            }
            (None, Some(minute), None, None, None) => {
                if !(0..60).contains(&minute) {
                    return Err(invalid(format!("`hourly: {minute}` must be a minute 0-59")));
                }
                Clock::Hourly(minute)
            }
            (None, None, Some(daily), None, None) => Clock::Daily(parse_time("daily", daily)?),
            (None, None, None, Some(weekly), None) => {
                if weekly.is_empty() {
                    return Err(invalid("`weekly` needs at least one day".to_string()));
                }
                let days = weekly
                    .iter()
                    .map(|day| parse_day(day))
                    .collect::<Result<_, _>>()
                    .map_err(invalid)?;
                let Some(time) = time else {
                    return Err(invalid(
                        "`weekly` needs a `time` like \"09:30\"".to_string(),
                    ));
                };
                Clock::Weekly {
                    days,
                    time: parse_time("time", time)?,
                }
            }
            (None, None, None, None, Some(datetime)) => {
                Clock::At(datetime.parse().map_err(|_| {
                    invalid(format!(
                        "`at: {datetime}` must be a date and time like \"2027-10-10T09:30:00\""
                    ))
                })?)
            }
            _ => {
                return Err(invalid(
                    "needs exactly one of `every`, `hourly`, `daily`, `weekly` or `at`".to_string(),
                ));
            }
        };
        Ok(Schedule::Clock {
            clock,
            run_on_start,
        })
    }
}

/// Parses `mon` to `sun`; the error is a message for the caller's error type.
fn parse_day(text: &str) -> Result<Weekday, String> {
    Ok(match text {
        "mon" => Weekday::Monday,
        "tue" => Weekday::Tuesday,
        "wed" => Weekday::Wednesday,
        "thu" => Weekday::Thursday,
        "fri" => Weekday::Friday,
        "sat" => Weekday::Saturday,
        "sun" => Weekday::Sunday,
        _ => return Err(format!("`{text}` is not a day like `mon`")),
    })
}

/// Parses `30s`, `5m` or `2h`; the error is a message for the caller's error type.
pub(crate) fn parse_duration(text: &str) -> Result<Duration, String> {
    let invalid = || format!("`{text}` is not a duration like `30s`, `5m` or `2h`");
    let unit = match text.chars().last() {
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 3600,
        _ => return Err(invalid()),
    };
    let count: u64 = text[..text.len() - 1].parse().map_err(|_| invalid())?;
    count
        .checked_mul(unit)
        .map(Duration::from_secs)
        .ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Schedule, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    fn zoned(text: &str) -> Zoned {
        text.parse().unwrap()
    }

    fn next(yaml: &str, now: &str) -> Option<Zoned> {
        let Schedule::Clock { clock, .. } = parse(yaml).unwrap() else {
            panic!("not a clock schedule: {yaml}");
        };
        clock.next_after(&zoned(now)).unwrap()
    }

    #[test]
    fn parses_every_with_optional_delay() {
        assert!(matches!(
            parse("every: 5m\n").unwrap(),
            Schedule::Every { interval, delay } if interval == Duration::from_secs(300) && delay.is_zero()
        ));
        assert!(matches!(
            parse("every: 2h\ndelay: 30s\n").unwrap(),
            Schedule::Every { interval, delay } if interval == Duration::from_secs(7200) && delay == Duration::from_secs(30)
        ));
    }

    #[test]
    fn rejects_invalid_schedules() {
        for (yaml, expected) in [
            ("every: 5\n", "not a duration"),
            ("every: 5d\n", "not a duration"),
            ("every: xm\n", "not a duration"),
            ("every: 0s\n", "greater than 0"),
            ("every: 5m\ndelay: soon\n", "not a duration"),
            ("daily: \"09:30\"\ndelay: 5s\n", "only works with `every`"),
            ("every: 5m\ndaily: \"09:30\"\n", "exactly one"),
            ("{}\n", "exactly one"),
            (
                "daily: \"09:30\"\nweekly: [mon]\ntime: \"09:30\"\n",
                "exactly one",
            ),
            (
                "daily: \"09:30\"\ntime: \"09:30\"\n",
                "only works with `weekly`",
            ),
            ("weekly: [mon]\n", "needs a `time`"),
            ("weekly: []\ntime: \"09:30\"\n", "at least one day"),
            (
                "weekly: [monday]\ntime: \"09:30\"\n",
                "`monday` is not a day",
            ),
            (
                "weekly: [mon]\ntime: \"9am\"\n",
                "`time: 9am` must be a time",
            ),
            (
                "every: 5m\nrun_on_start: true\n",
                "doesn't work with `every`",
            ),
            ("hourly: 60\n", "minute 0-59"),
            ("daily: \"25:00\"\n", "like \"09:30\""),
            ("at: tomorrow\n", "like \"2027-10-10T09:30:00\""),
            (
                "at: \"2026-10-10T09:30:00Z\"\n",
                "like \"2027-10-10T09:30:00\"",
            ),
        ] {
            let err = parse(yaml).unwrap_err().to_string();
            assert!(err.contains(expected), "{yaml}: {err}");
        }
        assert!(parse("monthly: 1\n").is_err());
    }

    #[test]
    fn hourly_runs_at_minute_past_next_hour() {
        assert_eq!(
            next("hourly: 15\n", "2026-10-09T10:05:00[UTC]"),
            Some(zoned("2026-10-09T10:15:00[UTC]"))
        );
        assert_eq!(
            next("hourly: 15\n", "2026-10-09T10:15:00[UTC]"),
            Some(zoned("2026-10-09T11:15:00[UTC]"))
        );
    }

    #[test]
    fn hourly_keeps_its_minute_across_half_hour_shifts() {
        // Lord Howe Island moves its clocks by 30 minutes.
        assert_eq!(
            next(
                "hourly: 45\n",
                "2026-04-05T01:45:00+11:00[Australia/Lord_Howe]"
            ),
            Some(zoned("2026-04-05T01:45:00+10:30[Australia/Lord_Howe]"))
        );
        assert_eq!(
            next(
                "hourly: 45\n",
                "2026-04-05T01:45:00+10:30[Australia/Lord_Howe]"
            ),
            Some(zoned("2026-04-05T02:45:00+10:30[Australia/Lord_Howe]"))
        );
        assert_eq!(
            next(
                "hourly: 45\n",
                "2026-10-04T01:45:00+10:30[Australia/Lord_Howe]"
            ),
            Some(zoned("2026-10-04T02:45:00+11:00[Australia/Lord_Howe]"))
        );
        // A repeated hour runs twice.
        assert_eq!(
            next(
                "hourly: 30\n",
                "2026-11-01T01:30:00-04:00[America/New_York]"
            ),
            Some(zoned("2026-11-01T01:30:00-05:00[America/New_York]"))
        );
    }

    #[test]
    fn daily_rolls_over_to_tomorrow() {
        assert_eq!(
            next("daily: \"09:30\"\n", "2026-10-09T08:00:00[UTC]"),
            Some(zoned("2026-10-09T09:30:00[UTC]"))
        );
        assert_eq!(
            next("daily: \"09:30\"\n", "2026-10-09T09:30:00[UTC]"),
            Some(zoned("2026-10-10T09:30:00[UTC]"))
        );
    }

    #[test]
    fn daily_takes_seconds() {
        assert_eq!(
            next("daily: \"09:30:15\"\n", "2026-10-09T09:30:00[UTC]"),
            Some(zoned("2026-10-09T09:30:15[UTC]"))
        );
    }

    #[test]
    fn daily_keeps_local_time_across_dst() {
        assert_eq!(
            next(
                "daily: \"09:30\"\n",
                "2026-03-07T12:00:00-05:00[America/New_York]"
            ),
            Some(zoned("2026-03-08T09:30:00-04:00[America/New_York]"))
        );
    }

    #[test]
    fn parses_run_on_start() {
        for (yaml, expected) in [
            ("daily: \"09:30\"\n", false),
            ("daily: \"09:30\"\nrun_on_start: true\n", true),
            ("weekly: [mon]\ntime: \"09:30\"\nrun_on_start: true\n", true),
        ] {
            assert!(
                matches!(parse(yaml).unwrap(), Schedule::Clock { run_on_start, .. } if run_on_start == expected),
                "{yaml}"
            );
        }
    }

    #[test]
    fn weekly_picks_next_listed_day() {
        // 2026-10-09 is a Friday.
        let weekly = "weekly: [mon, fri]\ntime: \"09:30\"\n";
        assert_eq!(
            next(weekly, "2026-10-09T08:00:00[UTC]"),
            Some(zoned("2026-10-09T09:30:00[UTC]"))
        );
        assert_eq!(
            next(weekly, "2026-10-09T09:30:00[UTC]"),
            Some(zoned("2026-10-12T09:30:00[UTC]"))
        );
        assert_eq!(
            next(
                "weekly: [fri]\ntime: \"09:30\"\n",
                "2026-10-09T10:00:00[UTC]"
            ),
            Some(zoned("2026-10-16T09:30:00[UTC]"))
        );
    }

    #[test]
    fn at_runs_once() {
        let at = "at: \"2026-10-10T09:30:00\"\n";
        assert_eq!(
            next(at, "2026-10-09T00:00:00[UTC]"),
            Some(zoned("2026-10-10T09:30:00[UTC]"))
        );
        assert_eq!(next(at, "2026-10-10T09:30:00[UTC]"), None);
        // An offset is ignored: the time is local.
        assert_eq!(
            next(
                "at: \"2026-10-10T09:30:00+09:00\"\n",
                "2026-10-09T00:00:00[UTC]"
            ),
            Some(zoned("2026-10-10T09:30:00[UTC]"))
        );
    }
}
