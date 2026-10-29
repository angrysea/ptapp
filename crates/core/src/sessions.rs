//! Scheduling (BZ-2): a coach's weekly availability, booked sessions, the
//! open slots a client can book, and the iCalendar feed (RFC 5545) that
//! puts sessions (and a client's workouts) in their phone's calendar.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

pub const MAX_WINDOWS: usize = 21;

/// A weekly block the coach takes sessions in, in their own time zone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// 0 = Monday … 6 = Sunday.
    pub day: u8,
    pub start: NaiveTime,
    pub end: NaiveTime,
}

/// `COACH#{coach}` / `AVAILABILITY`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Availability {
    #[serde(default)]
    pub windows: Vec<Window>,
    /// Session length in minutes.
    pub minutes: u32,
    /// Clients can't book closer to the start than this.
    #[serde(default = "default_notice")]
    pub notice_hours: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_link: Option<String>,
}

fn default_notice() -> u32 {
    12
}

impl Default for Availability {
    fn default() -> Self {
        Availability { windows: Vec::new(), minutes: 60, notice_hours: default_notice(), location: None, video_link: None }
    }
}

impl Availability {
    pub fn validate(&mut self) -> Result<(), String> {
        if !(15..=240).contains(&self.minutes) {
            return Err("sessions are 15-240 minutes".into());
        }
        if self.notice_hours > 168 {
            return Err("booking notice is at most a week (168 hours)".into());
        }
        if self.windows.len() > MAX_WINDOWS {
            return Err(format!("up to {MAX_WINDOWS} weekly time blocks"));
        }
        for w in &self.windows {
            if w.day > 6 || w.end <= w.start {
                return Err("each time block needs a day and an end after its start".into());
            }
        }
        self.windows.sort_by_key(|w| (w.day, w.start));
        for v in [&mut self.location, &mut self.video_link] {
            *v = v.take().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        }
        if self.video_link.as_ref().is_some_and(|l| !l.starts_with("https://") || l.len() > 300) {
            return Err("the video link must be an https:// address".into());
        }
        if self.location.as_ref().is_some_and(|l| l.chars().count() > 200) {
            return Err("the location must be at most 200 characters".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Booked,
    Cancelled,
}

/// A booked session: `CLIENT#{client}` / `SESSION#{start}#{id}`, copied to
/// `COACH#{coach}` / `SESSION#{start}#{id}` for the coach's calendar.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub coach_id: String,
    pub coach_name: String,
    pub start: DateTime<Utc>,
    pub minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_link: Option<String>,
    #[serde(default)]
    pub notes: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
}

impl Session {
    pub fn end(&self) -> DateTime<Utc> {
        self.start + Duration::minutes(self.minutes as i64)
    }

    pub fn overlaps(&self, start: DateTime<Utc>, minutes: u32) -> bool {
        self.status == SessionStatus::Booked && start < self.end() && self.start < start + Duration::minutes(minutes as i64)
    }
}

/// Bookable starts over `days` days from `from` (in the coach's zone):
/// every session-length step inside the weekly windows, at least the
/// notice period from `now`, and clear of booked sessions.
pub fn open_slots(av: &Availability, tz: Tz, from: NaiveDate, days: u32, booked: &[Session], now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let earliest = now + Duration::hours(av.notice_hours as i64);
    let step = Duration::minutes(av.minutes as i64);
    let mut out = Vec::new();
    for offset in 0..days {
        let date = from + Duration::days(offset as i64);
        let weekday = date.weekday().num_days_from_monday() as u8;
        for w in av.windows.iter().filter(|w| w.day == weekday) {
            let mut t = date.and_time(w.start);
            let end = date.and_time(w.end);
            while t + step <= end {
                // A time skipped by a DST change has no instant; an ambiguous one takes the earlier.
                if let Some(start) = tz.from_local_datetime(&t).earliest().map(|s| s.with_timezone(&Utc))
                    && start >= earliest
                    && !booked.iter().any(|s| s.overlaps(start, av.minutes))
                {
                    out.push(start);
                }
                t += step;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------- iCalendar

/// One calendar entry: a timed session, or an all-day workout.
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: Option<String>,
    pub when: When,
    pub cancelled: bool,
}

pub enum When {
    Timed { start: DateTime<Utc>, end: DateTime<Utc> },
    AllDay(NaiveDate),
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace('\n', "\\n")
}

/// Folds a content line to 75 octets, as RFC 5545 requires.
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for ch in line.chars() {
        let n = ch.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += n;
    }
    out.push_str("\r\n");
}

fn utc(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// An iCalendar document named `name` holding `events`.
pub fn ics(name: &str, events: &[Event], now: DateTime<Utc>) -> String {
    let mut out = String::new();
    for l in ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Personal Training Coach//Schedule//EN", "CALSCALE:GREGORIAN", "METHOD:PUBLISH"] {
        fold(l, &mut out);
    }
    fold(&format!("X-WR-CALNAME:{}", escape(name)), &mut out);
    fold("REFRESH-INTERVAL;VALUE=DURATION:PT1H", &mut out);
    fold("X-PUBLISHED-TTL:PT1H", &mut out);
    for e in events {
        fold("BEGIN:VEVENT", &mut out);
        fold(&format!("UID:{}", escape(&e.uid)), &mut out);
        fold(&format!("DTSTAMP:{}", utc(now)), &mut out);
        match &e.when {
            When::Timed { start, end } => {
                fold(&format!("DTSTART:{}", utc(*start)), &mut out);
                fold(&format!("DTEND:{}", utc(*end)), &mut out);
            }
            When::AllDay(d) => {
                fold(&format!("DTSTART;VALUE=DATE:{}", d.format("%Y%m%d")), &mut out);
                fold(&format!("DTEND;VALUE=DATE:{}", (*d + Duration::days(1)).format("%Y%m%d")), &mut out);
            }
        }
        fold(&format!("SUMMARY:{}", escape(&e.summary)), &mut out);
        if !e.description.is_empty() {
            fold(&format!("DESCRIPTION:{}", escape(&e.description)), &mut out);
        }
        if let Some(l) = &e.location {
            fold(&format!("LOCATION:{}", escape(l)), &mut out);
        }
        fold(if e.cancelled { "STATUS:CANCELLED" } else { "STATUS:CONFIRMED" }, &mut out);
        fold("END:VEVENT", &mut out);
    }
    fold("END:VCALENDAR", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn av() -> Availability {
        // Mondays 9:00-11:00 and Wednesdays 17:00-18:30, hour-long sessions
        let mut a = Availability { windows: vec![Window { day: 2, start: t(17, 0), end: t(18, 30) }, Window { day: 0, start: t(9, 0), end: t(11, 0) }], ..Availability::default() };
        a.validate().unwrap();
        a
    }

    fn session(start: DateTime<Utc>, status: SessionStatus) -> Session {
        Session {
            id: "s".into(),
            client_id: "c".into(),
            client_name: "C".into(),
            coach_id: "k".into(),
            coach_name: "K".into(),
            start,
            minutes: 60,
            location: None,
            video_link: None,
            notes: String::new(),
            status,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn slots_follow_the_coach_zone_notice_and_bookings() {
        let tz: Tz = "America/Chicago".parse().unwrap();
        let monday = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(); // after DST ended: CST, UTC-6
        let now = Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap();
        let slots = open_slots(&av(), tz, monday, 3, &[], now);
        let hours: Vec<String> = slots.iter().map(|s| s.with_timezone(&tz).format("%a %H:%M").to_string()).collect();
        assert_eq!(hours, ["Mon 09:00", "Mon 10:00", "Wed 17:00"], "18:00 doesn't fit a whole hour before 18:30");
        assert_eq!(slots[0], Utc.with_ymd_and_hms(2026, 11, 2, 15, 0, 0).unwrap());
        // a booking blocks its slot; a cancelled one doesn't
        let booked = [session(slots[1], SessionStatus::Booked), session(slots[0], SessionStatus::Cancelled)];
        assert_eq!(open_slots(&av(), tz, monday, 3, &booked, now).len(), 2);
        // the notice period hides slots that start too soon
        let just_before = slots[0] - Duration::hours(2);
        assert_eq!(open_slots(&av(), tz, monday, 1, &[], just_before).len(), 0, "both Monday slots are within 12 hours");
    }

    #[test]
    fn validation() {
        let mut bad = Availability { windows: vec![Window { day: 0, start: t(10, 0), end: t(9, 0) }], ..Availability::default() };
        assert!(bad.validate().is_err());
        let mut bad = Availability { video_link: Some("http://zoom.us/x".into()), ..Availability::default() };
        assert!(bad.validate().is_err());
        let mut bad = Availability { minutes: 5, ..Availability::default() };
        assert!(bad.validate().is_err());
        assert_eq!(av().windows[0].day, 0, "sorted by day");
    }

    #[test]
    fn ics_escapes_folds_and_marks_cancellations() {
        let now = Utc.with_ymd_and_hms(2026, 10, 29, 12, 0, 0).unwrap();
        let start = Utc.with_ymd_and_hms(2026, 11, 2, 15, 0, 0).unwrap();
        let events = [
            Event { uid: "s1@ptcoach".into(), summary: "Session with Sam; legs, core".into(), description: "Line one\nline two ".repeat(8), location: Some("Gym, Main St".into()), when: When::Timed { start, end: start + Duration::hours(1) }, cancelled: false },
            Event { uid: "w1@ptcoach".into(), summary: "Workout: Heavy".into(), description: String::new(), location: None, when: When::AllDay(NaiveDate::from_ymd_opt(2026, 11, 3).unwrap()), cancelled: true },
        ];
        let doc = ics("PT Coach", &events, now);
        assert!(doc.starts_with("BEGIN:VCALENDAR\r\n") && doc.ends_with("END:VCALENDAR\r\n"));
        assert!(doc.contains("SUMMARY:Session with Sam\\; legs\\, core\r\n"));
        assert!(doc.contains("DTSTART:20261102T150000Z\r\nDTEND:20261102T160000Z\r\n"));
        assert!(doc.contains("DTSTART;VALUE=DATE:20261103\r\nDTEND;VALUE=DATE:20261104\r\n"));
        assert!(doc.contains("STATUS:CANCELLED"));
        assert!(doc.split("\r\n").all(|l| l.len() <= 75), "lines are folded");
        assert!(doc.contains("\r\n "), "the long description was folded");
    }
}
