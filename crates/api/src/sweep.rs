//! The hourly sweep (bin/sweep.rs, run by EventBridge Scheduler): for every
//! coach's clients, sends what's due in the client's own time zone:
//! check-in and workout reminders (push), the coach's automated messages
//! (into the thread, like a message the coach typed), and missed-workout
//! alerts for the coach (LG-7, pushed). Each send is recorded first
//! (`CLIENT#{id}` / `SENT#…`, expiring after 40 days), so a retried or
//! overlapping sweep never sends anything twice.

use chrono::{DateTime, Utc};
use futures::{stream, StreamExt};
use ptcore::automation::{local_time, personalize, Automation};
use ptcore::checkin::{week_of, Checkin};
use ptcore::model::{CoachEntry, Comment, Profile, Role, RosterEntry, ScheduledWorkout};
use serde::Serialize;

use crate::auth::ClientScope;
use crate::error::ApiResult;
use crate::routes::invites::ADMIN_PK;
use crate::routes::{alerts, clients, messages, notify};
use crate::util::new_id;
use crate::AppState;

/// Clients handled at once, as the roster does.
const CONCURRENCY: usize = 5;
const SENT_DAYS: i64 = 40;

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub coaches: usize,
    pub clients: usize,
    pub checkin_reminders: usize,
    pub workout_reminders: usize,
    pub messages: usize,
    pub missed_alerts: usize,
    pub errors: usize,
}

#[derive(Default)]
struct Sent {
    checkin: bool,
    workout: bool,
    messages: usize,
    missed: bool,
}

/// Records that `what` was sent; false if it already was.
async fn first_time(st: &AppState, client_pk: &str, what: &str, now: DateTime<Utc>) -> ApiResult<bool> {
    let marker = serde_json::json!({ "at": now, "ttl": now.timestamp() + SENT_DAYS * 24 * 3600 });
    st.store.put_new(client_pk, &format!("SENT#{what}"), &marker).await
}

pub async fn run(st: &AppState, now: DateTime<Utc>) -> Report {
    let mut report = Report::default();
    let coaches: Vec<CoachEntry> = match st.store.query_prefix(ADMIN_PK, "COACH#").await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("sweep: listing coaches failed: {e:?}");
            report.errors += 1;
            return report;
        }
    };
    for coach in coaches.into_iter().filter(|c| c.suspended_at.is_none()) {
        report.coaches += 1;
        if let Err(e) = sweep_coach(st, &coach, now, &mut report).await {
            tracing::error!(coach = %coach.coach_id, "sweep failed: {e:?}");
            report.errors += 1;
        }
    }
    report
}

async fn sweep_coach(st: &AppState, coach: &CoachEntry, now: DateTime<Utc>, report: &mut Report) -> ApiResult<()> {
    let coach_pk = format!("COACH#{}", coach.coach_id);
    let Some(coach_profile) = st.store.get::<Profile>(&format!("USER#{}", coach.coach_id), "PROFILE").await? else { return Ok(()) };
    let automation: Automation = st.store.get(&coach_pk, "AUTOMATION").await?.unwrap_or_default();
    let threshold = alerts::settings(st, &coach.coach_id).await?.missed_in_a_row;
    let roster: Vec<RosterEntry> = st.store.query_prefix(&coach_pk, "CLIENT#").await?;
    let results: Vec<ApiResult<Sent>> = stream::iter(roster)
        .map(|entry| {
            let (automation, coach_profile) = (&automation, &coach_profile);
            async move { sweep_client(st, coach_profile, automation, threshold, &entry, now).await }
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    for r in results {
        report.clients += 1;
        match r {
            Ok(s) => {
                report.checkin_reminders += s.checkin as usize;
                report.workout_reminders += s.workout as usize;
                report.messages += s.messages;
                report.missed_alerts += s.missed as usize;
            }
            Err(e) => {
                tracing::warn!(coach = %coach.coach_id, "sweep: a client failed: {e:?}");
                report.errors += 1;
            }
        }
    }
    Ok(())
}

async fn sweep_client(
    st: &AppState,
    coach: &Profile,
    automation: &Automation,
    threshold: u32,
    entry: &RosterEntry,
    now: DateTime<Utc>,
) -> ApiResult<Sent> {
    let mut sent = Sent::default();
    let Some(client) = st.store.get::<Profile>(&format!("USER#{}", entry.client_id), "PROFILE").await? else { return Ok(sent) };
    let pk = format!("CLIENT#{}", client.user_id);
    let local = local_time(now, client.time_zone.as_deref());
    let today = local.date();
    let first_name = client.name.split_whitespace().next().unwrap_or("").to_string();
    let (sched, logs) = clients::recent(st, &pk, today, 28).await?;

    // Check-in reminder: this week's isn't in yet.
    if automation.checkin_reminder.as_ref().is_some_and(|w| w.due(local)) {
        let week = week_of(today);
        let done = st.store.get::<Checkin>(&pk, &format!("CHECKIN#{week}")).await?.is_some();
        if !done && first_time(st, &pk, &format!("checkin#{week}"), now).await? {
            notify::notify_user(st, &client.user_id, &coach.name, &personalize("Time for your weekly check-in, {name}.", &first_name), "#/checkin", "checkin").await;
            sent.checkin = true;
        }
    }

    // Workout reminder: something is scheduled today and not logged yet.
    if automation.workout_reminder.as_ref().is_some_and(|w| w.due(local)) {
        let todays: Vec<&ScheduledWorkout> = sched
            .iter()
            .filter(|s| s.date == today && !logs.iter().any(|l| l.scheduled_id.as_deref() == Some(s.id.as_str())))
            .collect();
        if let Some(s) = todays.first()
            && first_time(st, &pk, &format!("workout#{today}"), now).await?
        {
            let url = format!("#/workout/{}/{}", s.date, s.id);
            notify::notify_user(st, &client.user_id, &coach.name, &format!("Today: {}", s.workout.name), &url, "workout").await;
            sent.workout = true;
        }
    }

    // The coach's automated messages, into the thread as if typed.
    let scope = ClientScope { client: client.clone(), by_coach: true };
    for m in automation.messages.iter().filter(|m| m.active && m.when.due(local)) {
        if !first_time(st, &pk, &format!("msg#{}#{today}", m.id), now).await? {
            continue;
        }
        let c = Comment {
            id: new_id(),
            author_id: coach.user_id.clone(),
            author_role: Role::Coach,
            author_name: coach.name.clone(),
            log_id: None,
            item_id: None,
            set_index: None,
            text: personalize(&m.text, &first_name),
            reply_id: None,
            attachment: None,
            video_id: None,
            about: None,
            created_at: now,
        };
        st.store.put(&pk, &format!("COMMENT#{}#{}", c.created_at.to_rfc3339(), c.id), &c).await?;
        messages::on_message(st, &scope, &c).await;
        sent.messages += 1;
    }

    // Missed workouts: file the alert and tell the coach (LG-7).
    if let Some(alert) = alerts::check_missed(st, &coach.user_id, threshold, entry, &sched, &logs, today).await? {
        let url = format!("#/c/{}/calendar", client.user_id);
        notify::notify_user(st, &coach.user_id, &client.name, &alert.text, &url, &format!("alert-{}", client.user_id)).await;
        sent.missed = true;
    }
    Ok(sent)
}
