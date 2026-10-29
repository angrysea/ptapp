//! Scheduling (BZ-2; ptcore::sessions): a coach's availability, booking and
//! cancelling sessions, and each user's private calendar feed
//! (`GET /cal/{token}.ics`, no sign-in: the unguessable token is the key,
//! and resetting it turns the old link off).

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use ptcore::model::{Profile, Role, ScheduledWorkout};
use ptcore::sessions::{ics, open_slots, Availability, Event, Session, SessionStatus, When};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{client_scope, ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::keyed;
use crate::util::{invite_code, new_id};
use crate::AppState;

const HIGH: &str = "~";

fn zone(p: &Profile) -> Tz {
    p.time_zone.as_deref().and_then(|z| z.parse().ok()).unwrap_or(Tz::UTC)
}

/// "Thu Nov 5, 9:00 AM" on `p`'s clock.
fn local(t: DateTime<Utc>, p: &Profile) -> String {
    t.with_timezone(&zone(p)).format("%a %b %-d, %-I:%M %p").to_string()
}

async fn availability_of(st: &AppState, coach: &str) -> ApiResult<Availability> {
    Ok(st.store.get(&format!("COACH#{coach}"), "AVAILABILITY").await?.unwrap_or_default())
}

async fn profile(st: &AppState, user: &str) -> ApiResult<Profile> {
    st.store.get(&format!("USER#{user}"), "PROFILE").await?.ok_or(ApiError::NotFound("user"))
}

/// The coach's sessions overlapping [from, to).
async fn coach_sessions(st: &AppState, coach: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> ApiResult<Vec<Session>> {
    // A session starts at most four hours before anything it overlaps.
    let lo = (from - Duration::hours(4)).to_rfc3339();
    let all: Vec<Session> = st.store.query_between(&format!("COACH#{coach}"), &format!("SESSION#{lo}"), &format!("SESSION#{}{HIGH}", to.to_rfc3339()), true).await?;
    Ok(all.into_iter().filter(|s| s.end() > from && s.start < to).collect())
}

fn sk(s: &Session) -> String {
    format!("SESSION#{}#{}", s.start.to_rfc3339(), s.id)
}

// ---------------------------------------------------------------- availability

/// `GET /availability`: the coach's own, or a client's coach's.
pub async fn get_availability(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    if ctx.profile.role == Role::Admin {
        return Err(ApiError::Forbidden("coaches and clients only"));
    }
    let coach = profile(&st, &ctx.profile.coach_id).await?;
    let av = availability_of(&st, &coach.user_id).await?;
    Ok(Json(json!({ "availability": av, "timeZone": zone(&coach).name() })))
}

pub async fn put_availability(State(st): State<AppState>, ctx: Ctx, Json(mut av): Json<Availability>) -> ApiResult<Json<Availability>> {
    let coach = ctx.require_coach()?;
    av.validate().map_err(bad_request)?;
    st.store.put(&format!("COACH#{coach}"), "AVAILABILITY", &av).await?;
    Ok(Json(av))
}

#[derive(Deserialize)]
pub struct SlotsQuery {
    from: Option<NaiveDate>,
    days: Option<u32>,
}

/// `GET /clients/{cid}/slots?from=&days=14`: open slots with the client's coach.
pub async fn slots(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Query(q): Query<SlotsQuery>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let coach = profile(&st, &scope.client.coach_id).await?;
    let av = availability_of(&st, &coach.user_id).await?;
    let tz = zone(&coach);
    let now = Utc::now();
    let from = q.from.unwrap_or_else(|| now.with_timezone(&tz).date_naive());
    let days = q.days.unwrap_or(14).clamp(1, 28);
    let span_end = now + Duration::days(days as i64 + 2);
    let booked = coach_sessions(&st, &coach.user_id, now - Duration::days(1), span_end).await?;
    let open = open_slots(&av, tz, from, days, &booked, now);
    Ok(Json(json!({ "slots": open, "minutes": av.minutes, "timeZone": tz.name(), "location": av.location, "videoLink": av.video_link })))
}

// ---------------------------------------------------------------- sessions

#[derive(Deserialize)]
pub struct ListQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

/// `GET /sessions`: the coach's sessions with every client, or a client's own.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Query(q): Query<ListQuery>) -> ApiResult<Json<Vec<Session>>> {
    let now = Utc::now();
    let (from, to) = (q.from.unwrap_or(now - Duration::days(1)), q.to.unwrap_or(now + Duration::days(90)));
    let pk = match ctx.profile.role {
        Role::Coach => format!("COACH#{}", ctx.user_id()),
        Role::Client => format!("CLIENT#{}", ctx.user_id()),
        Role::Admin => return Err(ApiError::Forbidden("coaches and clients only")),
    };
    let all: Vec<Session> = st.store.query_between(&pk, &format!("SESSION#{}", from.to_rfc3339()), &format!("SESSION#{}{HIGH}", to.to_rfc3339()), true).await?;
    Ok(Json(all))
}

#[derive(Deserialize)]
pub struct Book {
    start: DateTime<Utc>,
    /// Coaches only; clients get the coach's session length.
    minutes: Option<u32>,
    #[serde(default)]
    notes: String,
}

/// `POST /clients/{cid}/sessions`: a client books an open slot; a coach
/// books any time that doesn't clash with their other sessions.
pub async fn book(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<Book>) -> ApiResult<Json<Session>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let coach = profile(&st, &scope.client.coach_id).await?;
    let av = availability_of(&st, &coach.user_id).await?;
    let now = Utc::now();
    let notes = body.notes.trim().to_string();
    if notes.chars().count() > 500 {
        return Err(bad_request("notes must be at most 500 characters"));
    }
    let minutes = if scope.by_coach { body.minutes.unwrap_or(av.minutes) } else { av.minutes };
    if !(15..=240).contains(&minutes) {
        return Err(bad_request("sessions are 15-240 minutes"));
    }
    if body.start < now - Duration::hours(1) || body.start > now + Duration::days(366) {
        return Err(bad_request("pick a time from now to a year ahead"));
    }
    let booked = coach_sessions(&st, &coach.user_id, body.start - Duration::days(1), body.start + Duration::days(1)).await?;
    if !scope.by_coach {
        let tz = zone(&coach);
        let day = body.start.with_timezone(&tz).date_naive();
        if !open_slots(&av, tz, day, 1, &booked, now).contains(&body.start) {
            return Err(ApiError::Conflict(json!({ "message": "that time isn't open any more; pick another" })));
        }
    } else if booked.iter().any(|s| s.overlaps(body.start, minutes)) {
        return Err(bad_request("that clashes with another session"));
    }
    // One booking per start time, even if two arrive at once.
    // The lock expires a day after the session (DynamoDB TTL).
    let lock = json!({ "at": now, "ttl": (body.start + Duration::days(1)).timestamp() });
    if !st.store.put_new(&format!("COACH#{}", coach.user_id), &format!("SLOTLOCK#{}", body.start.to_rfc3339()), &lock).await? {
        return Err(ApiError::Conflict(json!({ "message": "that time was just taken; pick another" })));
    }
    let s = Session {
        id: new_id()[..12].to_string(),
        client_id: scope.client.user_id.clone(),
        client_name: scope.client.name.clone(),
        coach_id: coach.user_id.clone(),
        coach_name: coach.name.clone(),
        start: body.start,
        minutes,
        location: av.location.clone(),
        video_link: av.video_link.clone(),
        notes,
        status: SessionStatus::Booked,
        created_at: now,
    };
    save(&st, &s).await?;
    tell_other(&st, &scope, &coach, &s, "Session booked").await;
    Ok(Json(s))
}

