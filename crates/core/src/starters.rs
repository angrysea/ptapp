//! Starter programs: ready-made programs shipped with the app that a coach
//! copies into their own programs and then edits like any other. Built from
//! the exercise library (library.rs) so names always match; loads are left
//! empty because they depend on the client, and intensity is given as RPE.
//!
//! * Strength benchmark: one day of ramp-ups to a hard set of 3-6 reps on
//!   the main lifts, which the app turns into estimated 1RMs (stats::e1rm,
//!   Progress › Maxes).
//! * Kettlebell & dumbbell, 12 weeks: three days a week, three 4-week
//!   phases from 10-12 reps down to 6-8, with lighter weeks 4 and 8.
//! * Bodyweight only, 12 weeks: the same shape, progressing by harder
//!   variations and more volume instead of load.

use crate::cardio::{CardioTarget, Phase};
use crate::library::builtin_exercises;
use crate::model::{Block, BlockKind, Item, Program, ProgramWorkout, Workout};

/// A program line for one exercise; `id` must be in the library.
fn line(id: &str, sets: u32, reps: &str, rpe: Option<f64>, rest_sec: Option<u32>, notes: &str) -> Item {
    let name = builtin_exercises().into_iter().find(|e| e.id == id).map(|e| e.name).unwrap_or_else(|| panic!("{id} is not a built-in exercise"));
    Item {
        id: String::new(),
        exercise_id: id.into(),
        exercise_name: name,
        sets,
        reps: reps.into(),
        load_kg: None,
        load_pct: None,
        progression: None,
        tempo: None,
        rest_sec,
        rpe,
        rir: None,
        cardio: None,
        notes: notes.into(),
    }
}

/// A line done for time, with a heart-rate zone.
fn timed(id: &str, minutes: u32, zone: Option<u8>) -> Item {
    Item { sets: 1, reps: "1".into(), cardio: Some(CardioTarget { duration_sec: Some(minutes * 60), distance_m: None, zone }), ..line(id, 1, "1", None, None, "") }
}

fn block(kind: BlockKind, phase: Phase, items: Vec<Item>) -> Block {
    Block { id: String::new(), kind, phase, rounds: None, duration_sec: None, work_sec: None, rest_sec: None, items, notes: String::new() }
}

fn main(items: Vec<Item>) -> Block {
    block(BlockKind::Straight, Phase::Main, items)
}

fn workout(name: &str, notes: &str, blocks: Vec<Block>) -> Workout {
    Workout { id: String::new(), name: name.into(), notes: notes.into(), skip_program_defaults: false, blocks }
}

fn program(name: &str, description: &str, weeks: u32, workouts: Vec<ProgramWorkout>, default_blocks: Vec<Block>) -> Program {
    Program { id: String::new(), name: name.into(), description: description.into(), weeks, is_template: true, workouts, default_blocks, updated_at: None }
}

/// Every starter program, in the order the app lists them. Each id is a
/// fixed key (`starter-…`) that the API copies by; a coach's copy gets its own.
pub fn starter_programs() -> Vec<Program> {
    let mut all = vec![benchmark(), kettlebell_dumbbell(), bodyweight()];
    for (p, key) in all.iter_mut().zip(["starter-benchmark", "starter-kettlebell-dumbbell", "starter-bodyweight"]) {
        p.id = key.into();
    }
    all
}

// ---------------------------------------------------------------- benchmark

/// One lift's ramp-up: light sets building to a hard set of 3-6 reps. The
/// hard set gives the estimate; the lighter stages are practice and warm-up.
fn ramp_to_test(id: &str) -> Block {
    let mut b = block(BlockKind::Ramp, Phase::Main, vec![
        line(id, 1, "10", None, Some(60), "Empty bar or a very light weight"),
        line(id, 1, "5", None, Some(90), "About half of what you think your best single is"),
        line(id, 1, "3", None, Some(120), "About 70%: should move fast"),
        line(id, 1, "1", None, Some(180), "About 85%: one smooth rep, no grinding"),
        line(id, 1, "3-6", Some(9.0), Some(240), "Test set: a weight you can lift 3-6 times. Stop when one more rep would be your last good one, and log the reps you did"),
    ]);
    b.notes = "Build up in the stages below, then one hard test set. Your coach can change the weights for you.".into();
    b
}

