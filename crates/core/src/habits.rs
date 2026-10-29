//! Daily habits (BZ-3): a yes/no habit ("Take creatine") or a count with a
//! target ("8 glasses of water", "10000 steps"). The client logs each day's
//! value from Today; a streak is the run of days the target was met.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

pub const MAX_HABITS: usize = 12;
/// How far back a day's value can still be entered or changed.
pub const EDITABLE_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HabitKind {
    /// Done or not (value 1 or 0).
    Check,
    /// A number toward `target`.
    Count,
}

/// `CLIENT#{id}` / `HABIT#{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Habit {
    pub id: String,
    pub name: String,
    pub kind: HabitKind,
    /// The count that meets the habit for a day (1 for a check).
    pub target: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Archived habits keep their history but leave Today.
    #[serde(default = "yes")]
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

fn yes() -> bool {
    true
}

/// `CLIENT#{id}` / `HABITLOG#{date}#{habitId}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HabitLog {
    pub habit_id: String,
    pub date: NaiveDate,
    pub value: f64,
    pub updated_at: DateTime<Utc>,
}

impl Habit {
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > 60 {
            return Err("a habit's name must be 1-60 characters".into());
        }
        self.unit = self.unit.take().map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        if self.unit.as_ref().is_some_and(|u| u.chars().count() > 20) {
            return Err("units must be at most 20 characters".into());
        }
        match self.kind {
            HabitKind::Check => self.target = 1.0,
            HabitKind::Count if !(self.target > 0.0 && self.target <= 1_000_000.0) => return Err("a count needs a target above 0".into()),
            HabitKind::Count => {}
        }
        Ok(())
    }

    pub fn met(&self, value: f64) -> bool {
        value >= self.target - 1e-9
    }
}

/// The current streak: days in a row the habit was met, ending today, or
/// ending yesterday while today isn't met yet (the day isn't over).
pub fn streak(habit: &Habit, logs: &[HabitLog], today: NaiveDate) -> u32 {
    let by_day: HashMap<NaiveDate, f64> = logs.iter().filter(|l| l.habit_id == habit.id).map(|l| (l.date, l.value)).collect();
    let met = |d: NaiveDate| by_day.get(&d).is_some_and(|v| habit.met(*v));
    let mut day = if met(today) { today } else { today - Duration::days(1) };
    let mut n = 0;
    while met(day) {
        n += 1;
        day -= Duration::days(1);
    }
    n
}

/// Whether the client may set a value for `date`: today or the week before.
pub fn editable(date: NaiveDate, today: NaiveDate) -> bool {
    date <= today && date > today - Duration::days(EDITABLE_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn habit(kind: HabitKind, target: f64) -> Habit {
        Habit { id: "h".into(), name: "Water".into(), kind, target, unit: Some("glasses".into()), active: true, created_at: Utc::now() }
    }

    fn log(day: u32, value: f64) -> HabitLog {
        HabitLog { habit_id: "h".into(), date: d(day), value, updated_at: Utc::now() }
    }

    #[test]
    fn streaks_run_back_from_today_or_yesterday() {
        let water = habit(HabitKind::Count, 8.0);
        let logs = vec![log(24, 8.0), log(25, 9.0), log(26, 8.0), log(27, 3.0)];
        assert_eq!(streak(&water, &logs, d(26)), 3, "met today");
        assert_eq!(streak(&water, &logs, d(27)), 3, "today not met yet: yesterday's run still counts");
        assert_eq!(streak(&water, &logs, d(28)), 0, "yesterday was missed");
        assert_eq!(streak(&water, &[], d(28)), 0);
    }

    #[test]
    fn validation_tidies_and_checks() {
        let mut c = habit(HabitKind::Check, 5.0);
        c.name = "  Creatine ".into();
        assert!(c.validate().is_ok());
        assert_eq!((c.name.as_str(), c.target), ("Creatine", 1.0));
        assert!(habit(HabitKind::Count, 0.0).validate().is_err());
        assert!(editable(d(28), d(28)) && editable(d(22), d(28)) && !editable(d(21), d(28)) && !editable(d(29), d(28)));
    }
}
