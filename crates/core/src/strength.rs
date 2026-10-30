//! Loads prescribed as a percentage of a client's 1RM (WB-5). A program item
//! may carry `load_pct` instead of a weight; assigning the program resolves it
//! against the client's recorded max for that exercise, rounded to the plates
//! they load in their own units. The weight is stored in kg like every load.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Role, UnitSystem, Workout, WorkoutLog};
use crate::stats::strength_trends;

pub const KG_PER_LB: f64 = 0.453_592_37;

/// Highest %1RM a program may prescribe (supramaximal eccentrics, holds).
pub const MAX_PCT: f64 = 150.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MaxSource {
    /// Lifted for one rep (a test day).
    Tested,
    /// Taken from a logged set's e1RM.
    Estimated,
}

/// A client's 1RM for one exercise: `CLIENT#{id}` / `MAX#{exerciseId}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Max {
    pub exercise_id: String,
    pub exercise_name: String,
    pub kg: f64,
    pub source: MaxSource,
    pub date: NaiveDate,
    pub set_by: Role,
    pub updated_at: DateTime<Utc>,
}

/// Rounds a load to the smallest jump the client can make: 5 lb (2.5 lb
/// plates a side) in imperial, 2.5 kg (1.25 kg a side) in metric.
pub fn round_to_plates(kg: f64, units: UnitSystem) -> f64 {
    match units {
        UnitSystem::Imperial => (kg / KG_PER_LB / 5.0).round() * 5.0 * KG_PER_LB,
        UnitSystem::Metric => (kg / 2.5).round() * 2.5,
    }
}

/// `pct` percent of `max_kg`, rounded to plates. The max is first taken in
/// the client's unit to 0.1, as they entered it: a 225 lb max is stored as
/// 102.058 kg, which reads back as 224.9995 lb and would round 70% (157.5 lb)
/// down instead of up.
pub fn load_for_pct(max_kg: f64, pct: f64, units: UnitSystem) -> f64 {
    let to_unit = match units {
        UnitSystem::Imperial => 1.0 / KG_PER_LB,
        UnitSystem::Metric => 1.0,
    };
    let max = (max_kg * to_unit * 10.0).round() / 10.0;
    round_to_plates(max * pct / 100.0 / to_unit, units)
}

/// Fills in the weight of every %1RM item from `maxes` (kg by exercise id).
/// An item whose exercise has no max falls back to its `fallback_kg`, or
/// gets no weight, so the client sees the percentage and picks one.
/// Returns whether anything changed.
pub fn resolve_workout(workout: &mut Workout, maxes: &HashMap<String, f64>, units: UnitSystem) -> bool {
    let mut changed = false;
    for item in workout.blocks.iter_mut().flat_map(|b| b.items.iter_mut()) {
        let Some(pct) = item.load_pct else { continue };
        let load = maxes.get(&item.exercise_id).map(|m| load_for_pct(*m, pct, units)).or(item.fallback_kg);
        if load != item.load_kg {
            item.load_kg = load;
            changed = true;
        }
    }
    changed
}

/// Maxes a finished log beats: for each exercise with a recorded max, the
/// best e1RM from the log's working sets (stats::strength_trends: done,
/// loaded, at most 12 reps) when it's higher. Exercises with no max are
/// left alone; their estimates are offered in Progress › Maxes instead.
pub fn raised_maxes(log: &WorkoutLog, maxes: &HashMap<String, f64>) -> Vec<(String, String, f64)> {
    strength_trends(std::slice::from_ref(log))
        .into_iter()
        .filter_map(|t| {
            let old = *maxes.get(&t.exercise_id)?;
            (t.best_e1rm_kg > old + 0.05).then_some((t.exercise_id, t.exercise_name, t.best_e1rm_kg))
        })
        .collect()
}

