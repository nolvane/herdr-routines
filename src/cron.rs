//! Five-field cron schedules (minute, hour, day of month, month, day of
//! week) in local time, with a plain-language description.
//!
//! Supports `*`, single values, ranges `a-b`, steps `*/n` and `a-b/n`, lists
//! `a,b`, month names (`jan`) and day names (`mon`); day of week 0 and 7 are
//! both Sunday. As in classic cron, when both day of month and day of week
//! are restricted, a day matching either one counts; a field starting with
//! `*` (such as `*/2`) does not count as restricted, so then both must match.

use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Timelike, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    pub text: String,
    minutes: Vec<bool>,
    hours: Vec<bool>,
    days: Vec<bool>,
    months: Vec<bool>,
    weekdays: Vec<bool>,
    days_any: bool,
    weekdays_any: bool,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn value(s: &str, names: &[&str], offset: u32) -> Result<u32, String> {
    let lower = s.to_ascii_lowercase();
    if let Some(i) = names.iter().position(|n| *n == lower) {
        return Ok(i as u32 + offset);
    }
    s.parse::<u32>()
        .map_err(|_| format!("\"{s}\" is not a number"))
}

/// One field into a table of allowed values `min..=max`.
fn field(spec: &str, min: u32, max: u32, names: &[&str], offset: u32) -> Result<Vec<bool>, String> {
    let mut allowed = vec![false; (max + 1) as usize];
    for part in spec.split(',') {
        if part.is_empty() {
            return Err("empty list entry".into());
        }
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step = s
                    .parse::<u32>()
                    .map_err(|_| format!("step \"{s}\" is not a number"))?;
                if step == 0 || step > max {
                    return Err(format!("a step must be between 1 and {max}"));
                }
                (r, step)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a, names, offset)?, value(b, names, offset)?)
        } else {
            let v = value(range, names, offset)?;
            // `5/15` means from 5 to the end in steps of 15.
            (v, if step > 1 { max } else { v })
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("\"{part}\" is outside {min} to {max}"));
        }
        let mut v = lo;
        while v <= hi {
            allowed[v as usize] = true;
            v += step;
        }
    }
    Ok(allowed)
}

impl Cron {
    pub fn parse(text: &str) -> Result<Cron, String> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!(
                "a schedule has 5 fields (minute hour day month weekday), this has {}",
                parts.len()
            ));
        }
        let minutes = field(parts[0], 0, 59, &[], 0).map_err(|e| format!("minute: {e}"))?;
        let hours = field(parts[1], 0, 23, &[], 0).map_err(|e| format!("hour: {e}"))?;
        let days = field(parts[2], 1, 31, &[], 0).map_err(|e| format!("day: {e}"))?;
        let months = field(parts[3], 1, 12, &MONTHS, 1).map_err(|e| format!("month: {e}"))?;
        let mut weekdays = field(parts[4], 0, 7, &DAYS, 0).map_err(|e| format!("weekday: {e}"))?;
        if weekdays[7] {
            weekdays[0] = true;
        }
        weekdays.truncate(7);
        Ok(Cron {
            text: parts.join(" "),
            minutes,
            hours,
            days,
            months,
            weekdays,
            // As in classic cron, `*/2` counts as unrestricted too.
            days_any: parts[2].starts_with('*'),
            weekdays_any: parts[4].starts_with('*'),
        })
    }

    fn day_matches<T: Datelike>(&self, t: &T) -> bool {
        if !self.months[t.month() as usize] {
            return false;
        }
        let dom = self.days[t.day() as usize];
        let dow = self.weekdays[t.weekday().num_days_from_sunday() as usize];
        // Classic cron: when either field starts with `*`, both must match
        // (a plain `*` matches every day anyway); otherwise either one.
        if self.days_any || self.weekdays_any {
            dom && dow
        } else {
            dom || dow
        }
    }

    /// The first matching minute strictly after `after`, within about a year.
    pub fn next_after<Tz: TimeZone>(&self, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
        let tz = after.timezone();
        let utc = after.with_timezone(&Utc);
        // Start at the next whole minute.
        let mut t = utc + Duration::seconds(60 - utc.second() as i64)
            - Duration::nanoseconds(utc.nanosecond() as i64);
        let end = utc + Duration::days(370);
        while t <= end {
            let local = t.with_timezone(&tz);
            let (h, m) = (local.hour(), local.minute());
            if !self.day_matches(&local) || !self.hours[h as usize] {
                // Jump to the next local hour.
                t += Duration::minutes(60 - m as i64);
                continue;
            }
            if self.minutes[m as usize] {
                return Some(local);
            }
            t += Duration::minutes(1);
        }
        None
    }

    /// The latest matching minute in `(after, until]`, if any.
    pub fn latest_in<Tz: TimeZone>(
        &self,
        after: &DateTime<Tz>,
        until: &DateTime<Tz>,
    ) -> Option<DateTime<Tz>> {
        let mut found = None;
        let mut from = after.clone();
        while let Some(next) = self.next_after(&from) {
            if next > *until {
                break;
            }
            found = Some(next.clone());
            from = next;
        }
        found
    }

    /// The schedule in words, such as "Weekdays, hourly at :15 from 08:15 to
    /// 18:15". Falls back to the cron text for shapes it does not know.
    pub fn describe(&self) -> String {
        describe(&self.text).unwrap_or_else(|| format!("Cron {}", self.text))
    }
}

