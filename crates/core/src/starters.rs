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
//! * Bodybuilding, 12 weeks, four days a week, as a push/pull split or an
//!   upper/lower split: hypertrophy work that stresses time under tension
//!   with a tempo on every lift (`tut`).

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
    let mut all = vec![benchmark(), kettlebell_dumbbell(), bodyweight(), push_pull(), upper_lower()];
    let keys = ["starter-benchmark", "starter-kettlebell-dumbbell", "starter-bodyweight", "starter-push-pull", "starter-upper-lower"];
    for (p, key) in all.iter_mut().zip(keys) {
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

// ---------------------------------------------------------------- bodybuilding (time under tension)

/// A week of the bodybuilding programs. Tempo is lowering-pause-lifting-top in
/// seconds, so "4-1-1-0" is four seconds down, a one-second pause, one up.
/// Compounds slow the lowering by phase and phase 3 adds a pause in the
/// stretch; isolation work holds a squeeze at the top instead.
struct Tut {
    sets: u32,
    reps: &'static str,
    iso_reps: &'static str,
    rpe: f64,
    tempo: &'static str,
    iso_tempo: &'static str,
}

fn tut(week: u32) -> Tut {
    match week {
        4 | 8 => Tut { sets: 2, reps: "10", iso_reps: "12", rpe: 6.0, tempo: "3-1-1-0", iso_tempo: "2-0-1-1" },
        1..=3 => Tut { sets: 3, reps: "10-12", iso_reps: "12-15", rpe: if week == 3 { 8.0 } else { 7.0 }, tempo: "3-1-1-0", iso_tempo: "2-0-1-1" },
        5..=7 => Tut { sets: 4, reps: "8-10", iso_reps: "10-12", rpe: if week == 7 { 8.5 } else { 8.0 }, tempo: "4-1-1-0", iso_tempo: "3-0-1-2" },
        12 => Tut { sets: 3, reps: "10-12", iso_reps: "12-15", rpe: 9.0, tempo: "4-1-1-0", iso_tempo: "3-0-1-2" },
        _ => Tut { sets: 4, reps: "8-10", iso_reps: "10-12", rpe: if week == 11 { 9.0 } else { 8.5 }, tempo: "3-2-1-0", iso_tempo: "3-0-1-2" },
    }
}

fn tut_note(week: u32) -> &'static str {
    match week {
        1 => "Tempo on every lift: 3-1-1-0 means 3 seconds lowering, a 1-second pause, 1 second up, straight into the next rep. Count it. Pick weights you can control at that speed; they will be lighter than usual.",
        4 | 8 => "Lighter week: two sets, same tempo, several reps in reserve.",
        5 => "Phase 2: lowering slows to 4 seconds. Drop the weight if you need to; the time under tension is the point.",
        9 => "Phase 3: a 2-second pause in the stretch (bottom of the squat, chest just above the bar, arms long on rows).",
        12 => "Last week: back to higher reps with a 4-second lowering, taken close to failure.",
        _ => "",
    }
}

/// A compound lift at the week's tempo; `notes` adds to the cue.
fn compound(t: &Tut, id: &str, notes: &str) -> Item {
    Item { tempo: Some(t.tempo.into()), ..line(id, t.sets, t.reps, Some(t.rpe), Some(120), notes) }
}

/// An isolation lift: more reps, a squeeze at the top.
fn isolation(t: &Tut, id: &str, notes: &str) -> Item {
    Item { tempo: Some(t.iso_tempo.into()), ..line(id, t.sets, t.iso_reps, Some(t.rpe), Some(60), notes) }
}

/// Two isolation lifts done back to back.
fn superset(t: &Tut, a: Item, b: Item) -> Block {
    let mut s = block(BlockKind::Superset, Phase::Main, vec![a, b]);
    (s.rounds, s.rest_sec) = (Some(t.sets), Some(75));
    s
}

fn bodybuilding_defaults() -> Vec<Block> {
    let warmup = block(BlockKind::Flow, Phase::Warmup, vec![
        timed("rowing-erg", 5, Some(2)),
        line("band-pull-apart", 1, "15", None, None, ""),
        line("worlds-greatest-stretch", 1, "5 per side", None, None, ""),
        line("leg-swing", 1, "10 per side", None, None, ""),
    ]);
    let cardio = block(BlockKind::Steady, Phase::Post, vec![timed("incline-walk", 15, Some(2))]);
    vec![warmup, cardio]
}

/// Four days a week on days 1, 2, 4 and 5 (Mon, Tue, Thu, Fri).
fn four_days(name: &str, description: &str, day: impl Fn(&Tut, u32, u32) -> Workout) -> Program {
    let mut workouts = vec![];
    for week in 1..=12 {
        let t = tut(week);
        for (slot, d) in [1, 2, 4, 5].into_iter().enumerate() {
            let mut w = day(&t, week, slot as u32);
            w.notes = tut_note(week).into();
            workouts.push(ProgramWorkout { week, day: d, workout: w });
        }
    }
    program(name, description, 12, workouts, bodybuilding_defaults())
}

