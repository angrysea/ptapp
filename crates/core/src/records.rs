//! Personal records (LG-5): the heaviest weight lifted for 1-10 reps, and the
//! best estimated 1RM, per exercise. Found when a log is saved; the first
//! time an exercise is logged sets a baseline, which isn't celebrated.

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::model::WorkoutLog;
use crate::stats::e1rm;

/// Rep counts that get their own record.
pub const MAX_RECORD_REPS: u32 = 10;
/// Sets above this many reps don't count toward the e1RM record (the
/// formula is unreliable there; see stats::strength_trends).
const MAX_E1RM_REPS: u32 = 12;
/// Smaller gains than this (rounding noise between lb and kg) aren't records.
const MIN_GAIN_KG: f64 = 0.05;

/// The record before the current one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Previous {
    pub kg: f64,
    pub date: NaiveDate,
}

/// `CLIENT#{id}` / `PR#{key}`, where the key is `{exerciseId}#R05` for a
/// 5-rep max or `{exerciseId}#E1RM`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonalRecord {
    pub key: String,
    pub exercise_id: String,
    pub exercise_name: String,
    /// Reps for a rep-max; None for the e1RM record.
    pub reps: Option<u32>,
    /// The weight lifted (rep-max) or the estimated 1RM.
    pub kg: f64,
    /// The set behind an e1RM record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<(f64, u32)>,
    pub date: NaiveDate,
    pub log_id: String,
    /// The first time this was recorded: a baseline, not a new best.
    pub first: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Previous>,
}

pub fn rep_key(exercise_id: &str, reps: u32) -> String {
    format!("{exercise_id}#R{reps:02}")
}

pub fn e1rm_key(exercise_id: &str) -> String {
    format!("{exercise_id}#E1RM")
}

/// The records `log` sets, given the client's current ones by key. Records
/// this same log set earlier (it is being re-saved) are judged against what
/// they replaced, so editing a log doesn't count a record twice.
pub fn new_records(log: &WorkoutLog, current: &HashMap<String, PersonalRecord>) -> Vec<PersonalRecord> {
    if !log.status.counts_as_done() {
        return Vec::new();
    }
    // The best candidate per record key within this log.
    let mut best: HashMap<String, PersonalRecord> = HashMap::new();
    let mut offer = |rec: PersonalRecord| {
        if best.get(&rec.key).is_none_or(|b| rec.kg > b.kg) {
            best.insert(rec.key.clone(), rec);
        }
    };
    for s in log.sets.iter().filter(|s| s.done) {
        let (Some(reps), Some(kg)) = (s.reps, s.load_kg) else { continue };
        if reps == 0 || kg <= 0.0 {
            continue;
        }
        let base = PersonalRecord {
            key: String::new(),
            exercise_id: s.exercise_id.clone(),
            exercise_name: s.exercise_name.clone(),
            reps: None,
            kg,
            set: None,
            date: log.date,
            log_id: log.id.clone(),
            first: false,
            previous: None,
        };
        if reps <= MAX_RECORD_REPS {
            offer(PersonalRecord { key: rep_key(&s.exercise_id, reps), reps: Some(reps), ..base.clone() });
        }
        if reps <= MAX_E1RM_REPS {
            let est = (e1rm(kg, reps) * 10.0).round() / 10.0;
            offer(PersonalRecord { key: e1rm_key(&s.exercise_id), kg: est, set: Some((kg, reps)), ..base });
        }
    }

    let mut out: Vec<PersonalRecord> = best
        .into_values()
        .filter_map(|mut rec| {
            let baseline = match current.get(&rec.key) {
                None => None,
                // Set by this log before: compare with what it replaced.
                Some(c) if c.log_id == log.id => {
                    if (c.kg - rec.kg).abs() < MIN_GAIN_KG {
                        return None; // unchanged
                    }
                    if c.first {
                        rec.first = true;
                        return Some(rec);
                    }
                    c.previous
                }
                Some(c) => Some(Previous { kg: c.kg, date: c.date }),
            };
            match baseline {
                None => {
                    rec.first = true;
                    Some(rec)
                }
                Some(b) if rec.kg >= b.kg + MIN_GAIN_KG => {
                    rec.previous = Some(b);
                    Some(rec)
                }
                Some(_) => None,
            }
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LogStatus, SetLog};
    use std::collections::BTreeMap;

    fn log(id: &str, day: u32, sets: &[(&str, u32, f64, bool)]) -> WorkoutLog {
        WorkoutLog {
            id: id.into(),
            scheduled_id: None,
            date: NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
            workout_name: "A".into(),
            status: LogStatus::Completed,
            sets: sets
                .iter()
                .enumerate()
                .map(|(i, (ex, reps, kg, done))| SetLog {
                    item_id: "i".into(),
                    exercise_id: (*ex).into(),
                    exercise_name: (*ex).into(),
                    set_index: i as u32,
                    reps: Some(*reps),
                    load_kg: Some(*kg),
                    rpe: None,
                    done: *done,
                })
                .collect(),
            comment: String::new(),
            check: None,
            exercise_notes: BTreeMap::new(),
            rating: None,
            updated_at: chrono::Utc::now(),
        }
    }

    fn by_key(recs: Vec<PersonalRecord>) -> HashMap<String, PersonalRecord> {
        recs.into_iter().map(|r| (r.key.clone(), r)).collect()
    }

    #[test]
    fn first_log_sets_baselines_then_heavier_sets_are_records() {
        let first = new_records(&log("a", 1, &[("squat", 5, 100.0, true), ("squat", 5, 110.0, true), ("squat", 20, 60.0, true)]), &HashMap::new());
        let keys: Vec<&str> = first.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["squat#E1RM", "squat#R05"], "20 reps sets neither record");
        assert!(first.iter().all(|r| r.first));
        assert_eq!(first[1].kg, 110.0, "the best set in the log");
        let current = by_key(first);

        let next = new_records(&log("b", 8, &[("squat", 5, 112.5, true), ("squat", 3, 120.0, false)]), &current);
        let r5 = next.iter().find(|r| r.key == "squat#R05").unwrap();
        assert!(!r5.first);
        assert_eq!(r5.previous, Some(Previous { kg: 110.0, date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap() }));
        assert!(next.iter().all(|r| r.reps != Some(3)), "a set not marked done doesn't count");
        assert!(new_records(&log("c", 9, &[("squat", 5, 110.0, true)]), &current).is_empty(), "matching isn't beating");
    }

    #[test]
    fn re_saving_a_log_is_judged_against_what_it_replaced() {
        let current = by_key(new_records(&log("a", 1, &[("bench", 5, 80.0, true)]), &HashMap::new()));
        let current = {
            let mut c = current;
            c.extend(by_key(new_records(&log("b", 8, &[("bench", 5, 85.0, true)]), &c)));
            c
        };
        // log b re-saved unchanged: nothing new
        assert!(new_records(&log("b", 8, &[("bench", 5, 85.0, true)]), &current).is_empty());
        // log b edited to 90: still a record over the 80 it replaced
        let edited = new_records(&log("b", 8, &[("bench", 5, 90.0, true)]), &current);
        let r5 = edited.iter().find(|r| r.key == "bench#R05").unwrap();
        assert_eq!(r5.previous.unwrap().kg, 80.0);
    }

    #[test]
    fn skipped_logs_set_nothing() {
        let mut l = log("a", 1, &[("squat", 5, 100.0, true)]);
        l.status = LogStatus::Skipped;
        assert!(new_records(&l, &HashMap::new()).is_empty());
    }
}
