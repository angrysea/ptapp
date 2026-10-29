//! Coach dashboard numbers (LG-6): adherence, missed workouts, weekly volume
//! and estimated-1RM strength trends.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Duration, NaiveDate};
use serde::Serialize;

use crate::cardio::Phase;
use crate::model::{LogStatus, ScheduledWorkout, WorkoutLog};

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Adherence {
    /// Scheduled workouts dated before `today`, plus today's if already logged.
    pub due: u32,
    pub completed: u32,
    pub skipped: u32,
    pub missed: u32,
    /// completed / due, 0-100. None when nothing was due yet.
    pub pct: Option<f64>,
    pub missed_dates: Vec<NaiveDate>,
}

pub fn adherence(scheduled: &[ScheduledWorkout], logs: &[WorkoutLog], today: NaiveDate) -> Adherence {
    let by_sched: HashMap<&str, &WorkoutLog> = logs
        .iter()
        .filter_map(|l| l.scheduled_id.as_deref().map(|s| (s, l)))
        .collect();
    let mut a = Adherence::default();
    // Today only counts once it has been logged, so an unfinished workout is never "missed" early.
    for s in scheduled.iter().filter(|s| s.date < today || (s.date == today && by_sched.contains_key(s.id.as_str()))) {
        a.due += 1;
        match by_sched.get(s.id.as_str()).map(|l| l.status) {
            Some(st) if st.counts_as_done() => a.completed += 1,
            Some(LogStatus::Skipped) => a.skipped += 1,
            _ => {
                a.missed += 1;
                a.missed_dates.push(s.date);
            }
        }
    }
    a.pct = (a.due > 0).then(|| (a.completed as f64 * 1000.0 / a.due as f64).round() / 10.0);
    a
}

/// The run of missed workouts ending with the latest one due (LG-7): how
/// many, and the date the run started. A completed or skipped workout ends
/// it (a skip is the client telling the coach). None if the latest due
/// workout wasn't missed.
pub fn missed_streak(scheduled: &[ScheduledWorkout], logs: &[WorkoutLog], today: NaiveDate) -> Option<(u32, NaiveDate)> {
    let logged: HashMap<&str, LogStatus> = logs.iter().filter_map(|l| l.scheduled_id.as_deref().map(|s| (s, l.status))).collect();
    let mut due: Vec<&ScheduledWorkout> = scheduled.iter().filter(|s| s.date < today).collect();
    due.sort_by_key(|s| std::cmp::Reverse(s.date));
    let mut run = 0;
    let mut start = None;
    for s in due {
        match logged.get(s.id.as_str()) {
            None | Some(LogStatus::InProgress) => {
                run += 1;
                start = Some(s.date);
            }
            Some(_) => break,
        }
    }
    start.map(|d| (run, d))
}

/// Epley estimated 1RM. A single is taken at face value.
pub fn e1rm(load_kg: f64, reps: u32) -> f64 {
    if reps <= 1 {
        load_kg
    } else {
        load_kg * (1.0 + reps as f64 / 30.0)
    }
}

pub fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyVolume {
    pub week_start: NaiveDate,
    /// Sum of reps x load over completed strength sets.
    pub volume_kg: f64,
    /// Completed strength sets: main work, not warm-ups or cardio.
    pub sets: u32,
    pub workouts: u32,
    /// Cardio logged for time or distance (main or post-workout, not
    /// warm-ups, which are only ticked off).
    pub cardio_sec: u32,
    pub cardio_m: f64,
}