fn push_pull() -> Program {
    four_days(
        "Bodybuilding push/pull (12 weeks, 4 days)",
        "Hypertrophy with time under tension. Push days (Mon, Thu) train chest, shoulders, triceps and quads; pull days (Tue, Fri) train back, rear delts, biceps, hamstrings and glutes. Every lift has a tempo: the lowering slows from 3 to 4 seconds and the last phase adds a pause in the stretch. Weeks 4 and 8 are lighter.",
        |t, week, slot| match slot {
            0 => workout(&format!("Week {week} · Push A"), "", vec![
                main(vec![compound(t, "back-squat", ""), compound(t, "bench-press", "")]),
                main(vec![compound(t, "seated-db-shoulder-press", ""), isolation(t, "leg-extension", "")]),
                superset(t, isolation(t, "lateral-raise", ""), isolation(t, "triceps-pushdown", "")),
                main(vec![isolation(t, "calf-raise", "Full stretch at the bottom")]),
            ]),
            1 => workout(&format!("Week {week} · Pull A"), "", vec![
                main(vec![compound(t, "romanian-deadlift", ""), compound(t, "lat-pulldown", "")]),
                main(vec![compound(t, "barbell-row", ""), isolation(t, "leg-curl", "")]),
                superset(t, isolation(t, "face-pull", ""), isolation(t, "bicep-curl", "")),
            ]),
            2 => workout(&format!("Week {week} · Push B"), "", vec![
                main(vec![compound(t, "leg-press", ""), compound(t, "incline-db-press", "")]),
                main(vec![compound(t, "bulgarian-split-squat", "Reps per leg"), compound(t, "dip", "Lean forward for chest")]),
                superset(t, isolation(t, "db-fly", ""), isolation(t, "overhead-triceps-extension", "")),
                main(vec![isolation(t, "lateral-raise", ""), isolation(t, "calf-raise", "")]),
            ]),
            _ => workout(&format!("Week {week} · Pull B"), "", vec![
                main(vec![compound(t, "hip-thrust", ""), compound(t, "chest-supported-row", "")]),
                main(vec![compound(t, "pull-up", "Use a band or the lat pulldown if needed"), compound(t, "one-arm-db-row", "Reps per arm")]),
                superset(t, isolation(t, "leg-curl", ""), isolation(t, "rear-delt-fly", "")),
                main(vec![isolation(t, "hammer-curl", "")]),
            ]),
        },
    )
}

fn upper_lower() -> Program {
    four_days(
        "Bodybuilding upper/lower (12 weeks, 4 days)",
        "Hypertrophy with time under tension. Upper days (Mon, Thu) and lower days (Tue, Fri), each muscle twice a week. Every lift has a tempo: the lowering slows from 3 to 4 seconds and the last phase adds a pause in the stretch. Weeks 4 and 8 are lighter.",
        |t, week, slot| match slot {
            0 => workout(&format!("Week {week} · Upper A"), "", vec![
                main(vec![compound(t, "bench-press", ""), compound(t, "barbell-row", "")]),
                main(vec![compound(t, "seated-db-shoulder-press", ""), compound(t, "lat-pulldown", "")]),
                superset(t, isolation(t, "bicep-curl", ""), isolation(t, "triceps-pushdown", "")),
                main(vec![isolation(t, "lateral-raise", "")]),
            ]),
            1 => workout(&format!("Week {week} · Lower A"), "", vec![
                main(vec![compound(t, "back-squat", ""), compound(t, "romanian-deadlift", "")]),
                main(vec![compound(t, "leg-press", ""), isolation(t, "leg-curl", "")]),
                main(vec![isolation(t, "calf-raise", "Full stretch at the bottom"), line("plank", t.sets.min(3), "45s", None, Some(45), "")]),
            ]),
            2 => workout(&format!("Week {week} · Upper B"), "", vec![
                main(vec![compound(t, "incline-db-press", ""), compound(t, "chest-supported-row", "")]),
                main(vec![compound(t, "dip", "Lean forward for chest"), compound(t, "one-arm-db-row", "Reps per arm")]),
                superset(t, isolation(t, "db-fly", ""), isolation(t, "rear-delt-fly", "")),
                superset(t, isolation(t, "hammer-curl", ""), isolation(t, "overhead-triceps-extension", "")),
            ]),
            _ => workout(&format!("Week {week} · Lower B"), "", vec![
                main(vec![compound(t, "front-squat", ""), compound(t, "hip-thrust", "")]),
                main(vec![compound(t, "bulgarian-split-squat", "Reps per leg"), isolation(t, "leg-extension", "")]),
                main(vec![isolation(t, "leg-curl", ""), isolation(t, "calf-raise", "")]),
            ]),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_are_valid_programs() {
        let all = starter_programs();
        assert_eq!(all.len(), 5);
        for p in &all {
            p.validate().unwrap_or_else(|e| panic!("{}: {e}", p.name));
            let size = serde_json::to_vec(p).unwrap().len();
            assert!(size < 300_000, "{} is {size} bytes", p.name);
        }
    }

    #[test]
    fn twelve_week_programs_fill_their_days_every_week() {
        for p in starter_programs().into_iter().filter(|p| p.weeks == 12) {
            let want = if p.name.starts_with("Bodybuilding") { vec![1, 2, 4, 5] } else { vec![1, 3, 5] };
            for week in 1..=12 {
                let days: Vec<u32> = p.workouts.iter().filter(|w| w.week == week).map(|w| w.day).collect();
                assert_eq!(days, want, "{} week {week}", p.name);
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

    #[test]
    fn bodybuilding_lifts_all_have_a_tempo_that_slows_by_phase() {
        for p in [push_pull(), upper_lower()] {
            for pw in &p.workouts {
                for it in pw.workout.blocks.iter().flat_map(|b| &b.items).filter(|it| it.exercise_id != "plank") {
                    assert!(it.tempo.is_some(), "{} week {}: {} has no tempo", p.name, pw.week, it.exercise_name);
                }
            }
        }
        let lowering = |w: u32| tut(w).tempo.split('-').next().unwrap().parse::<u32>().unwrap();
        assert!(lowering(1) < lowering(5), "phase 2 lowers slower");
        assert_eq!(tut(9).tempo, "3-2-1-0", "phase 3 pauses in the stretch");
        assert_eq!((tut(4).sets, tut(8).sets), (2, 2));
    }
}