fn benchmark() -> Program {
    let warmup = block(BlockKind::Flow, Phase::Warmup, vec![
        timed("rowing-erg", 5, Some(2)),
        line("worlds-greatest-stretch", 1, "5 per side", None, None, ""),
        line("leg-swing", 1, "10 per side", None, None, ""),
        line("band-pull-apart", 1, "15", None, None, ""),
    ]);
    let pull_ups = main(vec![line("pull-up", 1, "max", Some(9.0), None, "One set of as many good reps as you can. Use a band or a lat pulldown if you can't do one yet")]);
    let notes = "A test day to estimate your one-rep maxes without lifting a true max. For each lift, build up through the light stages, then do one hard set of 3-6 reps and log the weight and reps. \
                 The app estimates your max from it (Progress › Maxes), and your coach can then set your training weights as a % of it. Rest 3-4 minutes before each test set. Stop any set where your form breaks down.";
    let day = workout("Strength benchmark", notes, vec![
        warmup,
        ramp_to_test("back-squat"),
        ramp_to_test("bench-press"),
        ramp_to_test("deadlift"),
        ramp_to_test("overhead-press"),
        pull_ups,
    ]);
    program(
        "Strength benchmark (1RM estimate)",
        "One day: ramp up to a hard set of 3-6 reps on the squat, bench press, deadlift and overhead press, plus max pull-ups. Logged sets become estimated 1RMs in Progress › Maxes. Repeat every 8-12 weeks to track progress.",
        1,
        vec![ProgramWorkout { week: 1, day: 1, workout: day }],
        vec![],
    )
}

// ---------------------------------------------------------------- 12-week programs

/// Sets, reps and RPE for the main lifts in a week of a 12-week program.
struct Dose {
    sets: u32,
    reps: &'static str,
    rpe: f64,
    /// Accessory work: same sets, a little more reps.
    acc_reps: &'static str,
    /// A finisher's minutes or an interval block's rounds grow by phase.
    phase: u32,
}

fn dose(week: u32) -> Dose {
    let phase = (week - 1) / 4 + 1;
    match week {
        4 | 8 => Dose { sets: 2, reps: if week == 4 { "10" } else { "8" }, rpe: 6.0, acc_reps: "10", phase },
        1..=3 => Dose { sets: 3, reps: "10-12", rpe: 7.0, acc_reps: "12-15", phase },
        5..=7 => Dose { sets: 4, reps: "8-10", rpe: if week == 7 { 8.0 } else { 7.5 }, acc_reps: "10-12", phase },
        12 => Dose { sets: 3, reps: "5", rpe: 8.0, acc_reps: "8", phase },
        _ => Dose { sets: 4, reps: "6-8", rpe: if week == 11 { 8.5 } else { 8.0 }, acc_reps: "8-10", phase },
    }
}

fn week_note(week: u32) -> &'static str {
    match week {
        4 | 8 => "Lighter week: fewer sets and easier efforts so you recover. Keep the weights, leave several reps in reserve.",
        12 => "Last week: heavier sets of 5. Note your weights; they show how far you've come.",
        1 => "Week 1: pick weights you could lift 3-4 more times at the end of each set.",
        5 | 9 => "New phase: fewer reps, so go a little heavier than last week.",
        _ => "",
    }
}

