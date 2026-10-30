//! Service credits: what a client may use of each coached service. A coach
//! defines plans (bundles of so many credits per service, or unlimited, for
//! a number of days) and grants them to clients; each use of a service takes
//! one credit from the client's grants for it. A coach turns enforcement on
//! for their clients (`Settings::enforce`); until then nothing is refused,
//! but grants that exist are still counted down. Stripe will later grant
//! plans on purchase the same way a coach does by hand.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// The services credits are counted for.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Service {
    /// A message the client sends their coach (text or voice note).
    Message,
    /// A photo the client sends for the coach to look at.
    PhotoReview,
    /// A set video the coach reviews (held from upload, used on review).
    VideoReview,
    /// A booked session with the coach (returned if it's cancelled).
    Session,
    /// A program assigned to the client: counted, never refused.
    Programming,
}

impl Service {
    pub const ALL: [Service; 5] = [Service::Message, Service::PhotoReview, Service::VideoReview, Service::Session, Service::Programming];

    pub fn label(self) -> &'static str {
        match self {
            Service::Message => "messages",
            Service::PhotoReview => "photo reviews",
            Service::VideoReview => "video reviews",
            Service::Session => "sessions",
            Service::Programming => "programs",
        }
    }

    /// Whether running out stops the service. Programming is only counted:
    /// a coach is never stopped from assigning a program.
    pub fn enforced(self) -> bool {
        self != Service::Programming
    }
}

/// Most credits of one service in one plan line or grant.
pub const MAX_QUANTITY: u32 = 10_000;
/// Longest a plan may run.
pub const MAX_DAYS: u32 = 3660;

/// One line of a plan: so many credits of a service, or unlimited.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanLine {
    pub service: Service,
    /// `None`: unlimited.
    #[serde(default)]
    pub quantity: Option<u32>,
}

/// `COACH#{coach}` / `PLAN#{id}`: a bundle of credits a coach grants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub lines: Vec<PlanLine>,
    /// Days the credits last from the day they're granted; `None`: no expiry.
    #[serde(default)]
    pub valid_days: Option<u32>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

impl Plan {
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_string();
        self.description = self.description.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > 80 {
            return Err("a plan's name must be 1-80 characters".into());
        }
        if self.description.chars().count() > 500 {
            return Err("a plan's description must be at most 500 characters".into());
        }
        if self.lines.is_empty() {
            return Err("a plan needs at least one service".into());
        }
        let mut seen = vec![];
        for l in &self.lines {
            if seen.contains(&l.service) {
                return Err(format!("{} appear twice in the plan", l.service.label()));
            }
            seen.push(l.service);
            if l.quantity.is_some_and(|q| q == 0 || q > MAX_QUANTITY) {
                return Err(format!("{}: 1-{MAX_QUANTITY} credits, or unlimited", l.service.label()));
            }
        }
        if self.valid_days.is_some_and(|d| d == 0 || d > MAX_DAYS) {
            return Err(format!("a plan lasts 1-{MAX_DAYS} days, or doesn't expire"));
        }
        Ok(())
    }
}

/// `CLIENT#{id}` / `CREDIT#{id}`: credits of one service given to a client,
/// from a plan or by hand. Uses count up `used`; an unlimited grant counts
/// too, for the record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub id: String,
    pub service: Service,
    /// `None`: unlimited.
    pub total: Option<u32>,
    #[serde(default)]
    pub used: u32,
    /// The last day the credits can be used; `None`: no expiry.
    #[serde(default)]
    pub expires_on: Option<NaiveDate>,
    /// The plan it came from, if any (its name as it was).
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub plan_name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    pub granted_at: DateTime<Utc>,
}

impl Grant {
    pub fn active(&self, today: NaiveDate) -> bool {
        self.expires_on.is_none_or(|d| today <= d)
    }

