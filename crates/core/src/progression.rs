//! Progression rules (WB-6). A program item can carry a rule such as "when
//! every set is done at RPE 8 or lower, add 2.5 kg". When the client logs
//! the workout, a met rule becomes a *suggestion* for the next workout with
//! that exercise; the coach accepts or dismisses it, nothing changes on its
//! own. A pain flag on the log blocks suggestions (BRD risk).

use serde::{Deserialize, Serialize};

use crate::model::{Item, WorkoutLog};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionRule {
    /// Every done set must be at or below this RPE (and have one recorded).
    /// None: hitting the reps is enough.
    #[serde(default)]
    pub max_rpe: Option<f64>,
    /// Add this much load...
    #[serde(default)]
    pub add_kg: Option<f64>,
    /// ...or this many reps (for a plain number of reps, like "8").
    #[serde(default)]
    pub add_reps: Option<u32>,
    /// Never suggest a load above this.
    #[serde(default)]
    pub cap_kg: Option<f64>,
}

impl ProgressionRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.add_kg.is_none() && self.add_reps.is_none() {
            return Err("a progression rule needs a load or reps to add".into());
        }
        if self.add_kg.is_some_and(|k| !(k > 0.0 && k <= 50.0)) {
            return Err("progression load step must be above 0 and at most 50 kg".into());
        }
        if self.add_reps.is_some_and(|r| !(1..=5).contains(&r)) {
            return Err("progression rep step must be 1-5".into());
        }
        if self.max_rpe.is_some_and(|r| !(1.0..=10.0).contains(&r)) {
            return Err("progression RPE must be 1-10".into());
        }
        if self.cap_kg.is_some_and(|c| c <= 0.0) {
            return Err("progression cap must be above 0".into());
        }
        Ok(())
    }
}

/// What the client did with an item, when it met the item's rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Earned {
    /// The heaviest load used across the item's sets.
    pub top_kg: Option<f64>,
    pub why: String,
}

/// A change to suggest for the next workout's item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_kg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reps: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionStatus {
    Pending,
    Accepted,
    Dismissed,
    /// The workout it was for got logged first.
    Expired,
}

/// A suggested change for a coach to accept or dismiss:
/// `COACH#{coach}` / `SUGGEST#{id}`, id = `{logId}.{itemId}` so re-saving
/// the log never suggests the same thing twice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub exercise_id: String,
    pub exercise_name: String,
    pub log_id: String,
    pub log_date: chrono::NaiveDate,
    pub why: String,
    /// The next workout with this exercise, and its item.
    pub target_sched_id: String,
    pub target_date: chrono::NaiveDate,
    pub target_item_id: String,
    pub current: Change,
    pub proposed: Change,
    pub status: SuggestionStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn plain_reps(reps: &str) -> Option<u32> {
    reps.trim().parse().ok()
}

/// Whether the client's sets for `item` in `log` met its rule.
pub fn earned(item: &Item, log: &WorkoutLog) -> Option<Earned> {
    let rule = item.progression.as_ref()?;
    if log.check.as_ref().is_some_and(|c| c.pain) {
        return None;
    }
    let sets: Vec<_> = log.sets.iter().filter(|s| s.item_id == item.id).collect();
    if sets.is_empty() || (sets.len() as u32) < item.sets || !sets.iter().all(|s| s.done) {
        return None;
    }
    // The target is the lowest number in the prescription ("8-10" needs 8).
    let target = item.reps.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse::<u32>().ok()).min()?;
    if !sets.iter().all(|s| s.reps.is_some_and(|r| r >= target)) {
        return None;
    }
    if let Some(max) = rule.max_rpe
        && !sets.iter().all(|s| s.rpe.is_some_and(|r| r <= max))
    {
        return None;
    }
    let top_kg = sets.iter().filter_map(|s| s.load_kg).fold(None, |m: Option<f64>, k| Some(m.map_or(k, |m| m.max(k))));
    let why = match rule.max_rpe {
        Some(max) => format!("all {} sets of {target}+ reps at RPE {max} or lower", sets.len()),
        None => format!("all {} sets of {target}+ reps done", sets.len()),
    };
    Some(Earned { top_kg, why })
}