async fn save(st: &AppState, s: &Session) -> ApiResult<()> {
    st.store
        .batch_put(vec![keyed(&format!("CLIENT#{}", s.client_id), &sk(s), s)?, keyed(&format!("COACH#{}", s.coach_id), &sk(s), s)?])
        .await
}

/// Pushes a booking change to whichever side didn't make it.
async fn tell_other(st: &AppState, scope: &ClientScope, coach: &Profile, s: &Session, what: &str) {
    let (to, sender) = if scope.by_coach { (&scope.client, &coach.name) } else { (coach, &scope.client.name) };
    let text = format!("{what}: {}", local(s.start, to));
    let url = if to.role == Role::Coach { "#/schedule" } else { "#/sessions" };
    super::notify::notify_user(st, &to.user_id, sender, &text, url, &format!("session-{}", s.id)).await;
}

/// `POST /clients/{cid}/sessions/{sid}/cancel`: either side can cancel.
pub async fn cancel(State(st): State<AppState>, ctx: Ctx, Path((cid, sid)): Path<(String, String)>) -> ApiResult<Json<Session>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let mine: Vec<Session> = st.store.query_prefix(&scope.pk(), "SESSION#").await?;
    let mut s = mine.into_iter().find(|s| s.id == sid).ok_or(ApiError::NotFound("session"))?;
    if s.status == SessionStatus::Cancelled {
        return Ok(Json(s));
    }
    s.status = SessionStatus::Cancelled;
    save(&st, &s).await?;
    st.store.delete(&format!("COACH#{}", s.coach_id), &format!("SLOTLOCK#{}", s.start.to_rfc3339())).await?;
    let coach = profile(&st, &s.coach_id).await?;
    tell_other(&st, &scope, &coach, &s, "Session cancelled").await;
    Ok(Json(s))
}

// ---------------------------------------------------------------- calendar feed

#[derive(Serialize, Deserialize)]
struct CalToken {
    user_id: String,
}

/// The feed's path on the API; the app puts its API address in front.
fn feed_path(token: &str) -> String {
    format!("/cal/{token}.ics")
}