fn kettlebell_dumbbell() -> Program {
    let mut workouts = vec![];
    for week in 1..=12 {
        let d = dose(week);
        let main_line = |id: &str, notes: &str| line(id, d.sets, d.reps, Some(d.rpe), Some(90), notes);
        let acc = |id: &str, notes: &str| line(id, d.sets, d.acc_reps, Some(d.rpe), Some(60), notes);
        let note = week_note(week);

        let day1 = workout(&format!("Week {week} · Squat & press"), note, vec![
            main(vec![main_line("goblet-squat", ""), main_line("kb-press", "Reps per arm")]),
            main(vec![acc("kb-single-leg-deadlift", "Reps per leg"), acc("one-arm-db-row", "Reps per arm")]),
            main(vec![line("plank", d.sets.min(3), "30-45s", None, Some(45), "")]),
        ]);

        let mut swings = block(BlockKind::Emom, Phase::Main, vec![line("kettlebell-swing", 1, if d.phase == 1 { "12" } else { "15" }, Some(7.0), None, "Every minute, on the minute")]);
        swings.duration_sec = Some(if week == 4 || week == 8 { 6 * 60 } else { (6 + 2 * d.phase) * 60 });
        let day2 = workout(&format!("Week {week} · Hinge & pull"), note, vec![
            main(vec![main_line("kb-deadlift", ""), main_line("incline-db-press", "")]),
            swings,
            main(vec![acc("kb-reverse-lunge", "Reps per leg"), acc("kb-high-pull", "")]),
            main(vec![line("kb-suitcase-carry", d.sets.min(3), "30 m per side", None, Some(45), ""), line("dead-bug", d.sets.min(3), "8 per side", None, Some(45), "")]),
        ]);

        let mut finisher = block(BlockKind::Amrap, Phase::Main, vec![
            line("kettlebell-swing", 1, "10", None, None, ""),
            line("kb-thruster", 1, "6", None, None, "A pair of light bells, or one held at the chest"),
            line("burpee", 1, "4", None, None, ""),
        ]);
        finisher.duration_sec = Some(if week == 4 || week == 8 { 6 * 60 } else { (6 + 2 * d.phase) * 60 });
        finisher.notes = "As many rounds as you can at a steady pace; count your rounds.".into();
        let day3 = workout(&format!("Week {week} · Full body"), note, vec![
            main(vec![main_line("kb-front-squat", "One or two bells in the rack"), line("turkish-get-up", d.sets.min(3), "2 per side", Some(7.0), Some(60), "Slow and controlled")]),
            main(vec![acc("kb-row", "Reps per arm"), line("push-up", d.sets, if d.phase == 1 { "8-12" } else { "10-15" }, Some(d.rpe), Some(60), "Hands on a bench if needed")]),
            finisher,
        ]);
        workouts.extend([(1, day1), (3, day2), (5, day3)].map(|(day, workout)| ProgramWorkout { week, day, workout }));
    }
    let warmup = block(BlockKind::Flow, Phase::Warmup, vec![
        line("kb-halo", 1, "30s", None, None, "Light bell; change direction halfway"),
        line("worlds-greatest-stretch", 1, "5 per side", None, None, ""),
        line("cat-cow", 1, "30s", None, None, ""),
        line("leg-swing", 1, "10 per side", None, None, ""),
        line("goblet-squat", 1, "8", None, None, "Light, with a pause at the bottom"),
    ]);
    let cooldown = block(BlockKind::Steady, Phase::Post, vec![timed("brisk-walk", 10, Some(2))]);
    program(
        "Kettlebell & dumbbell strength (12 weeks)",
        "Three days a week (Mon, Wed, Fri) with kettlebells and dumbbells. Weeks 1-4 build with 10-12 reps, weeks 5-8 go to 8-10, weeks 9-12 to 6-8 and finish with sets of 5. Weeks 4 and 8 are lighter. Choose weights so each set ends at the RPE shown.",
        12,
        workouts,
        vec![warmup, cooldown],
    )
}