/// The change `rule` makes to the next workout's `target` item, given what
/// was earned; None if it would change nothing (e.g. already at the cap).
pub fn propose(rule: &ProgressionRule, earned: &Earned, target: &Item) -> Option<Change> {
    let mut change = Change { load_kg: None, reps: None };
    if let (Some(step), Some(top)) = (rule.add_kg, earned.top_kg) {
        let mut next = ((top + step) * 1000.0).round() / 1000.0;
        if let Some(cap) = rule.cap_kg {
            next = next.min(cap);
        }
        if target.load_kg.is_none_or(|cur| next > cur + 1e-6) {
            change.load_kg = Some(next);
        }
    }
    if let Some(step) = rule.add_reps
        && let Some(reps) = plain_reps(&target.reps)
    {
        change.reps = Some((reps + step).to_string());
    }
    (change.load_kg.is_some() || change.reps.is_some()).then_some(change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LogStatus, PostCheck, SetLog};
    use chrono::NaiveDate;
    use std::collections::BTreeMap;

    fn item(reps: &str, rule: ProgressionRule) -> Item {
        Item {
            id: "sq".into(),
            exercise_id: "squat".into(),
            exercise_name: "Squat".into(),
            sets: 3,
            reps: reps.into(),
            load_kg: Some(100.0),
            load_pct: None,
            progression: Some(rule),
            tempo: None,
            rest_sec: None,
            rpe: None,
            rir: None,
            cardio: None,
            notes: String::new(),
        }
    }

    fn rule(max_rpe: Option<f64>) -> ProgressionRule {
        ProgressionRule { max_rpe, add_kg: Some(2.5), add_reps: None, cap_kg: Some(104.0) }
    }

    fn log(sets: &[(u32, f64, Option<f64>, bool)], pain: bool) -> WorkoutLog {
        WorkoutLog {
            id: "l".into(),
            scheduled_id: Some("s".into()),
            date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            workout_name: "A".into(),
            status: LogStatus::Completed,
            sets: sets
                .iter()
                .enumerate()
                .map(|(i, (reps, kg, rpe, done))| SetLog {
                    item_id: "sq".into(),
                    exercise_id: "squat".into(),
                    exercise_name: "Squat".into(),
                    set_index: i as u32,
                    reps: Some(*reps),
                    load_kg: Some(*kg),
                    rpe: *rpe,
                    done: *done,
                    ..Default::default()
                })
                .collect(),
            comment: String::new(),
            check: Some(PostCheck { pain, ..Default::default() }),
            exercise_notes: BTreeMap::new(),
            rating: None,
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn earned_needs_every_set_at_target_and_rpe() {
        let it = item("5", rule(Some(8.0)));
        let good = log(&[(5, 100.0, Some(7.0), true), (5, 100.0, Some(8.0), true), (6, 100.0, Some(8.0), true)], false);
        assert_eq!(earned(&it, &good).unwrap().top_kg, Some(100.0));
        assert!(earned(&it, &log(&[(5, 100.0, Some(7.0), true), (4, 100.0, Some(8.0), true), (5, 100.0, Some(8.0), true)], false)).is_none(), "a short set");
        assert!(earned(&it, &log(&[(5, 100.0, Some(7.0), true), (5, 100.0, Some(9.0), true), (5, 100.0, Some(8.0), true)], false)).is_none(), "RPE too high");
        assert!(earned(&it, &log(&[(5, 100.0, None, true), (5, 100.0, Some(8.0), true), (5, 100.0, Some(8.0), true)], false)).is_none(), "RPE not recorded");
        assert!(earned(&it, &log(&[(5, 100.0, Some(7.0), true), (5, 100.0, Some(7.0), true)], false)).is_none(), "a set missing");
        assert!(earned(&it, &log(&[(5, 100.0, Some(7.0), true), (5, 100.0, Some(7.0), true), (5, 100.0, Some(7.0), false)], false)).is_none(), "a set not done");
        assert!(earned(&it, &good.clone()).is_some());
        assert!(earned(&it, &log(&[(5, 100.0, Some(7.0), true), (5, 100.0, Some(8.0), true), (5, 100.0, Some(8.0), true)], true)).is_none(), "pain blocks it");
        // a rep range needs its bottom number; no RPE rule means reps alone
        let range = item("8-10", rule(None));
        assert!(earned(&range, &log(&[(8, 60.0, None, true), (9, 60.0, None, true), (8, 60.0, None, true)], false)).is_some());
    }

    #[test]
    fn propose_adds_load_up_to_the_cap_or_reps() {
        let r = rule(None);
        let e = Earned { top_kg: Some(100.0), why: String::new() };
        let next = item("5", r.clone());
        assert_eq!(propose(&r, &e, &next), Some(Change { load_kg: Some(102.5), reps: None }));
        let at_cap = Earned { top_kg: Some(103.0), why: String::new() };
        assert_eq!(propose(&r, &at_cap, &next).unwrap().load_kg, Some(104.0), "capped");
        let heavier_already = Item { load_kg: Some(110.0), ..next.clone() };
        assert_eq!(propose(&r, &e, &heavier_already), None, "the next workout is already heavier");
        let reps_rule = ProgressionRule { add_kg: None, add_reps: Some(1), ..r };
        assert_eq!(propose(&reps_rule, &e, &item("8", reps_rule.clone())), Some(Change { load_kg: None, reps: Some("9".into()) }));
        assert_eq!(propose(&reps_rule, &e, &item("8-10", reps_rule.clone())), None, "a range isn't bumped");
    }

    #[test]
    fn validates() {
        assert!(rule(Some(8.0)).validate().is_ok());
        assert!(ProgressionRule { add_kg: None, add_reps: None, max_rpe: None, cap_kg: None }.validate().is_err());
        assert!(ProgressionRule { add_kg: Some(0.0), ..rule(None) }.validate().is_err());
        assert!(ProgressionRule { add_reps: Some(9), add_kg: None, ..rule(None) }.validate().is_err());
    }
}
