//! Warm-ups and post-workout cardio. A block's phase says where it sits in
//! the workout: warm-up blocks come first and are ticked off as a whole;
//! post-workout blocks are training, logged and reported like the rest. An
//! item with a cardio target is done for time or distance instead of
//! sets × reps; its intensity is a heart-rate zone, the item's RPE, or both.

use serde::{Deserialize, Serialize};

use crate::model::{Block, Workout};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Warmup,
    #[default]
    Main,
    /// Cardio after the main work. Real training, not a cool-down.
    Post,
}

impl Phase {
    pub fn is_main(&self) -> bool {
        *self == Phase::Main
    }
}

/// Longest cardio piece a program may prescribe or a client may log.
pub const MAX_DURATION_SEC: u32 = 6 * 3600;
/// Longest distance, in metres.
pub const MAX_DISTANCE_M: f64 = 200_000.0;

/// How much of a cardio item to do. At least one of duration and distance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CardioTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
    /// Heart-rate zone 1-5 (`ZONES`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<u8>,
}

impl CardioTarget {
    pub fn validate(&self) -> Result<(), String> {
        if self.duration_sec.is_none() && self.distance_m.is_none() {
            return Err("cardio needs a time or a distance".into());
        }
        if self.duration_sec.is_some_and(|d| d == 0 || d > MAX_DURATION_SEC) {
            return Err("cardio time must be up to 6 hours".into());
        }
        if self.distance_m.is_some_and(|d| !(d > 0.0 && d <= MAX_DISTANCE_M)) {
            return Err("cardio distance must be above 0 and at most 200 km".into());
        }
        if self.zone.is_some_and(|z| !(1..=5).contains(&z)) {
            return Err("heart-rate zone must be 1-5".into());
        }
        Ok(())
    }
}

/// Heart-rate zones as percentages of max heart rate, zone 1 first. The web
/// client shows them as beats per minute (web/js/effort.js keeps a copy).
pub const ZONES: [(u8, u8); 5] = [(50, 60), (60, 70), (70, 80), (80, 90), (90, 100)];

/// Estimated max heart rate for an age (Tanaka: 208 − 0.7 × age).
pub fn max_hr(age_years: f64) -> u32 {
    (208.0 - 0.7 * age_years).round() as u32
}

/// Puts the program's default warm-up and post-workout blocks into a
/// workout being assigned, for each phase the workout doesn't fill itself,
/// unless it opts out. Warm-ups go first and post-workout blocks last.
pub fn add_program_defaults(workout: &mut Workout, defaults: &[Block]) {
    if workout.skip_program_defaults {
        return;
    }
    let has = |w: &Workout, p: Phase| w.blocks.iter().any(|b| b.phase == p);
    let warmup: Vec<Block> = if has(workout, Phase::Warmup) { vec![] } else { pick(defaults, Phase::Warmup) };
    let post: Vec<Block> = if has(workout, Phase::Post) { vec![] } else { pick(defaults, Phase::Post) };
    workout.blocks.splice(0..0, warmup);
    workout.blocks.extend(post);
}

fn pick(blocks: &[Block], phase: Phase) -> Vec<Block> {
    blocks.iter().filter(|b| b.phase == phase).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BlockKind;

    fn block(id: &str, phase: Phase) -> Block {
        Block { id: id.into(), kind: BlockKind::Straight, phase, rounds: None, duration_sec: None, work_sec: None, rest_sec: None, items: vec![], notes: String::new() }
    }

    fn workout(blocks: Vec<Block>) -> Workout {
        Workout { id: "w".into(), name: "Day".into(), notes: String::new(), skip_program_defaults: false, blocks }
    }

    fn ids(w: &Workout) -> Vec<&str> {
        w.blocks.iter().map(|b| b.id.as_str()).collect()
    }

    #[test]
    fn defaults_fill_the_phases_a_workout_leaves_empty() {
        let defaults = [block("dw", Phase::Warmup), block("dp", Phase::Post)];
        let mut w = workout(vec![block("m", Phase::Main)]);
        add_program_defaults(&mut w, &defaults);
        assert_eq!(ids(&w), ["dw", "m", "dp"]);

        // Its own warm-up wins; the default cardio still goes on the end.
        let mut w = workout(vec![block("m", Phase::Main), block("ow", Phase::Warmup)]);
        add_program_defaults(&mut w, &defaults);
        assert_eq!(ids(&w), ["m", "ow", "dp"]);

        let mut w = workout(vec![block("m", Phase::Main)]);
        w.skip_program_defaults = true;
        add_program_defaults(&mut w, &defaults);
        assert_eq!(ids(&w), ["m"]);
    }

    #[test]
    fn targets_are_bounded() {
        let t = |d: Option<u32>, m: Option<f64>, z: Option<u8>| CardioTarget { duration_sec: d, distance_m: m, zone: z };
        assert!(t(Some(1200), None, Some(2)).validate().is_ok());
        assert!(t(None, Some(5000.0), None).validate().is_ok());
        assert!(t(None, None, Some(2)).validate().is_err());
        assert!(t(Some(0), None, None).validate().is_err());
        assert!(t(Some(1200), None, Some(6)).validate().is_err());
        assert!(t(None, Some(-1.0), None).validate().is_err());
    }

    #[test]
    fn max_heart_rate_by_age() {
        assert_eq!(max_hr(40.0), 180);
        assert_eq!(max_hr(20.0), 194);
    }
}