fn bodyweight() -> Program {
    let mut workouts = vec![];
    for week in 1..=12 {
        let d = dose(week);
        let light = week == 4 || week == 8;
        let note = week_note(week);
        // Harder versions each phase instead of more load.
        let (squat, push, hinge, shoulder, core) = match d.phase {
            1 => (("air-squat", "15-20"), ("incline-push-up", "10-12"), ("glute-bridge", "15-20"), ("prone-ytw", "6 of each letter"), ("plank", "30s")),
            2 => (("reverse-lunge", "10-12 per leg"), ("push-up", "8-12"), ("single-leg-glute-bridge", "10-12 per leg"), ("pike-push-up", "6-8"), ("side-plank", "30s per side")),
            _ => (("bulgarian-split-squat", "10-12 per leg"), ("push-up", "12-15"), ("single-leg-glute-bridge", "15 per leg"), ("pike-push-up", "8-12"), ("hollow-hold", "30s")),
        };
        let mut push_line = line(push.0, d.sets, push.1, Some(d.rpe), Some(75), if d.phase == 3 { "Lower for 3 seconds" } else { "" });
        if d.phase == 3 {
            push_line.tempo = Some("3-0-1-0".into());
        }
        let day1 = workout(&format!("Week {week} · Legs & push"), note, vec![
            main(vec![line(squat.0, d.sets, squat.1, Some(d.rpe), Some(75), if squat.0 == "bulgarian-split-squat" { "Rear foot on a chair or couch" } else { "" }), push_line]),
            main(vec![line(hinge.0, d.sets, hinge.1, Some(d.rpe), Some(60), ""), line(shoulder.0, d.sets.min(3), shoulder.1, Some(d.rpe), Some(60), "")]),
            main(vec![line(core.0, d.sets.min(3), core.1, None, Some(45), "")]),
        ]);

        let rounds = if light { 5 } else { 4 + 2 * d.phase };
        let (work, rest) = match d.phase { 1 => (30, 30), 2 => (40, 20), _ => (45, 15) };
        let mut intervals = block(BlockKind::Interval, Phase::Main, vec![
            line("jumping-jack", 1, "max", None, None, ""),
            line("mountain-climber", 1, "max", None, None, ""),
            line(if d.phase == 1 { "air-squat" } else { "squat-jump" }, 1, "max", None, None, ""),
            line("burpee", 1, "max", None, None, if d.phase == 1 { "Step back instead of jumping if you need to" } else { "" }),
        ]);
        (intervals.rounds, intervals.work_sec, intervals.rest_sec) = (Some(rounds), Some(work), Some(rest));
        intervals.notes = format!("{work}s work, {rest}s rest, moving through the four exercises in order; {rounds} rounds in all.");
        let day2 = workout(&format!("Week {week} · Conditioning & core"), note, vec![
            intervals,
            main(vec![line("prone-ytw", d.sets.min(3), "6 of each letter", None, Some(45), "Back and shoulders: lie face down"), line("dead-bug", d.sets.min(3), "8 per side", None, Some(45), ""), line("side-plank", d.sets.min(3), "20-30s per side", None, Some(45), ""), line("bear-crawl", d.sets.min(3), "20 steps", None, Some(45), "")]),
        ]);

        let mut circuit = block(BlockKind::Circuit, Phase::Main, vec![
            line(if d.phase == 1 { "reverse-lunge" } else { "walking-lunge" }, 1, d.acc_reps, None, None, "Reps per leg"),
            line(push.0, 1, push.1, None, None, ""),
            line("bench-dip", 1, if d.phase == 1 { "8" } else { "12" }, None, None, "Hands on a sturdy chair"),
            line("wall-sit", 1, if d.phase == 3 { "60s" } else { "30-45s" }, None, None, ""),
            line("calf-raise", 1, "20", None, None, "On a step if you have one"),
        ]);
        (circuit.rounds, circuit.rest_sec) = (Some(if light { 2 } else { d.sets }), Some(90));
        let mut day3_blocks = vec![circuit];
        if week == 12 {
            day3_blocks.push(main(vec![
                line("push-up", 1, "max", Some(9.0), Some(120), "Retest: one set of as many good push-ups as you can"),
                line("plank", 1, "max", None, None, "Retest: hold as long as you can with good form"),
            ]));
        }
        let day3 = workout(&format!("Week {week} · Full-body circuit"), note, day3_blocks);
        workouts.extend([(1, day1), (3, day2), (5, day3)].map(|(day, workout)| ProgramWorkout { week, day, workout }));
    }
    let warmup = block(BlockKind::Flow, Phase::Warmup, vec![
        line("jumping-jack", 1, "60s", None, None, ""),
        line("cat-cow", 1, "30s", None, None, ""),
        line("worlds-greatest-stretch", 1, "5 per side", None, None, ""),
        line("leg-swing", 1, "10 per side", None, None, ""),
        line("air-squat", 1, "10", None, None, "Slow, with a pause at the bottom"),
    ]);
    let cooldown = block(BlockKind::Steady, Phase::Post, vec![timed("brisk-walk", 10, Some(2))]);
    program(
        "Bodyweight only (12 weeks)",
        "Three days a week (Mon, Wed, Fri), no equipment beyond a chair and a wall. Each 4-week phase moves to harder versions (incline to full push-ups, bridges to single-leg bridges, squats to split squats) and longer intervals. Weeks 4 and 8 are lighter; week 12 ends with a push-up and plank retest.",
        12,
        workouts,
        vec![warmup, cooldown],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_are_valid_programs() {
        let all = starter_programs();
        assert_eq!(all.len(), 3);
        for p in &all {
            p.validate().unwrap_or_else(|e| panic!("{}: {e}", p.name));
            let size = serde_json::to_vec(p).unwrap().len();
            assert!(size < 300_000, "{} is {size} bytes", p.name);
        }
    }

    #[test]
    fn twelve_week_programs_fill_three_days_every_week() {
        for p in starter_programs().into_iter().filter(|p| p.weeks == 12) {
            for week in 1..=12 {
                let days: Vec<u32> = p.workouts.iter().filter(|w| w.week == week).map(|w| w.day).collect();
                assert_eq!(days, vec![1, 3, 5], "{} week {week}", p.name);
            }
            assert!(p.default_blocks.iter().any(|b| b.kind == BlockKind::Flow && b.phase == Phase::Warmup));
        }
    }

    #[test]
    fn the_benchmark_ends_each_lift_with_a_test_set() {
        let p = benchmark();
        let day = &p.workouts[0].workout;
        let ramps: Vec<_> = day.blocks.iter().filter(|b| b.kind == BlockKind::Ramp).collect();
        assert_eq!(ramps.len(), 4);
        for b in ramps {
            let last = b.items.last().unwrap();
            assert_eq!((last.reps.as_str(), last.rpe), ("3-6", Some(9.0)), "{}", last.exercise_name);
        }
    }

    #[test]
    fn doses_get_heavier_by_phase_and_lighter_in_weeks_4_and_8() {
        assert_eq!(dose(1).reps, "10-12");
        assert_eq!(dose(6).reps, "8-10");
        assert_eq!(dose(10).reps, "6-8");
        assert_eq!((dose(4).sets, dose(8).sets), (2, 2));
        assert_eq!(dose(12).reps, "5");
    }
}
