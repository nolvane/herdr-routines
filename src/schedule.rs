//! When a routine runs: a cron schedule, once at a date and time
//! (`@once 2026-11-20 14:00`), every so many days or weeks from a first
//! time (`@every 2 weeks from 2026-11-09 09:00`), or each time Herdr starts
//! (`@start`).

use crate::cron::Cron;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDateTime, TimeZone};

pub const AT_START: &str = "@start";
const ONCE: &str = "@once";
const EVERY: &str = "@every";
const DATE_TIME: &str = "%Y-%m-%d %H:%M";

#[derive(Debug, Clone)]
pub enum Schedule {
    Cron(Cron),
    Once(DateTime<Local>),
    Every(Every),
    AtStart,
}

/// Every so many days, counted from a first date and time. Cron cannot say
/// "every two weeks": its fields know nothing of the week before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Every {
    /// Days between two runs (14 for every two weeks).
    pub days: u32,
    /// The first run, in local time.
    pub start: NaiveDateTime,
}

impl Every {
    fn parse(rest: &str) -> Result<Every, String> {
        let shape = "a repeating run is \"@every 2 weeks from YYYY-MM-DD HH:MM\" (or days)";
        let (span, start) = rest.split_once(" from ").ok_or(shape)?;
        let mut words = span.split_whitespace();
        let n: u32 = words.next().and_then(|n| n.parse().ok()).ok_or(shape)?;
        let per = match words.next().map(str::to_ascii_lowercase).as_deref() {
            Some("day" | "days") => 1,
            Some("week" | "weeks") => 7,
            _ => return Err(shape.into()),
        };
        if words.next().is_some() {
            return Err(shape.into());
        }
        let days = n.checked_mul(per).filter(|d| (1..=366).contains(d));
        let days = days.ok_or("every 1 to 366 days, or 1 to 52 weeks")?;
        let start = NaiveDateTime::parse_from_str(start.trim(), DATE_TIME).map_err(|_| shape)?;
        Ok(Every { days, start })
    }

    /// The text for `routine.toml`.
    pub fn text(&self) -> String {
        let span = if self.days % 7 == 0 {
            format!("{} weeks", self.days / 7)
        } else {
            format!("{} days", self.days)
        };
        format!("{EVERY} {span} from {}", self.start.format(DATE_TIME))
    }

    /// The run number `k` (0 is the first), in local time. A time the
    /// clock change skips moves an hour later.
    fn at(&self, k: i64) -> Option<DateTime<Local>> {
        let naive = self
            .start
            .checked_add_signed(Duration::days(k.checked_mul(i64::from(self.days))?))?;
        Local.from_local_datetime(&naive).earliest().or_else(|| {
            Local
                .from_local_datetime(&(naive + Duration::hours(1)))
                .earliest()
        })
    }

    fn next_after(&self, after: &DateTime<Local>) -> Option<DateTime<Local>> {
        let gap = (after.naive_local().date() - self.start.date()).num_days();
        let first = gap.div_euclid(i64::from(self.days)).max(0);
        (first..first + 3)
            .filter_map(|k| self.at(k))
            .find(|t| t > after)
    }

    fn describe(&self) -> String {
        let time = self.start.format("%H:%M");
        if self.days % 7 == 0 {
            let day = self.start.format("%A");
            match self.days / 7 {
                1 => format!("{day}s at {time}"),
                n => format!("Every {n} weeks on {day} at {time}"),
            }
        } else if self.days == 1 {
            format!("Daily at {time}")
        } else {
            format!("Every {} days at {time}", self.days)
        }
    }
}

/// Cron's own short forms, as five fields.
fn short_form(text: &str) -> Option<&'static str> {
    Some(match text {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        _ => return None,
    })
}

impl Schedule {
    pub fn parse(text: &str) -> Result<Schedule, String> {
        let t = text.trim();
        if t == AT_START {
            return Ok(Schedule::AtStart);
        }
        if let Some(rest) = t.strip_prefix(ONCE) {
            let naive = NaiveDateTime::parse_from_str(rest.trim(), DATE_TIME)
                .map_err(|_| "a single run is \"@once YYYY-MM-DD HH:MM\"".to_string())?;
            let local = Local
                .from_local_datetime(&naive)
                .earliest()
                .ok_or("that time does not exist here (clock change)")?;
            return Ok(Schedule::Once(local));
        }
        if let Some(rest) = t.strip_prefix(EVERY) {
            return Every::parse(rest.trim()).map(Schedule::Every);
        }
        Cron::parse(short_form(t).unwrap_or(t)).map(Schedule::Cron)
    }

    /// The text for `routine.toml` of a single run.
    pub fn once_text(date: &str, time: &str) -> String {
        format!("{ONCE} {} {}", date.trim(), time.trim())
    }