/// `GET /me/calendar`: the user's feed (its path), made on first use.
pub async fn my_calendar(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    if ctx.profile.role == Role::Admin {
        return Err(ApiError::Forbidden("coaches and clients only"));
    }
    let pk = format!("USER#{}", ctx.user_id());
    let token = match st.store.get::<Value>(&pk, "CALTOKEN").await? {
        Some(v) => v["token"].as_str().unwrap_or_default().to_string(),
        None => new_token(&st, ctx.user_id()).await?,
    };
    Ok(Json(json!({ "path": feed_path(&token) })))
}

/// `POST /me/calendar/reset`: a new link; the old one stops working.
pub async fn reset_calendar(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    if ctx.profile.role == Role::Admin {
        return Err(ApiError::Forbidden("coaches and clients only"));
    }
    forget_calendar(&st, ctx.user_id()).await?;
    let token = new_token(&st, ctx.user_id()).await?;
    Ok(Json(json!({ "path": feed_path(&token) })))
}

async fn new_token(st: &AppState, user: &str) -> ApiResult<String> {
    let token = format!("{}{}", invite_code(), new_id()).to_lowercase();
    st.store.put(&format!("CALTOKEN#{token}"), "CALTOKEN", &CalToken { user_id: user.to_string() }).await?;
    st.store.put(&format!("USER#{user}"), "CALTOKEN", &json!({ "token": token })).await?;
    Ok(token)
}

/// Turns off a user's feed link (calendar reset, account deletion).
pub async fn forget_calendar(st: &AppState, user: &str) -> ApiResult<()> {
    if let Some(v) = st.store.get::<Value>(&format!("USER#{user}"), "CALTOKEN").await?
        && let Some(t) = v["token"].as_str()
    {
        st.store.delete(&format!("CALTOKEN#{t}"), "CALTOKEN").await?;
    }
    st.store.delete(&format!("USER#{user}"), "CALTOKEN").await
}

/// `GET /cal/{token}.ics` (no sign-in): sessions, and for a client their
/// scheduled workouts too, from a month back to a year ahead.
pub async fn feed(State(st): State<AppState>, Path(file): Path<String>) -> ApiResult<impl IntoResponse> {
    let token = file.strip_suffix(".ics").unwrap_or(&file).to_lowercase();
    if token.len() > 80 || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::NotFound("calendar"));
    }
    let owner: CalToken = st.store.get(&format!("CALTOKEN#{token}"), "CALTOKEN").await?.ok_or(ApiError::NotFound("calendar"))?;
    let me = profile(&st, &owner.user_id).await?;
    let now = Utc::now();
    let (from, to) = (now - Duration::days(31), now + Duration::days(366));
    let pk = match me.role {
        Role::Coach => format!("COACH#{}", me.user_id),
        _ => format!("CLIENT#{}", me.user_id),
    };
    let sessions: Vec<Session> = st.store.query_between(&pk, &format!("SESSION#{}", from.to_rfc3339()), &format!("SESSION#{}{HIGH}", to.to_rfc3339()), true).await?;
    let mut events: Vec<Event> = sessions
        .iter()
        .map(|s| {
            let with = if me.role == Role::Coach { &s.client_name } else { &s.coach_name };
            let mut description = s.notes.clone();
            if let Some(link) = &s.video_link {
                description = format!("{description}\nVideo: {link}").trim().to_string();
            }
            Event {
                uid: format!("session-{}@ptcoach", s.id),
                summary: format!("Training session with {with}"),
                description,
                location: s.location.clone().or_else(|| s.video_link.clone()),
                when: When::Timed { start: s.start, end: s.end() },
                cancelled: s.status == SessionStatus::Cancelled,
            }
        })
        .collect();
    if me.role == Role::Client {
        let today = me.today(now);
        let sched: Vec<ScheduledWorkout> = st
            .store
            .query_between(&pk, &format!("SCHED#{}", today - Duration::days(31)), &format!("SCHED#{}{HIGH}", today + Duration::days(120)), true)
            .await?;
        events.extend(sched.iter().map(|s| Event {
            uid: format!("workout-{}@ptcoach", s.id),
            summary: format!("Workout: {}", s.workout.name),
            description: s.workout.blocks.iter().flat_map(|b| b.items.iter().map(|i| i.exercise_name.clone())).collect::<Vec<_>>().join(", "),
            location: None,
            when: When::AllDay(s.date),
            cancelled: false,
        }));
    }
    let name = if me.role == Role::Coach { "Training sessions".to_string() } else { format!("Training with {}", profile(&st, &me.coach_id).await.map(|c| c.name).unwrap_or_else(|_| "your coach".into())) };
    let body = ics(&name, &events, now);
    Ok(([(header::CONTENT_TYPE, "text/calendar; charset=utf-8"), (header::CACHE_CONTROL, "private, max-age=300")], body))
}
