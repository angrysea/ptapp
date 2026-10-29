//! A coach who finds PT Coach without an invitation link can leave a
//! request (email + name) in the admin's queue; signup itself stays
//! invitation-only. The admin approves each request — which creates the
//! same single-use emailed invitation as a hand-made coach invite
//! (routes/invites.rs) — or denies it. The auto-approve switch (an admin
//! setting) approves each request on arrival instead. A request carries no
//! more than an email and a name: everything else belongs to the coach's
//! profile after they sign up.
//!
//! The form is the one anonymous way in, so it is guarded (routes/requests.rs):
//! a hidden field only bots fill; Cloudflare Turnstile's human check when it
//! is set up (a minimum time on the page otherwise); a few requests an hour
//! per sender; the same answer for every email (it never says who was
//! invited); denials that stick for a while; and a cap on the pending queue.
//! API Gateway also throttles the route. Auto-approve needs Turnstile.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// One request per email; a denied email may ask again after `DENY_DAYS`.
pub const MAX_EMAIL: usize = 80;
/// A denied email can't go back on the queue for this long.
pub const DENY_DAYS: i64 = 30;
/// Pending requests beyond this are dropped (silently), so a flood can't
/// bury the admin's queue.
pub const MAX_PENDING: usize = 500;
/// Faster than this from opening the page to sending is a script (the
/// check used when Turnstile isn't set up; the page reports the time).
pub const MIN_FILL_MS: u64 = 3000;
/// Requests one sender (IP address) may make in an hour.
pub const MAX_PER_SENDER_HOUR: u32 = 5;

/// A plausible address: one `@`, a local part, and a dotted domain of
/// letters, digits, dots and hyphens; no spaces or control characters.
pub fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else { return false };
    let label_ok = |l: &str| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    email.chars().count() <= MAX_EMAIL
        && !local.is_empty()
        && local.len() <= 64
        && local.chars().all(|c| c.is_ascii_graphic() && !"@\"(),:;<>[]".contains(c))
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && domain.contains('.')
        && domain.split('.').all(label_ok)
        && domain.rsplit('.').next().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
}
/// Like admin's clean_name, so approval can't fail on the name.
pub const MAX_NAME: usize = 100;

/// Where a request has got to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Waiting for the admin.
    Pending,
    /// Approved: the invitation was created and emailed.
    Invited,
    /// The admin said no; the email may request again.
    Denied,
}

/// The admin's queue settings (pk `ADMIN`, sk `SETTINGS#AUTO`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub auto_approve: bool,
    pub updated_at: DateTime<Utc>,
}

impl Settings {
    /// Off until the admin switches it: requests default to human review.
    pub fn off() -> Settings {
        Settings { auto_approve: false, updated_at: Utc::now() }
    }
}

/// One queue entry, keyed by email so one email holds one request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub email: String,
    pub name: String,
    pub status: Status,
    pub requested_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_code: Option<String>,
    /// DynamoDB expiry, refreshed on every transition (a stale queue ages
    /// itself out).
    pub ttl: i64,
}

impl Request {
    pub fn validate(&mut self) -> Result<(), String> {
        self.email = self.email.trim().to_lowercase();
        if !valid_email(&self.email) {
            return Err("an email is required, at most 80 characters".into());
        }
        self.name = self.name.trim().to_string();
        if self.name.chars().count() > MAX_NAME {
            return Err("a name is at most 100 characters".into());
        }
        Ok(())
    }

    /// Approved: record which invitation went out, so re-approval can be
    /// refused and the admin can find the email again.
    pub fn invited(&mut self, code: String) {
        self.status = Status::Invited;
        self.invite_code = Some(code);
    }

    pub fn denied(&mut self) {
        self.status = Status::Denied;
    }

    /// A denied email may ask again once the denial is `DENY_DAYS` old.
    pub fn may_ask_again(&self, now: DateTime<Utc>) -> bool {
        self.status == Status::Denied && now - self.updated_at >= Duration::days(DENY_DAYS)
    }

    /// A denied email re-reading the form goes back on the queue.
    pub fn re_request(&mut self) {
        self.status = Status::Pending;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_cleans_and_bounds() {
        let mut r = Request { email: "  Wannabe@Gym.Co  ".into(), name: "  Coach Wanna ".into(), status: Status::Pending, requested_at: Utc::now(), updated_at: Utc::now(), invite_code: None, ttl: 0 };
        assert!(r.validate().is_ok());
        assert_eq!(r.email, "wannabe@gym.co");
        assert_eq!(r.name, "Coach Wanna");
        let mut bad = Request { email: "no-at-sign".into(), name: "".into(), status: Status::Pending, requested_at: Utc::now(), updated_at: Utc::now(), invite_code: None, ttl: 0 };
        assert_eq!(bad.validate(), Err("an email is required, at most 80 characters".into()));
    }

    #[test]
    fn emails_must_look_real() {
        for ok in ["a@b.co", "first.last+tag@gym-club.example.com", "x_y@sub.domain.io"] {
            assert!(valid_email(ok), "{ok}");
        }
        for bad in ["nope", "@b.co", "a@", "a@b", "a@b.c", "a b@c.co", "a@b..co", "a@-b.co", "a@b.co\n", ".a@b.co", "a..b@c.co", "a@b.c0m", "<a>@b.co"] {
            assert!(!valid_email(bad), "{bad:?}");
        }
    }

    #[test]
    fn denials_stick_for_a_while() {
        let now = Utc::now();
        let mut r = Request { email: "a@b.co".into(), name: "".into(), status: Status::Denied, requested_at: now, updated_at: now - Duration::days(DENY_DAYS - 1), invite_code: None, ttl: 0 };
        assert!(!r.may_ask_again(now));
        r.updated_at = now - Duration::days(DENY_DAYS);
        assert!(r.may_ask_again(now));
        r.status = Status::Pending;
        assert!(!r.may_ask_again(now), "only denials");
    }

    #[test]
    fn a_request_walks_the_queue() {
        let mut r = Request { email: "a@b.co".into(), name: "".into(), status: Status::Pending, requested_at: Utc::now(), updated_at: Utc::now(), invite_code: None, ttl: 0 };
        r.invited("ABC123XYZ0".into());
        assert_eq!(r.status, Status::Invited);
        assert_eq!(r.invite_code.as_ref().unwrap(), "ABC123XYZ0");
        r.denied();
        assert_eq!(r.status, Status::Denied);
        r.re_request();
        assert_eq!(r.status, Status::Pending);
        assert_eq!(r.invite_code.as_ref().unwrap(), "ABC123XYZ0", "an old code stays until approval replaces it");
    }
}