fn single(spec: &str) -> Option<u32> {
    spec.parse().ok()
}

fn list(spec: &str) -> Option<Vec<u32>> {
    spec.split(',').map(|s| s.parse().ok()).collect()
}

fn days_phrase(dom: &str, month: &str, dow: &str) -> Option<String> {
    if month != "*" {
        return None;
    }
    let lower = dow.to_ascii_lowercase();
    let by_week = match lower.as_str() {
        "*" => None,
        "1-5" | "mon-fri" => Some("Weekdays".to_string()),
        "0,6" | "6,0" | "6,7" | "sat,sun" | "sun,sat" => Some("Weekends".to_string()),
        _ => {
            let names: Option<Vec<&str>> = lower
                .split(',')
                .map(|d| {
                    let i = value(d, &DAYS, 0).ok()? % 7;
                    Some(DAY_NAMES[i as usize])
                })
                .collect();
            let names = names?;
            Some(if names.len() == 1 {
                format!("{}s", names[0])
            } else {
                names.iter().map(|n| &n[..3]).collect::<Vec<_>>().join(", ")
            })
        }
    };
    match (dom, by_week) {
        ("*", None) => Some("Daily".into()),
        ("*", Some(w)) => Some(w),
        (d, None) => {
            let days = list(d)?;
            let ords: Vec<String> = days.iter().map(|d| ordinal(*d)).collect();
            Some(format!("Monthly on the {}", ords.join(" and ")))
        }
        _ => None,
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, x) if x != 11 => "st",
        (2, x) if x != 12 => "nd",
        (3, x) if x != 13 => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn describe(text: &str) -> Option<String> {
    let p: Vec<&str> = text.split_whitespace().collect();
    let (min, hour) = (p[0], p[1]);
    let days = days_phrase(p[2], p[3], p[4])?;
    let time = |h: u32, m: u32| format!("{h:02}:{m:02}");

    // "Daily, every hour" reads better as "Every hour".
    let lead = |rest: String| {
        if days == "Daily" {
            let mut c = rest.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        } else {
            format!("{days}, {rest}")
        }
    };
    if min == "*" && hour == "*" {
        return Some(lead("every minute".into()));
    }
    if let Some(step) = min.strip_prefix("*/") {
        let step: u32 = step.parse().ok()?;
        let every = format!("every {step} minutes");
        return Some(match hour {
            "*" => lead(every),
            h => {
                let (a, b) = h.split_once('-')?;
                let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
                format!("{days}, {every} from {} to {}", time(a, 0), time(b, 59))
            }
        });
    }
    let m = single(min)?;
    if hour == "*" {
        return Some(lead(format!("every hour at :{m:02}")));
    }
    if let Some((a, b)) = hour.split_once('-') {
        let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
        return Some(format!(
            "{days}, hourly at :{m:02} from {} to {}",
            time(a, m),
            time(b, m)
        ));
    }
    if let Some(step) = hour.strip_prefix("*/") {
        let step: u32 = step.parse().ok()?;
        return Some(lead(format!("every {step} hours at :{m:02}")));
    }
    let hours = list(hour)?;
    let times: Vec<String> = hours.iter().map(|h| time(*h, m)).collect();
    Some(format!("{days} at {}", times.join(", ")))
}

/// The time of a run as shown in lists: "08:15" today, "Tue 08:15" this
/// week, "12 Nov 08:15" otherwise.
pub fn short_time(t: &DateTime<Local>, now: &DateTime<Local>) -> String {
    let days = (t.date_naive() - now.date_naive()).num_days();
    if days == 0 {
        t.format("%H:%M").to_string()
    } else if (-6..=6).contains(&days) {
        t.format("%a %H:%M").to_string()
    } else {
        t.format("%-d %b %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    #[test]
    fn parses_and_rejects() {
        assert!(Cron::parse("15 8-18 * * 1-5").is_ok());
        assert!(Cron::parse("*/5 * * * *").is_ok());
        assert!(Cron::parse("0 9 * jan mon").is_ok());
        assert!(Cron::parse("0 9 * *").is_err());
        assert!(Cron::parse("60 9 * * *").is_err());
        assert!(Cron::parse("0 9 * * 8").is_err());
        assert!(Cron::parse("*/0 * * * *").is_err());
        assert!(Cron::parse("1-5/4294967295 * * * *").is_err());
        assert!(Cron::parse("0 9-5 * * *").is_err());
    }

    #[test]
    fn next_on_weekdays() {
        let c = Cron::parse("15 8-18 * * 1-5").unwrap();
        // Monday 2026-11-09 08:00 → 08:15.
        let n = c.next_after(&at("2026-11-09T08:00:00+01:00")).unwrap();
        assert_eq!(n, at("2026-11-09T08:15:00+01:00"));
        // Exactly at a slot → the next one.
        let n = c.next_after(&at("2026-11-09T08:15:00+01:00")).unwrap();
        assert_eq!(n, at("2026-11-09T09:15:00+01:00"));
        // Friday evening → Monday morning.
        let n = c.next_after(&at("2026-11-13T19:00:00+01:00")).unwrap();
        assert_eq!(n, at("2026-11-16T08:15:00+01:00"));
    }

    #[test]
    fn steps_and_lists() {
        let c = Cron::parse("*/20 * * * *").unwrap();
        let n = c.next_after(&at("2026-11-09T08:41:30+01:00")).unwrap();
        assert_eq!(n, at("2026-11-09T09:00:00+01:00"));
        let c = Cron::parse("0 7,19 * * *").unwrap();
        let n = c.next_after(&at("2026-11-09T08:00:00+01:00")).unwrap();
        assert_eq!(n, at("2026-11-09T19:00:00+01:00"));
    }

    #[test]
    fn day_of_month_or_week() {
        // The 1st or any Sunday.
        let c = Cron::parse("0 9 1 * 0").unwrap();
        let n = c.next_after(&at("2026-11-09T10:00:00+01:00")).unwrap();
        assert_eq!(n, at("2026-11-15T09:00:00+01:00"));
        let c = Cron::parse("0 9 1 * *").unwrap();
        let n = c.next_after(&at("2026-11-09T10:00:00+01:00")).unwrap();
        assert_eq!(n, at("2026-12-01T09:00:00+01:00"));
        // A stepped day of month with a weekday: both must match.
        let c = Cron::parse("0 9 */2 * 1").unwrap();
        let n = c.next_after(&at("2026-11-09T10:00:00+01:00")).unwrap();
        assert_eq!(n, at("2026-11-23T09:00:00+01:00"));
    }

    #[test]
    fn impossible_date_ends() {
        let c = Cron::parse("0 9 30 2 *").unwrap();
        assert_eq!(c.next_after(&at("2026-11-09T10:00:00+01:00")), None);
    }

    #[test]
    fn latest_slot_in_window() {
        let c = Cron::parse("15 8-18 * * 1-5").unwrap();
        let l = c.latest_in(
            &at("2026-11-09T07:00:00+01:00"),
            &at("2026-11-09T11:20:00+01:00"),
        );
        assert_eq!(l, Some(at("2026-11-09T11:15:00+01:00")));
        let none = c.latest_in(
            &at("2026-11-09T11:15:00+01:00"),
            &at("2026-11-09T12:00:00+01:00"),
        );
        assert_eq!(none, None);
    }

    #[test]
    fn descriptions() {
        let d = |s: &str| Cron::parse(s).unwrap().describe();
        assert_eq!(
            d("15 8-18 * * 1-5"),
            "Weekdays, hourly at :15 from 08:15 to 18:15"
        );
        assert_eq!(d("0 9 * * *"), "Daily at 09:00");
        assert_eq!(d("30 7,19 * * *"), "Daily at 07:30, 19:30");
        assert_eq!(d("0 9 * * 1"), "Mondays at 09:00");
        assert_eq!(d("0 9 * * mon,thu"), "Mon, Thu at 09:00");
        assert_eq!(d("5 * * * *"), "Every hour at :05");
        assert_eq!(d("*/15 * * * *"), "Every 15 minutes");
        assert_eq!(d("0 */3 * * *"), "Every 3 hours at :00");
        assert_eq!(d("* * * * *"), "Every minute");
        assert_eq!(d("0 * * * 1-5"), "Weekdays, every hour at :00");
        assert_eq!(d("0 8 1 * *"), "Monthly on the 1st at 08:00");
        assert_eq!(d("0 8 * 1 *"), "Cron 0 8 * 1 *");
    }
}