    /// The next time after `after`; None for "at start" and a past single run.
    pub fn next_after(&self, after: &DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            Schedule::Cron(c) => c.next_after(after),
            Schedule::Once(t) => (t > after).then_some(*t),
            Schedule::Every(e) => e.next_after(after),
            Schedule::AtStart => None,
        }
    }

    /// The latest time in `(after, until]`.
    pub fn latest_in(
        &self,
        after: &DateTime<Local>,
        until: &DateTime<Local>,
    ) -> Option<DateTime<Local>> {
        match self {
            Schedule::Cron(c) => c.latest_in(after, until),
            Schedule::Once(t) => (t > after && t <= until).then_some(*t),
            Schedule::Every(e) => {
                let mut found = None;
                let mut from = *after;
                while let Some(next) = e.next_after(&from).filter(|t| t <= until) {
                    found = Some(next);
                    from = next;
                }
                found
            }
            Schedule::AtStart => None,
        }
    }

    /// The next `count` times after `after`, for a preview.
    pub fn upcoming(&self, after: &DateTime<Local>, count: usize) -> Vec<DateTime<Local>> {
        let mut times = Vec::new();
        let mut from = *after;
        while times.len() < count {
            let Some(next) = self.next_after(&from) else {
                break;
            };
            times.push(next);
            from = next;
        }
        times
    }

    /// Whether the system's cron can run this schedule itself.
    pub fn classic(&self) -> bool {
        match self {
            Schedule::Cron(c) => c.classic(),
            Schedule::Once(_) | Schedule::AtStart => true,
            Schedule::Every(_) => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Schedule::Cron(c) => c.describe(),
            // Short enough for the list's schedule column.
            Schedule::Once(t) if t.year() == Local::now().year() => {
                format!("Once on {}", t.format("%-d %b at %H:%M"))
            }
            Schedule::Once(t) => format!("Once on {}", t.format("%-d %b %Y %H:%M")),
            Schedule::Every(e) => e.describe(),
            Schedule::AtStart => "When Herdr starts".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_runs_and_start() {
        let s = Schedule::parse("@once 2026-11-20 14:00").unwrap();
        let before = Local.with_ymd_and_hms(2026, 11, 20, 13, 0, 0).unwrap();
        let at = Local.with_ymd_and_hms(2026, 11, 20, 14, 0, 0).unwrap();
        assert_eq!(s.next_after(&before), Some(at));
        assert_eq!(s.next_after(&at), None);
        assert_eq!(s.latest_in(&before, &at), Some(at));
        let words = s.describe();
        assert!(
            words == "Once on 20 Nov at 14:00" || words == "Once on 20 Nov 2026 14:00",
            "{words}"
        );
        assert!(Schedule::parse("@once 2026-11-20").is_err());
        let start = Schedule::parse("@start").unwrap();
        assert_eq!(start.next_after(&before), None);
        assert_eq!(start.describe(), "When Herdr starts");
        assert!(matches!(
            Schedule::parse("0 9 * * *"),
            Ok(Schedule::Cron(_))
        ));
    }

    #[test]
    fn every_two_weeks_from_a_first_time() {
        // Monday 9 Nov 2026, then every second Monday.
        let s = Schedule::parse("@every 2 weeks from 2026-11-09 09:00").unwrap();
        let t = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap();
        // Before the first time: the first time itself.
        assert_eq!(s.next_after(&t(2, 8)), Some(t(9, 9)));
        assert_eq!(s.next_after(&t(9, 8)), Some(t(9, 9)));
        assert_eq!(s.next_after(&t(9, 9)), Some(t(23, 9)));
        assert_eq!(s.next_after(&t(16, 12)), Some(t(23, 9)));
        assert_eq!(s.latest_in(&t(2, 0), &t(24, 0)), Some(t(23, 9)));
        assert_eq!(s.latest_in(&t(10, 0), &t(20, 0)), None);
        assert_eq!(s.upcoming(&t(2, 8), 2), vec![t(9, 9), t(23, 9)]);
        assert_eq!(s.describe(), "Every 2 weeks on Monday at 09:00");
        assert!(!s.classic());

        let days = Schedule::parse("@every 3 days from 2026-11-09 07:30").unwrap();
        assert_eq!(days.describe(), "Every 3 days at 07:30");
        let Schedule::Every(e) = &days else {
            panic!("an interval");
        };
        assert_eq!(e.text(), "@every 3 days from 2026-11-09 07:30");
        let weekly = Schedule::parse("@every 1 week from 2026-11-09 09:00").unwrap();
        assert_eq!(weekly.describe(), "Mondays at 09:00");

        for bad in [
            "@every",
            "@every 2 weeks",
            "@every two weeks from 2026-11-09 09:00",
            "@every 0 days from 2026-11-09 09:00",
            "@every 60 weeks from 2026-11-09 09:00",
            "@every 2 months from 2026-11-09 09:00",
            "@every 2 weeks from 2026-11-09",
            "@every 4294967295 weeks from 2026-11-09 09:00",
        ] {
            assert!(Schedule::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn crons_short_forms() {
        let words = |s: &str| Schedule::parse(s).unwrap().describe();
        assert_eq!(words("@hourly"), "Every hour at :00");
        assert_eq!(words("@daily"), "Daily at 00:00");
        assert_eq!(words("@weekly"), "Sundays at 00:00");
        assert_eq!(words("@monthly"), "Monthly on the 1st at 00:00");
        assert!(Schedule::parse("@yearly").is_ok());
        assert!(Schedule::parse("@sometimes").is_err());
        assert!(Schedule::parse("@daily").unwrap().classic());
        assert!(!Schedule::parse("0 9 L * *").unwrap().classic());
    }
}