    /// Credits left; `None`: unlimited.
    pub fn remaining(&self) -> Option<u32> {
        self.total.map(|t| t.saturating_sub(self.used))
    }

    pub fn usable(&self, today: NaiveDate) -> bool {
        self.active(today) && self.remaining() != Some(0)
    }
}

/// The grants a plan becomes for a client granted it `today`; `expires_on`
/// overrides the plan's own length.
pub fn grants_for(plan: &Plan, today: NaiveDate, now: DateTime<Utc>, expires_on: Option<NaiveDate>, mut new_id: impl FnMut() -> String) -> Vec<Grant> {
    let expiry = expires_on.or_else(|| plan.valid_days.map(|d| today + chrono::Duration::days(d as i64 - 1)));
    plan.lines
        .iter()
        .map(|l| Grant {
            id: new_id(),
            service: l.service,
            total: l.quantity,
            used: 0,
            expires_on: expiry,
            plan_id: Some(plan.id.clone()),
            plan_name: Some(plan.name.clone()),
            note: None,
            granted_at: now,
        })
        .collect()
}

/// The grant a use of `service` comes out of: the usable grant that
/// expires first, counted ones before unlimited ones (so nothing that will
/// expire is wasted while an unlimited grant covers the rest).
pub fn pick(grants: &[Grant], service: Service, today: NaiveDate) -> Option<&Grant> {
    grants
        .iter()
        .filter(|g| g.service == service && g.usable(today))
        .min_by_key(|g| (g.total.is_none(), g.expires_on.unwrap_or(NaiveDate::MAX), g.granted_at))
}

/// What a client has of one service right now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    pub service: Service,
    /// Credits left across usable grants; `None`: unlimited.
    pub remaining: Option<u32>,
    /// Credits held for requests not yet delivered (videos awaiting review).
    pub held: u32,
    /// Whether the client can use the service once more.
    pub available: bool,
    /// When the soonest of those credits expire.
    pub next_expiry: Option<NaiveDate>,
    /// Whether any grant (even used up or expired) exists for it.
    pub granted: bool,
}

/// Balances for every service. `held` counts requests that hold a credit
/// until delivered, by service.
pub fn balances(grants: &[Grant], today: NaiveDate, held: &[(Service, u32)]) -> Vec<Balance> {
    Service::ALL
        .iter()
        .map(|&service| {
            let usable: Vec<&Grant> = grants.iter().filter(|g| g.service == service && g.usable(today)).collect();
            let unlimited = usable.iter().any(|g| g.total.is_none());
            let left: u32 = usable.iter().filter_map(|g| g.remaining()).sum();
            let held_n = held.iter().filter(|(s, _)| *s == service).map(|(_, n)| *n).sum();
            Balance {
                service,
                remaining: if unlimited { None } else { Some(left) },
                held: held_n,
                available: unlimited || left > held_n,
                next_expiry: usable.iter().filter_map(|g| g.expires_on).min(),
                granted: grants.iter().any(|g| g.service == service),
            }
        })
        .collect()
}

/// `CLIENT#{id}` / `LEDGER#{at}#{id}`: one use, refund or grant, for the
/// history the coach and client see.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    pub id: String,
    pub service: Service,
    /// Credits added (grant, refund) or taken (a use: -1). Unlimited grants
    /// are recorded as 0 with `unlimited`.
    pub change: i64,
    #[serde(default)]
    pub unlimited: bool,
    pub reason: String,
    /// The grant it came out of or went into, if any.
    #[serde(default)]
    pub grant_id: Option<String>,
    pub at: DateTime<Utc>,
}

