//! Domain types. These serialize to camelCase JSON for the web client and are
//! stored as-is in DynamoDB (plus `pk`/`sk` key attributes added by the API).
//! All quantities are stored in metric units (kg, cm, mm); the client converts
//! for display according to the user's unit preference (BM-8).

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::bodycomp::{BodyCompResult, DensityEquation, SkinfoldMethod};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Coach,
    Client,
    Admin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sex {
    Male,
    Female,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UnitSystem {
    Metric,
    /// New accounts start in pounds and inches; anyone can switch on Profile.
    #[default]
    Imperial,
}

// ---------------------------------------------------------------- users

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub user_id: String,
    pub role: Role,
    /// Tenant key. For a coach this is their own user id; for a client it is
    /// the id of the coach they belong to.
    pub coach_id: String,
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub units: UnitSystem,
    pub sex: Option<Sex>,
    pub birth_date: Option<NaiveDate>,
    #[serde(default)]
    pub business_name: Option<String>,
    /// IANA time zone reported by the user's device (e.g. "America/Chicago").
    /// Workouts are dated in local time, so "today" must be too.
    #[serde(default)]
    pub time_zone: Option<String>,
    /// When they agreed to how their data is used (privacy NFR): at sign-up,
    /// or on first sign-in for accounts made before the agreement existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consented_at: Option<DateTime<Utc>>,
    /// Set while an admin has suspended this coach; the API then refuses
    /// their requests (auth.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Whether `tz` is a time zone name `Profile::today` understands.
pub fn valid_time_zone(tz: &str) -> bool {
    tz.parse::<chrono_tz::Tz>().is_ok()
}

impl Profile {
    /// The user's local calendar date at `now`; UTC if no time zone is known.
    pub fn today(&self, now: DateTime<Utc>) -> NaiveDate {
        match self.time_zone.as_deref().and_then(|tz| tz.parse::<chrono_tz::Tz>().ok()) {
            Some(tz) => now.with_timezone(&tz).date_naive(),
            None => now.date_naive(),
        }
    }

    pub fn age_on(&self, date: NaiveDate) -> Option<f64> {
        self.birth_date
            .map(|b| (date - b).num_days() as f64 / 365.2425)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub code: String,
    /// Client invites: what the invitee joins as. Coach invites come from an
    /// admin and create a new coach account.
    #[serde(default)]
    pub kind: InviteKind,
    /// The coach (tenant) a client joins; empty on coach invites.
    pub coach_id: String,
    /// Shown on the join screen: the coach's name, or the app's on coach invites.
    pub coach_name: String,
    pub email: String,
    /// The invitee's name as the inviter typed it (client or new coach).
    pub client_name: String,
    pub created_at: DateTime<Utc>,
    pub accepted_by: Option<String>,
    /// DynamoDB TTL (epoch seconds); unaccepted invites expire.
    pub ttl: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum InviteKind {
    #[default]
    Client,
    Coach,
}

/// An admin's list of coaches (`ADMIN` / `COACH#{id}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoachEntry {
    pub coach_id: String,
    pub name: String,
    pub email: String,
    pub joined_at: DateTime<Utc>,
    /// Mirrors `Profile::suspended_at`, for the admin's coach list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterEntry {
    pub client_id: String,
    pub name: String,
    pub email: String,
    pub joined_at: DateTime<Utc>,
}

// ---------------------------------------------------------------- onboarding

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Intake {
    #[serde(default)]
    pub goals: String,
    #[serde(default)]
    pub training_history: String,
    #[serde(default)]
    pub equipment: Vec<String>,
    #[serde(default)]
    pub schedule: String,
    #[serde(default)]
    pub injuries: String,
    pub sex: Option<Sex>,
    pub birth_date: Option<NaiveDate>,
    pub height_cm: Option<f64>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParqAnswer {
    pub question: String,
    pub answer: bool,
}

/// Signed health screening + liability waiver (ON-3). Immutable once written.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiver {
    pub id: String,
    pub waiver_version: String,
    pub parq: Vec<ParqAnswer>,
    /// True when any PAR-Q answer is "yes": the coach should request
    /// medical clearance before programming.
    pub parq_flagged: bool,
    pub signed_name: String,
    pub signed_at: DateTime<Utc>,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
}

// ---------------------------------------------------------------- exercises

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exercise {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub muscle_groups: Vec<String>,
    #[serde(default)]
    pub equipment: Vec<String>,
    #[serde(default)]
    pub cues: Vec<String>,
    /// External demo link (e.g. YouTube) or empty.
    #[serde(default)]
    pub demo_url: Option<String>,
    /// S3 key of a coach-uploaded demo video; served via presigned URL.
    /// Only set once the upload is confirmed to have landed.
    #[serde(default)]
    pub demo_key: Option<String>,
    /// Key handed out for an upload that has not been confirmed yet. The
    /// current demo stays in place until it is.
    #[serde(default)]
    pub pending_demo_key: Option<String>,
    #[serde(default)]
    pub custom: bool,
}

// ---------------------------------------------------------------- programs

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BlockKind {
    /// Plain sets of one exercise (or several done one after another).
    Straight,
    Superset,
    Circuit,
    Amrap,
    Emom,
    Interval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    #[serde(default)]
    pub id: String,
    pub exercise_id: String,
    pub exercise_name: String,
    pub sets: u32,
    /// Free text so coaches can write "8-10", "5", "AMRAP", "30s".
    pub reps: String,
    pub load_kg: Option<f64>,
    /// Load as a percentage of the client's 1RM (WB-5). Assigning fills
    /// `load_kg` from their max (strength.rs); in a program it stays empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_pct: Option<f64>,
    /// Suggest a change for the next workout when this one goes well (WB-6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progression: Option<crate::progression::ProgressionRule>,
    pub tempo: Option<String>,
    pub rest_sec: Option<u32>,
    pub rpe: Option<f64>,
    pub rir: Option<f64>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    #[serde(default)]
    pub id: String,
    pub kind: BlockKind,
    /// Rounds for superset/circuit/interval.
    pub rounds: Option<u32>,
    /// Total cap for AMRAP / EMOM.
    pub duration_sec: Option<u32>,
    pub work_sec: Option<u32>,
    pub rest_sec: Option<u32>,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workout {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramWorkout {
    /// 1-based week number.
    pub week: u32,
    /// 1-based day within the week (1 = first day of the program week).
    pub day: u32,
    pub workout: Workout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Program {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub weeks: u32,
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub workouts: Vec<ProgramWorkout>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Program {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("program name is required".into());
        }
        if !(1..=52).contains(&self.weeks) {
            return Err("weeks must be between 1 and 52".into());
        }
        for pw in &self.workouts {
            if pw.week < 1 || pw.week > self.weeks {
                return Err(format!("workout '{}' is in week {} outside the program", pw.workout.name, pw.week));
            }
            if !(1..=7).contains(&pw.day) {
                return Err(format!("workout '{}' has day {}; days are 1-7", pw.workout.name, pw.day));
            }
            if pw.workout.name.trim().is_empty() {
                return Err("every workout needs a name".into());
            }
            for b in &pw.workout.blocks {
                for it in &b.items {
                    if !(1..=50).contains(&it.sets) {
                        return Err(format!("{}: sets must be 1-50", it.exercise_name));
                    }
                    if it.rpe.is_some_and(|r| !(1.0..=10.0).contains(&r)) {
                        return Err(format!("{}: RPE must be 1-10", it.exercise_name));
                    }
                    if it.rir.is_some_and(|r| !(0.0..=10.0).contains(&r)) {
                        return Err(format!("{}: RIR must be 0-10", it.exercise_name));
                    }
                    if it.load_kg.is_some_and(|l| l < 0.0) {
                        return Err(format!("{}: load cannot be negative", it.exercise_name));
                    }
                    if let Some(rule) = &it.progression {
                        rule.validate().map_err(|e| format!("{}: {e}", it.exercise_name))?;
                    }
                    if it.load_pct.is_some_and(|p| !(p > 0.0 && p <= crate::strength::MAX_PCT)) {
                        return Err(format!("{}: % of 1RM must be above 0 and at most {}", it.exercise_name, crate::strength::MAX_PCT));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    pub id: String,
    pub program_id: String,
    pub program_name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub created_at: DateTime<Utc>,
}

/// A workout placed on a client's calendar. Holds a snapshot of the workout
/// so later edits to the program template do not rewrite history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledWorkout {
    pub id: String,
    pub date: NaiveDate,
    pub assignment_id: String,
    pub program_name: String,
    pub week: u32,
    pub day: u32,
    pub workout: Workout,
}

// ---------------------------------------------------------------- logging

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LogStatus {
    InProgress,
    Completed,
    Skipped,
    Modified,
}

impl LogStatus {
    pub fn counts_as_done(self) -> bool {
        matches!(self, LogStatus::Completed | LogStatus::Modified)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLog {
    pub item_id: String,
    pub exercise_id: String,
    pub exercise_name: String,
    pub set_index: u32,
    pub reps: Option<u32>,
    pub load_kg: Option<f64>,
    pub rpe: Option<f64>,
    #[serde(default)]
    pub done: bool,
}

/// Post-workout check (FB-2).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PostCheck {
    pub rpe: Option<f64>,
    /// 1-5 scales.
    pub energy: Option<u8>,
    pub sleep: Option<u8>,
    pub soreness: Option<u8>,
    #[serde(default)]
    pub pain: bool,
    pub pain_location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkoutLog {
    /// Client-generated id so offline writes are idempotent (LG-3).
    pub id: String,
    pub scheduled_id: Option<String>,
    pub date: NaiveDate,
    pub workout_name: String,
    pub status: LogStatus,
    #[serde(default)]
    pub sets: Vec<SetLog>,
    #[serde(default)]
    pub comment: String,
    pub check: Option<PostCheck>,
    /// The client's own note on each exercise (item id -> text): how it felt,
    /// anything bothering them. The coach sees it with the log.
    #[serde(default)]
    pub exercise_notes: BTreeMap<String, String>,
    /// How the workout went overall: 1 bad, 2 poor, 3 OK, 4 good, 5 great.
    #[serde(default)]
    pub rating: Option<u8>,
    /// Client clock at the time of the edit; last write wins.
    pub updated_at: DateTime<Utc>,
}

/// Longest exercise note, in characters.
pub const MAX_EXERCISE_NOTE: usize = 1000;

impl WorkoutLog {
    /// Trims exercise notes and drops empty ones; refuses a rating outside
    /// 1-5, an over-long note, or more notes than the workout could have.
    pub fn clean_feedback(&mut self) -> Result<(), String> {
        if self.rating.is_some_and(|r| !(1..=5).contains(&r)) {
            return Err("rating must be 1-5".into());
        }
        for note in self.exercise_notes.values_mut() {
            *note = note.trim().to_string();
            if note.chars().count() > MAX_EXERCISE_NOTE {
                return Err(format!("exercise notes must be {MAX_EXERCISE_NOTE} characters or fewer"));
            }
        }
        self.exercise_notes.retain(|_, n| !n.is_empty());
        if self.exercise_notes.len() > 100 {
            return Err("too many exercise notes".into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- feedback

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub author_id: String,
    pub author_role: Role,
    pub author_name: String,
    /// Optional anchor: a whole workout log, or a single exercise/set in it (FB-1).
    pub log_id: Option<String>,
    pub item_id: Option<String>,
    pub set_index: Option<u32>,
    pub text: String,
    /// A coach's video reply this message carries (VR-2), played from the thread.
    #[serde(default)]
    pub reply_id: Option<String>,
    /// A photo or voice note sent with (or as) the message.
    #[serde(default)]
    pub attachment: Option<Attachment>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Photo,
    Voice,
}

/// A file sent in a message, stored at `media/{coach}/{client}/messages/…`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub key: String,
    pub content_type: String,
    #[serde(default)]
    pub duration_sec: Option<f64>,
}

impl Comment {
    /// What a notification or inbox preview says about the message.
    pub fn preview_text(&self) -> String {
        match (&self.attachment, self.text.is_empty()) {
            (_, false) => self.text.clone(),
            (Some(a), true) if a.kind == AttachmentKind::Photo => "📷 Photo".into(),
            (Some(_), true) => "🎤 Voice note".into(),
            (None, true) => String::new(),
        }
    }
}

/// A coach-client conversation's summary, `COACH#{coach}` / `THREAD#{client}`:
/// the coach's inbox lists these, and each side's unread count lives here.
/// Updated on every message; the messages themselves are `COMMENT#` items.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageThread {
    pub client_id: String,
    pub client_name: String,
    pub last_at: Option<DateTime<Utc>>,
    pub last_text: String,
    pub last_author_role: Option<Role>,
    /// Messages from the client the coach hasn't opened yet.
    pub unread_coach: u32,
    /// Messages from the coach the client hasn't opened yet.
    pub unread_client: u32,
    pub coach_read_at: Option<DateTime<Utc>>,
    pub client_read_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------- body metrics

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
    Weight,
    Tape,
    Skinfold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinfoldRecord {
    pub method: SkinfoldMethod,
    pub equation: DensityEquation,
    /// Site name -> readings in mm (several readings are averaged).
    pub sites_mm: BTreeMap<String, Vec<f64>>,
    pub sex: Sex,
    pub age_years: f64,
    pub body_weight_kg: Option<f64>,
    pub result: BodyCompResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    /// `{date}-{random}` so ids sort by date.
    pub id: String,
    pub date: NaiveDate,
    pub kind: MetricKind,
    pub taken_by: Role,
    pub weight_kg: Option<f64>,
    pub tape_cm: Option<BTreeMap<String, f64>>,
    pub skinfold: Option<SkinfoldRecord>,
    #[serde(default)]
    pub notes: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PhotoPose {
    Front,
    Side,
    Back,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub id: String,
    pub date: NaiveDate,
    pub pose: PhotoPose,
    pub key: String,
    pub content_type: String,
    /// Who uploaded it; None on photos from before this was recorded.
    #[serde(default)]
    pub taken_by: Option<Role>,
    /// True until the browser's upload is confirmed (`POST .../confirm`).
    /// Photos from before confirming existed default to confirmed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------- videos (VS-1..VS-4)

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VideoStatus {
    /// Created; parts are being uploaded (possibly across sessions).
    Uploading,
    /// Upload confirmed; playable.
    Ready,
}

/// A client's video of one set (`CLIENT#{cid}` / `VIDEO#{id}`). Usually
/// trimmed and re-encoded on the device; when a browser can't, the original
/// is uploaded with `trim_start`/`trim_end` and the player keeps to them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    pub id: String,
    /// The workout log the set belongs to; logs get their ids on the device,
    /// so this can name a log that hasn't synced yet.
    pub log_id: String,
    pub item_id: String,
    pub set_index: u32,
    pub exercise_id: String,
    pub exercise_name: String,
    pub status: VideoStatus,
    pub key: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// Multipart upload in progress; cleared once the upload is complete.
    #[serde(default)]
    pub upload_id: Option<String>,
    pub part_size: i64,
    /// Set once a poster frame upload has landed.
    #[serde(default)]
    pub poster_key: Option<String>,
    #[serde(default)]
    pub duration_sec: Option<f64>,
    #[serde(default)]
    pub trim_start: Option<f64>,
    #[serde(default)]
    pub trim_end: Option<f64>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub reviewed_at: Option<DateTime<Utc>>,
}

/// A coach's reply to a set video (VR-1..VR-3), `CLIENT#{client}` /
/// `REPLY#{id}`: a recording of the coach going through the clip with their
/// voice over it, or just a written note. Uploaded in parts like a set video.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub id: String,
    pub video_id: String,
    pub coach_id: String,
    pub status: VideoStatus,
    /// Empty for a written reply with no recording.
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub size_bytes: i64,
    #[serde(default)]
    pub upload_id: Option<String>,
    #[serde(default)]
    pub part_size: i64,
    #[serde(default)]
    pub duration_sec: Option<f64>,
    #[serde(default)]
    pub note: String,
    /// Milliseconds from opening the review screen to sending (the BRD's
    /// five-minute review gate), as measured on the coach's device.
    #[serde(default)]
    pub review_ms: Option<u64>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub sent_at: Option<DateTime<Utc>>,
    /// When the client first played it.
    #[serde(default)]
    pub seen_at: Option<DateTime<Utc>>,
}

/// A ready video waiting for the coach's review (VS-4), stored as
/// `COACH#{coach}` / `REVIEW#{createdAt}#{videoId}` so a prefix query returns
/// the queue oldest first across all of the coach's clients. Carries what the
/// queue shows and plays, so listing it needs no per-video reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEntry {
    pub video_id: String,
    pub client_id: String,
    pub client_name: String,
    pub log_id: String,
    pub item_id: String,
    pub set_index: u32,
    pub exercise_name: String,
    pub key: String,
    #[serde(default)]
    pub poster_key: Option<String>,
    #[serde(default)]
    pub duration_sec: Option<f64>,
    #[serde(default)]
    pub trim_start: Option<f64>,
    #[serde(default)]
    pub trim_end: Option<f64>,
    pub created_at: DateTime<Utc>,
}

pub const TAPE_SITES: &[&str] = &[
    "neck", "chest", "waist", "hips", "armL", "armR", "thighL", "thighR", "calfL", "calfR",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workout_feedback_is_cleaned_and_bounded() {
        let mut log: WorkoutLog = serde_json::from_value(serde_json::json!({
            "id": "l", "scheduledId": null, "date": "2026-10-28", "workoutName": "W", "status": "completed",
            "check": null, "updatedAt": "2026-10-28T12:00:00Z",
            "exerciseNotes": { "a": "  knee ok ", "b": " " }, "rating": 5
        }))
        .unwrap();
        log.clean_feedback().unwrap();
        assert_eq!(log.exercise_notes.into_iter().collect::<Vec<_>>(), vec![("a".to_string(), "knee ok".to_string())]);

        let mut bad = WorkoutLog { rating: Some(0), exercise_notes: BTreeMap::new(), ..serde_json::from_value(serde_json::json!({
            "id": "l", "scheduledId": null, "date": "2026-10-28", "workoutName": "W", "status": "completed",
            "check": null, "updatedAt": "2026-10-28T12:00:00Z"
        })).unwrap() };
        assert!(bad.clean_feedback().is_err());
        bad.rating = Some(3);
        bad.exercise_notes.insert("a".into(), "x".repeat(MAX_EXERCISE_NOTE + 1));
        assert!(bad.clean_feedback().is_err());
    }

    fn item(sets: u32, rpe: Option<f64>) -> Item {
        Item {
            id: "i".into(),
            exercise_id: "squat".into(),
            exercise_name: "Squat".into(),
            sets,
            reps: "5".into(),
            load_kg: Some(100.0),
            load_pct: None,
            progression: None,
            tempo: None,
            rest_sec: Some(180),
            rpe,
            rir: None,
            notes: String::new(),
        }
    }

    fn program(week: u32, day: u32, it: Item) -> Program {
        Program {
            id: "p".into(),
            name: "Strength".into(),
            description: String::new(),
            weeks: 4,
            is_template: true,
            updated_at: None,
            workouts: vec![ProgramWorkout {
                week,
                day,
                workout: Workout {
                    id: "w".into(),
                    name: "Day A".into(),
                    notes: String::new(),
                    blocks: vec![Block {
                        id: "b".into(),
                        kind: BlockKind::Straight,
                        rounds: None,
                        duration_sec: None,
                        work_sec: None,
                        rest_sec: None,
                        items: vec![it],
                        notes: String::new(),
                    }],
                },
            }],
        }
    }

    #[test]
    fn validates_program_bounds() {
        assert!(program(1, 1, item(3, Some(8.0))).validate().is_ok());
        assert!(program(5, 1, item(3, None)).validate().is_err());
        assert!(program(1, 8, item(3, None)).validate().is_err());
        assert!(program(1, 1, item(0, None)).validate().is_err());
        assert!(program(1, 1, item(3, Some(11.0))).validate().is_err());
        let pct = |p: f64| Item { load_pct: Some(p), ..item(3, None) };
        assert!(program(1, 1, pct(75.0)).validate().is_ok());
        assert!(program(1, 1, pct(0.0)).validate().is_err());
        assert!(program(1, 1, pct(151.0)).validate().is_err());
    }

    #[test]
    fn program_json_is_camel_case() {
        let json = serde_json::to_value(program(1, 1, item(3, None))).unwrap();
        assert!(json["workouts"][0]["workout"]["blocks"][0]["items"][0]["exerciseName"].is_string());
        assert!(json["isTemplate"].is_boolean());
    }

    #[test]
    fn today_is_local_to_the_profile_time_zone() {
        use chrono::TimeZone;
        let mut p = Profile {
            user_id: "u".into(),
            role: Role::Client,
            coach_id: "c".into(),
            name: "N".into(),
            email: "n@x".into(),
            units: UnitSystem::Metric,
            sex: None,
            birth_date: None,
            business_name: None,
            time_zone: None,
            consented_at: None,
            suspended_at: None,
            created_at: Utc::now(),
        };
        // 5 pm Oct 27 in Los Angeles is already Oct 28 in UTC.
        let now = Utc.with_ymd_and_hms(2026, 10, 28, 0, 30, 0).unwrap();
        assert_eq!(p.today(now), NaiveDate::from_ymd_opt(2026, 10, 28).unwrap());
        p.time_zone = Some("America/Los_Angeles".into());
        assert_eq!(p.today(now), NaiveDate::from_ymd_opt(2026, 10, 27).unwrap());
        p.time_zone = Some("Pacific/Auckland".into());
        assert_eq!(p.today(now), NaiveDate::from_ymd_opt(2026, 10, 28).unwrap());
        p.time_zone = Some("Not/AZone".into());
        assert_eq!(p.today(now), NaiveDate::from_ymd_opt(2026, 10, 28).unwrap());
        assert!(valid_time_zone("Europe/London") && !valid_time_zone("Not/AZone"));
    }
}
