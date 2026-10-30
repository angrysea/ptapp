# Plan: Warm-ups and post-workout cardio

## Overview

Coaches add a warm-up before a client's workout and cardio after it. Until now, warm-ups were written into the workout notes. Cardio could only be faked as `1 × "10 min"`, which gave the client reps and load boxes and counted toward strength volume.

## Decisions (2026-10-29)

- **Post-workout cardio is real training**, not a cool-down: it's logged (time, distance, average heart rate, RPE) and reported as weekly minutes.
- **Warm-ups are ticked off as a whole.** No amounts are logged.
- **Neither one decides the workout's status.** A workout with every main set done is "completed" even if the warm-up or cardio was skipped, so adherence is unaffected.
- **Intensity can be a heart-rate zone, RPE, or both.** Zones are shown to the client in bpm from an estimated max heart rate (Tanaka, 208 − 0.7 × age) when their birth date is known, otherwise as % of max.

## Design

- **Block phase** (`Block.phase`: `warmup | main | post`, default `main`; `crates/core/src/cardio.rs`). This is separate from the block kind, so a warm-up can be a circuit and cardio can be intervals. The logger shows warm-ups first and post-workout blocks last.
- **Cardio items** (`Item.cardio`: `durationSec`, `distanceM`, `zone`). Time or distance, or both. `sets` counts repeats, and intensity uses the zone and the item's existing `rpe`. Any exercise can be switched to "time or distance" in the builder. Exercises now have a type (`Exercise.modality`: strength, cardio or mobility), and cardio ones default to time. The library gains treadmill, incline walk, run, walk, bike, elliptical, stair climber, ski erg and swim, plus four mobility drills.
- **Logged sets** (`SetLog`) gain `phase`, `durationSec`, `distanceM` and `avgHr`, so a log can be read without its scheduled workout. Warm-up sets and cardio sets are left out of strength volume, set counts, e1RM trends and records. `WeeklyVolume` gains `cardioSec` and `cardioM`, counting cardio from the main and post-workout phases.
- **Program defaults** (`Program.defaultBlocks`, warm-up and post-workout only). Assigning a program copies them into each workout that doesn't have its own block for that phase. A workout can opt out with `skipProgramDefaults`. Already-assigned workouts are copies, so changing the defaults affects only future assignments.
- **Compatibility:** every new field is optional or has a serde default, so existing programs, scheduled workouts, logs and offline drafts load unchanged.

## Block types for warm-ups and cardio (2026-10-30)

Straight sets read wrong for a single run or a mobility routine, so four block kinds were added (`BlockKind` in `crates/core/src/model.rs`). They can be used in any part of the workout:

- **Steady state** (`steady`): one continuous effort for time or distance. The server refuses steady items without a cardio target or with more than one repeat. This is the type a new post-workout block starts as.
- **Flow** (`flow`): drills done once through in order (or `rounds` times), each for time ("30s") or reps. This is the type a new warm-up starts as.
- **Ramp-up** (`ramp`): each line is a stage, easiest first. Lines are shown as "Stage 1 · …".
- **For time** (`fortime`): the work as fast as possible, with an optional time cap (`durationSec`). A stopwatch fills in the client's time, which is stored in `WorkoutLog.blockTimes` (block id → seconds) and shown in the coach's Logs tab.

The block timer steps through a flow's or ramp-up's timed lines (`lineSteps` in `web/js/timer.js`). The builder's type picker starts at "Usual type" (straight for main blocks, flow for warm-ups, steady for post-workout cardio). Older programs keep their straight-set warm-ups and cardio; nothing is converted.

In the workout screen, every type except straight sets has an ⓘ next to its heading that explains to the client how to do it (`KIND_EXPLAIN` in `web/js/views/workout.js`). For steady-state post-workout cardio, the ⓘ sits next to "Post-workout cardio".

## Not in this pass

- Progression rules for cardio (for example, +2 minutes a week).
- Importing heart rate or distance from a watch or Health app.
- A cardio-specific trend chart per exercise (pace or distance over time).