pub fn weekly_volume(logs: &[WorkoutLog]) -> Vec<WeeklyVolume> {
    let mut weeks: BTreeMap<NaiveDate, WeeklyVolume> = BTreeMap::new();
    for l in logs.iter().filter(|l| l.status.counts_as_done()) {
        let ws = week_start(l.date);
        let w = weeks.entry(ws).or_insert(WeeklyVolume { week_start: ws, volume_kg: 0.0, sets: 0, workouts: 0, cardio_sec: 0, cardio_m: 0.0 });
        w.workouts += 1;
        for s in l.sets.iter().filter(|s| s.done && s.phase != Phase::Warmup && s.is_cardio()) {
            w.cardio_sec += s.duration_sec.unwrap_or(0);
            w.cardio_m += s.distance_m.unwrap_or(0.0);
        }
        for s in l.sets.iter().filter(|s| s.done && s.is_strength()) {
            w.sets += 1;
            if let (Some(r), Some(kg)) = (s.reps, s.load_kg) {
                w.volume_kg += r as f64 * kg;
            }
        }
    }
    weeks
        .into_values()
        .map(|mut w| {
            w.volume_kg = w.volume_kg.round();
            w.cardio_m = w.cardio_m.round();
            w
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StrengthPoint {
    pub date: NaiveDate,
    pub e1rm_kg: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StrengthTrend {
    pub exercise_id: String,
    pub exercise_name: String,
    pub points: Vec<StrengthPoint>,
    pub best_e1rm_kg: f64,
}

/// Best e1RM per exercise per day, for exercises logged with load. Sorted by
/// how often the exercise appears so the main lifts come first.
pub fn strength_trends(logs: &[WorkoutLog]) -> Vec<StrengthTrend> {
    let mut per: HashMap<String, (String, BTreeMap<NaiveDate, f64>)> = HashMap::new();
    for l in logs.iter().filter(|l| l.status.counts_as_done()) {
        for s in l.sets.iter().filter(|s| s.done && s.is_strength()) {
            let (Some(r), Some(kg)) = (s.reps, s.load_kg) else { continue };
            if r == 0 || kg <= 0.0 || r > 12 {
                continue; // e1RM formulas are unreliable past ~12 reps
            }
            let est = e1rm(kg, r);
            let entry = per.entry(s.exercise_id.clone()).or_insert_with(|| (s.exercise_name.clone(), BTreeMap::new()));
            let best = entry.1.entry(l.date).or_insert(0.0);
            if est > *best {
                *best = est;
            }
        }
    }
    let mut out: Vec<StrengthTrend> = per
        .into_iter()
        .map(|(id, (name, days))| {
            let points: Vec<StrengthPoint> = days
                .into_iter()
                .map(|(date, v)| StrengthPoint { date, e1rm_kg: (v * 10.0).round() / 10.0 })
                .collect();
            let best = points.iter().map(|p| p.e1rm_kg).fold(0.0, f64::max);
            StrengthTrend { exercise_id: id, exercise_name: name, points, best_e1rm_kg: best }
        })
        .collect();
    out.sort_by(|a, b| b.points.len().cmp(&a.points.len()).then(a.exercise_name.cmp(&b.exercise_name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SetLog, Workout};
    use chrono::{TimeZone, Utc};

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn sched(id: &str, day: u32) -> ScheduledWorkout {
        ScheduledWorkout {
            id: id.into(),
            date: d(day),
            assignment_id: "a".into(),
            program_name: "P".into(),
            week: 1,
            day: 1,
            workout: Workout { id: "w".into(), name: "W".into(), notes: String::new(), skip_program_defaults: false, blocks: vec![] },
        }
    }

    fn log(sched_id: &str, day: u32, status: LogStatus, sets: Vec<(u32, f64)>) -> WorkoutLog {
        WorkoutLog {
            id: format!("log-{sched_id}"),
            scheduled_id: Some(sched_id.into()),
            date: d(day),
            workout_name: "W".into(),
            status,
            exercise_notes: Default::default(),
            rating: None,
            sets: sets
                .into_iter()
                .enumerate()
                .map(|(i, (r, kg))| SetLog {
                    item_id: "i".into(),
                    exercise_id: "back-squat".into(),
                    exercise_name: "Back Squat".into(),
                    set_index: i as u32,
                    reps: Some(r),
                    load_kg: Some(kg),
                    rpe: None,
                    done: true,
                    ..Default::default()
                })
                .collect(),
            comment: String::new(),
            check: None,
            updated_at: Utc.with_ymd_and_hms(2026, 10, day, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn missed_streak_counts_back_from_the_latest_due() {
        let s = vec![sched("a", 5), sched("b", 7), sched("c", 9), sched("d", 12), sched("e", 20)];
        // a done; b, c, d missed; e is in the future
        let logs = vec![log("a", 5, LogStatus::Completed, vec![])];
        assert_eq!(missed_streak(&s, &logs, d(14)), Some((3, d(7))));
        // a skip ends the run: only d counts
        let logs = vec![log("a", 5, LogStatus::Completed, vec![]), log("c", 9, LogStatus::Skipped, vec![])];
        assert_eq!(missed_streak(&s, &logs, d(14)), Some((1, d(12))));
        // the latest due was done: no run
        let logs = vec![log("d", 12, LogStatus::Completed, vec![])];
        assert_eq!(missed_streak(&s, &logs, d(14)), None);
        // started but never finished counts as missed
        let logs = vec![log("d", 12, LogStatus::InProgress, vec![]), log("c", 9, LogStatus::Completed, vec![])];
        assert_eq!(missed_streak(&s, &logs, d(14)), Some((1, d(12))));
    }

    #[test]
    fn adherence_counts() {
        let s = vec![sched("a", 5), sched("b", 7), sched("c", 9), sched("d", 12), sched("e", 14)];
        let logs = vec![
            log("a", 5, LogStatus::Completed, vec![]),
            log("b", 7, LogStatus::Modified, vec![]),
            log("c", 9, LogStatus::Skipped, vec![]),
        ];
        let a = adherence(&s, &logs, d(14));
        assert_eq!(a.due, 4);
        assert_eq!(a.completed, 2);
        assert_eq!(a.skipped, 1);
        assert_eq!(a.missed, 1);
        assert_eq!(a.missed_dates, vec![d(12)]);
        assert_eq!(a.pct, Some(50.0));
        assert_eq!(adherence(&s, &logs, d(1)).pct, None);
        // a workout logged today counts; an unlogged one today does not
        assert_eq!(adherence(&s, &logs, d(9)).due, 3);
        assert_eq!(adherence(&s, &logs, d(12)).due, 3);
    }

    #[test]
    fn volume_and_e1rm() {
        // Oct 5 2026 is a Monday.
        let logs = vec![
            log("a", 5, LogStatus::Completed, vec![(5, 100.0), (5, 100.0)]),
            log("b", 7, LogStatus::Completed, vec![(3, 110.0)]),
            log("c", 12, LogStatus::Completed, vec![(1, 130.0)]),
            log("d", 13, LogStatus::Skipped, vec![(5, 500.0)]),
        ];
        let v = weekly_volume(&logs);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].week_start, d(5));
        assert_eq!(v[0].volume_kg, 1330.0);
        assert_eq!(v[0].sets, 3);
        assert_eq!(v[1].volume_kg, 130.0);

        let t = strength_trends(&logs);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].points.len(), 3);
        assert_eq!(t[0].points[0].e1rm_kg, 116.7); // 100 * (1 + 5/30)
        assert_eq!(t[0].points[1].e1rm_kg, 121.0); // 110 * 1.1
        assert_eq!(t[0].best_e1rm_kg, 130.0);
    }

    #[test]
    fn warmups_and_cardio_stay_out_of_strength_numbers() {
        let mut l = log("a", 5, LogStatus::Completed, vec![(5, 100.0), (10, 20.0)]);
        l.sets[1].phase = Phase::Warmup; // a light goblet squat warm-up
        let cardio = |phase, sec, m: Option<f64>| SetLog { exercise_id: "rowing-erg".into(), phase, duration_sec: Some(sec), distance_m: m, done: true, ..Default::default() };
        l.sets.push(cardio(Phase::Warmup, 300, None));
        l.sets.push(cardio(Phase::Post, 1200, Some(4000.0)));
        l.sets.push(cardio(Phase::Main, 600, None));
        let v = weekly_volume(&[l.clone()]);
        assert_eq!((v[0].sets, v[0].volume_kg), (1, 500.0));
        assert_eq!((v[0].cardio_sec, v[0].cardio_m), (1800, 4000.0));
        let t = strength_trends(&[l]);
        assert_eq!(t[0].points[0].e1rm_kg, 116.7);
    }
}
