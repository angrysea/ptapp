//! A coach who finds PT Coach without an invitation link can leave a
//! request (email + name) in the admin's queue; signup itself stays
//! invitation-only. The admin approves each request — which creates the
//! same single-use emailed invitation as a hand-made coach invite
//! (routes/invites.rs) — or denies it. The auto-approve switch (an admin
//! setting) approves each request on arrival instead. A request carries no
//! more than an email and a name: everything else belongs to the coach's
//! profile after they sign up.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One request per email; a denied email may ask again.
pub const MAX_EMAIL: usize = 80;
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
        if !self.email.contains('@') || self.email.chars().count() > MAX_EMAIL {
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