/// `COACH#{coach}` / `CREDIT_SETTINGS`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Stop clients using a service they have no credits for.
    #[serde(default)]
    pub enforce: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    fn grant(id: &str, service: Service, total: Option<u32>, used: u32, expires: Option<u32>) -> Grant {
        Grant { id: id.into(), service, total, used, expires_on: expires.map(d), plan_id: None, plan_name: None, note: None, granted_at: Utc::now() }
    }

    #[test]
    fn plans_validate_their_lines() {
        let line = |service, quantity| PlanLine { service, quantity };
        let mut ok = Plan { id: String::new(), name: " Monthly ".into(), description: String::new(), lines: vec![line(Service::Message, None), line(Service::VideoReview, Some(4))], valid_days: Some(30), updated_at: None };
        assert!(ok.validate().is_ok());
        assert_eq!(ok.name, "Monthly");
        let mut twice = Plan { lines: vec![line(Service::Message, Some(1)), line(Service::Message, Some(2))], ..ok.clone() };
        assert!(twice.validate().is_err());
        let mut zero = Plan { lines: vec![line(Service::Session, Some(0))], ..ok.clone() };
        assert!(zero.validate().is_err(), "0 credits is not a line");
        let mut empty = Plan { lines: vec![], ..ok.clone() };
        assert!(empty.validate().is_err());
        let mut forever = Plan { valid_days: None, ..ok };
        assert!(forever.validate().is_ok(), "no expiry is allowed");
    }

    #[test]
    fn a_plan_becomes_one_grant_per_line_with_its_expiry() {
        let plan = Plan { id: "p".into(), name: "Monthly".into(), description: String::new(), lines: vec![PlanLine { service: Service::Message, quantity: None }, PlanLine { service: Service::Session, quantity: Some(4) }], valid_days: Some(30), updated_at: None };
        let mut n = 0;
        let gs = grants_for(&plan, d(1), Utc::now(), None, || { n += 1; format!("g{n}") });
        assert_eq!(gs.len(), 2);
        assert_eq!(gs[0].expires_on, Some(d(30)), "30 days counting the first");
        assert_eq!((gs[1].service, gs[1].total), (Service::Session, Some(4)));
        let custom = grants_for(&plan, d(1), Utc::now(), Some(d(15)), || "x".into());
        assert_eq!(custom[0].expires_on, Some(d(15)), "the coach's date wins");
    }

    #[test]
    fn uses_come_from_the_soonest_expiring_counted_grant() {
        let gs = vec![
            grant("unlimited", Service::Message, None, 0, None),
            grant("late", Service::Message, Some(5), 0, Some(28)),
            grant("soon", Service::Message, Some(5), 0, Some(10)),
            grant("expired", Service::Message, Some(5), 0, Some(2)),
            grant("empty", Service::Message, Some(3), 3, Some(5)),
        ];
        assert_eq!(pick(&gs, Service::Message, d(4)).unwrap().id, "soon");
        assert_eq!(pick(&gs, Service::Message, d(29)).unwrap().id, "unlimited", "counted ones expired");
        assert!(pick(&gs, Service::Session, d(4)).is_none());
    }

    #[test]
    fn balances_sum_counted_credits_and_subtract_holds() {
        let gs = vec![
            grant("a", Service::VideoReview, Some(3), 1, Some(30)),
            grant("b", Service::VideoReview, Some(2), 0, Some(20)),
            grant("old", Service::VideoReview, Some(9), 0, Some(1)),
            grant("m", Service::Message, None, 40, None),
        ];
        let b = balances(&gs, d(5), &[(Service::VideoReview, 4)]);
        let video = b.iter().find(|x| x.service == Service::VideoReview).unwrap();
        assert_eq!((video.remaining, video.held, video.available, video.next_expiry), (Some(4), 4, false, Some(d(20))), "4 left, all held by videos awaiting review");
        let msg = b.iter().find(|x| x.service == Service::Message).unwrap();
        assert_eq!((msg.remaining, msg.available), (None, true), "unlimited");
        let session = b.iter().find(|x| x.service == Service::Session).unwrap();
        assert_eq!((session.remaining, session.available, session.granted), (Some(0), false, false));
        assert!(!Service::Programming.enforced() && Service::Session.enforced());
    }
}