/// Whether the workout prescribes any %1RM load of `exercise_id`.
pub fn uses_pct_of(workout: &Workout, exercise_id: &str) -> bool {
    workout.blocks.iter().flat_map(|b| &b.items).any(|i| i.load_pct.is_some() && i.exercise_id == exercise_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, BlockKind, Item};

    fn lb(kg: f64) -> f64 {
        kg / KG_PER_LB
    }

    fn workout(items: Vec<Item>) -> Workout {
        Workout {
            id: "w".into(),
            name: "Day 1".into(),
            blocks: vec![Block { id: "b".into(), kind: BlockKind::Straight, phase: Default::default(), rounds: None, duration_sec: None, work_sec: None, rest_sec: None, items, notes: String::new() }],
            notes: String::new(),
            skip_program_defaults: false,
        }
    }

    fn item(exercise: &str, pct: Option<f64>, load_kg: Option<f64>) -> Item {
        Item {
            id: exercise.into(),
            exercise_id: exercise.into(),
            exercise_name: exercise.into(),
            sets: 3,
            reps: "5".into(),
            load_kg,
            load_pct: pct,
            fallback_kg: None,
            progression: None,
            tempo: None,
            rest_sec: None,
            rpe: None,
            rir: None,
            cardio: None,
            notes: String::new(),
        }
    }

    #[test]
    fn imperial_rounds_to_5_lb() {
        // 75% of a 275 lb squat is 206.25 lb: 205 lb on the bar.
        let max = 275.0 * KG_PER_LB;
        assert!((lb(load_for_pct(max, 75.0, UnitSystem::Imperial)) - 205.0).abs() < 1e-9);
        // 72.5% of 315 is 228.4: 230.
        assert!((lb(load_for_pct(315.0 * KG_PER_LB, 72.5, UnitSystem::Imperial)) - 230.0).abs() < 1e-9);
        // halfway cases round up even when the max was stored rounded to grams
        assert!((lb(load_for_pct(102.058, 70.0, UnitSystem::Imperial)) - 160.0).abs() < 1e-9);
    }

    #[test]
    fn metric_rounds_to_2_5_kg() {
        assert_eq!(load_for_pct(140.0, 77.0, UnitSystem::Metric), 107.5); // 107.8
        assert_eq!(load_for_pct(100.0, 81.0, UnitSystem::Metric), 80.0); // 81 → 80 (nearest)
    }

    #[test]
    fn resolve_fills_and_clears_pct_items_only() {
        let mut w = workout(vec![item("squat", Some(80.0), None), item("curl", None, Some(20.0)), item("bench", Some(70.0), Some(99.0))]);
        let maxes = HashMap::from([("squat".to_string(), 200.0)]);
        assert!(resolve_workout(&mut w, &maxes, UnitSystem::Metric));
        let items = &w.blocks[0].items;
        assert_eq!(items[0].load_kg, Some(160.0));
        assert_eq!(items[1].load_kg, Some(20.0), "a fixed load is left alone");
        assert_eq!(items[2].load_kg, None, "no bench max: the client picks the weight");
        assert!(!resolve_workout(&mut w, &maxes, UnitSystem::Metric), "already resolved");
        assert!(uses_pct_of(&w, "bench") && !uses_pct_of(&w, "curl"));
    }

    #[test]
    fn no_max_falls_back_to_the_programs_weight() {
        let mut w = workout(vec![Item { fallback_kg: Some(60.0), ..item("bench", Some(70.0), None) }]);
        resolve_workout(&mut w, &HashMap::new(), UnitSystem::Metric);
        assert_eq!(w.blocks[0].items[0].load_kg, Some(60.0), "no max: the fallback weight");
        resolve_workout(&mut w, &HashMap::from([("bench".to_string(), 100.0)]), UnitSystem::Metric);
        assert_eq!(w.blocks[0].items[0].load_kg, Some(70.0), "a max wins over the fallback");
        resolve_workout(&mut w, &HashMap::new(), UnitSystem::Metric);
        assert_eq!(w.blocks[0].items[0].load_kg, Some(60.0), "the max deleted: back to the fallback");
    }

    #[test]
    fn a_logged_set_raises_a_max_only_when_it_beats_it() {
        let log: WorkoutLog = serde_json::from_value(serde_json::json!({
            "id": "l", "scheduledId": null, "date": "2026-10-28", "workoutName": "W", "status": "completed", "updatedAt": "2026-10-28T10:00:00Z",
            "sets": [
                { "itemId": "a", "exerciseId": "squat", "exerciseName": "Squat", "setIndex": 0, "reps": 5, "loadKg": 120.0, "done": true },
                { "itemId": "b", "exerciseId": "bench", "exerciseName": "Bench", "setIndex": 0, "reps": 5, "loadKg": 60.0, "done": true },
                { "itemId": "c", "exerciseId": "row", "exerciseName": "Row", "setIndex": 0, "reps": 8, "loadKg": 70.0, "done": true },
                { "itemId": "d", "exerciseId": "squat", "exerciseName": "Squat", "setIndex": 1, "reps": 20, "loadKg": 125.0, "done": true },
            ]
        })).unwrap();
        let maxes = HashMap::from([("squat".to_string(), 130.0), ("bench".to_string(), 100.0)]);
        let raised = raised_maxes(&log, &maxes);
        assert_eq!(raised, vec![("squat".to_string(), "Squat".to_string(), 140.0)], "120 × 5 → 140 beats 130; bench 70 doesn't; row has no max; 20 reps don't count");
        let mut unfinished = log.clone();
        unfinished.status = crate::model::LogStatus::InProgress;
        assert!(raised_maxes(&unfinished, &maxes).is_empty(), "only finished workouts");
    }
}
