//! Automated messages and reminders (MS-4, FB-3 reminders). A coach sets
//! when clients are reminded to check in or train, and messages that go out
//! on a schedule. An hourly sweep (api::sweep) sends whatever is due in each
//! client's own time zone, once per day per item.

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};

pub const MAX_MESSAGES: usize = 20;
const MAX_TEXT: usize = 1000;
/// Something due at 8:00 still goes out if the sweep runs late, until 10:59;
/// a rule saved in the afternoon waits for the next day instead of firing at once.
pub const WINDOW_HOURS: u32 = 3;

/// Days of the week as 0 = Monday … 6 = Sunday; empty means every day.
pub type Days = Vec<u8>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct When {
    #[serde(default)]
    pub days: Days,
    /// Local hour, 0-23.
    pub hour: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutoMessage {
    pub id: String,
    /// `{name}` becomes the client's first name.
    pub text: String,
    pub when: When,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

/// `COACH#{coach}` / `AUTOMATION`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    /// Remind clients who haven't sent this week's check-in.
    #[serde(default)]
    pub checkin_reminder: Option<When>,
    /// Remind clients of a workout scheduled today that they haven't logged.
    #[serde(default)]
    pub workout_reminder: Option<When>,
    #[serde(default)]
    pub messages: Vec<AutoMessage>,
}

impl Default for Automation {
    /// Check-in reminders on Sunday at 17:00; everything else off.
    fn default() -> Self {
        Automation { checkin_reminder: Some(When { days: vec![6], hour: 17 }), workout_reminder: None, messages: Vec::new() }
    }
}

impl When {
    fn validate(&self) -> Result<(), String> {
        if self.hour > 23 {
            return Err("hours are 0-23".into());
        }
        if self.days.iter().any(|d| *d > 6) {
            return Err("days are 0 (Monday) to 6 (Sunday)".into());
        }
        Ok(())
    }

    /// Whether this is due at the client's local time `local`.
    pub fn due(&self, local: NaiveDateTime) -> bool {
        let day = local.weekday().num_days_from_monday() as u8;
        (self.days.is_empty() || self.days.contains(&day)) && (self.hour..self.hour + WINDOW_HOURS).contains(&local.hour())
    }
}

impl Automation {
    pub fn validate(&mut self) -> Result<(), String> {
        for w in [&self.checkin_reminder, &self.workout_reminder].into_iter().flatten() {
            w.validate()?;
        }
        if self.messages.len() > MAX_MESSAGES {
            return Err(format!("up to {MAX_MESSAGES} automated messages"));
        }
        for m in &mut self.messages {
            m.text = m.text.trim().to_string();
            if m.text.is_empty() || m.text.chars().count() > MAX_TEXT {
                return Err(format!("automated messages must be 1-{MAX_TEXT} characters"));
            }
            if m.id.is_empty() || m.id.len() > 40 {
                return Err("each automated message needs an id".into());
            }
            m.when.days.sort_unstable();
            m.when.days.dedup();
            m.when.validate()?;
        }
        Ok(())
    }
}

/// `now` on the client's clock (UTC if their time zone is unknown).
pub fn local_time(now: DateTime<Utc>, time_zone: Option<&str>) -> NaiveDateTime {
    match time_zone.and_then(|tz| tz.parse::<chrono_tz::Tz>().ok()) {
        Some(tz) => now.with_timezone(&tz).naive_local(),
        None => now.naive_utc(),
    }
}

/// `text` with `{name}` filled in.
pub fn personalize(text: &str, first_name: &str) -> String {
    text.replace("{name}", if first_name.is_empty() { "there" } else { first_name })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    fn at(day: u32, hour: u32) -> NaiveDateTime {
        // October 2026: the 26th is a Monday
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(hour, 5, 0).unwrap()
    }

    #[test]
    fn due_on_its_days_within_the_window() {
        let sunday_5pm = When { days: vec![6], hour: 17 };
        assert!(!sunday_5pm.due(at(29, 17)), "the 29th is a Thursday");
        assert!(sunday_5pm.due(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(17, 5, 0).unwrap()));
        assert!(sunday_5pm.due(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(19, 59, 0).unwrap()), "a late sweep still sends");
        assert!(!sunday_5pm.due(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(20, 0, 0).unwrap()));
        assert!(!sunday_5pm.due(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(16, 59, 0).unwrap()));
        let daily_8 = When { days: vec![], hour: 8 };
        assert!(daily_8.due(at(26, 8)) && daily_8.due(at(27, 9)) && !daily_8.due(at(28, 11)));
        let monday = When { days: vec![0], hour: 7 };
        assert!(monday.due(at(26, 7)) && !monday.due(at(27, 7)));
    }

    #[test]
    fn local_time_uses_the_client_zone() {
        let now = Utc.with_ymd_and_hms(2026, 10, 28, 15, 0, 0).unwrap();
        assert_eq!(local_time(now, Some("America/Chicago")).hour(), 10);
        assert_eq!(local_time(now, Some("not/a-zone")).hour(), 15);
        assert_eq!(local_time(now, None).hour(), 15);
    }

    #[test]
    fn validates_and_tidies() {
        let mut a = Automation::default();
        a.messages.push(AutoMessage { id: "m1".into(), text: "  New week, {name}!  ".into(), when: When { days: vec![2, 0, 2], hour: 8 }, active: true });
        assert!(a.validate().is_ok());
        assert_eq!(a.messages[0].text, "New week, {name}!");
        assert_eq!(a.messages[0].when.days, vec![0, 2]);
        assert_eq!(personalize(&a.messages[0].text, "Sam"), "New week, Sam!");
        let mut bad = Automation { workout_reminder: Some(When { days: vec![], hour: 24 }), ..Automation::default() };
        assert!(bad.validate().is_err());
        let mut bad = Automation::default();
        bad.messages.push(AutoMessage { id: "m".into(), text: " ".into(), when: When { days: vec![7], hour: 8 }, active: true });
        assert!(bad.validate().is_err());
    }
}
