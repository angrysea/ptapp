//! Loads prescribed as a percentage of a client's 1RM (WB-5). A program item
//! may carry `load_pct` instead of a weight; assigning the program resolves it
//! against the client's recorded max for that exercise, rounded to the plates
//! they load in their own units. The weight is stored in kg like every load.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Role, UnitSystem, Workout};

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
/// An item whose exercise has no max gets no weight, so the client sees the
/// percentage and picks one. Returns whether anything changed.
pub fn resolve_workout(workout: &mut Workout, maxes: &HashMap<String, f64>, units: UnitSystem) -> bool {
    let mut changed = false;
    for item in workout.blocks.iter_mut().flat_map(|b| b.items.iter_mut()) {
        let Some(pct) = item.load_pct else { continue };
        let load = maxes.get(&item.exercise_id).map(|m| load_for_pct(*m, pct, units));
        if load != item.load_kg {
            item.load_kg = load;
            changed = true;
        }
    }
    changed
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
}
