//! When a routine runs: a cron schedule, once at a date and time
//! (`@once 2026-11-20 14:00`), or each time Herdr starts (`@start`).

use crate::cron::Cron;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone};

pub const AT_START: &str = "@start";
const ONCE: &str = "@once";

#[derive(Debug, Clone)]
pub enum Schedule {
    Cron(Cron),
    Once(DateTime<Local>),
    AtStart,
}

impl Schedule {
    pub fn parse(text: &str) -> Result<Schedule, String> {
        let t = text.trim();
        if t == AT_START {
            return Ok(Schedule::AtStart);
        }
        if let Some(rest) = t.strip_prefix(ONCE) {
            let naive = NaiveDateTime::parse_from_str(rest.trim(), "%Y-%m-%d %H:%M")
                .map_err(|_| "a single run is \"@once YYYY-MM-DD HH:MM\"".to_string())?;
            let local = Local
                .from_local_datetime(&naive)
                .earliest()
                .ok_or("that time does not exist here (clock change)")?;
            return Ok(Schedule::Once(local));
        }
        Cron::parse(t).map(Schedule::Cron)
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
            Schedule::AtStart => None,
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
}
