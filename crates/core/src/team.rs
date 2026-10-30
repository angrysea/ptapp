//! Assistant coaches: a head coach's team. An assistant joins by invitation,
//! works in the head coach's business (their library of programs, recipes
//! and meal plans), and reaches only the clients the head coach assigns
//! them, for only the kinds of work they're granted (`Perm`). Clients see
//! them by name and title.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What an assistant may do, each for their assigned clients only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Perm {
    /// Programs (the head coach's library and clients' copies), assigning
    /// them, 1RMs, custom exercises.
    Programming,
    /// Recipes, recipe books, meal plans, the food journal and targets.
    Nutrition,
    /// The client's conversation, photos and voice notes included.
    Messaging,
    /// Set videos waiting for review, video replies.
    Review,
    /// Read-only: workouts, logs, body metrics and photos, check-ins, habits
    /// and the food journal.
    Progress,
    /// The intake questionnaire and health screening.
    Health,
}

impl Perm {
    pub const ALL: [Perm; 6] = [Perm::Programming, Perm::Nutrition, Perm::Messaging, Perm::Review, Perm::Progress, Perm::Health];
}

pub const DEFAULT_TITLE: &str = "Assistant Coach";
pub const MAX_TITLE: usize = 40;
pub const MAX_TEAM: usize = 25;

/// `COACH#{head}` / `ASSISTANT#{userId}`: a team member, written when they
/// accept their invitation. Removing it removes all their access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub user_id: String,
    pub name: String,
    pub email: String,
    pub title: String,
    #[serde(default)]
    pub perms: Vec<Perm>,
    /// Clients the head coach has assigned them.
    #[serde(default)]
    pub clients: Vec<String>,
    pub joined_at: DateTime<Utc>,
}

impl Member {
    pub fn can(&self, p: Perm) -> bool {
        self.perms.contains(&p)
    }

    pub fn assigned(&self, client_id: &str) -> bool {
        self.clients.iter().any(|c| c == client_id)
    }

    /// How clients see them: "Maria · Nutrition Coach".
    pub fn display(&self) -> String {
        format!("{} · {}", self.name, self.title)
    }
}

/// A title as typed: trimmed, the default when empty.
pub fn clean_title(t: Option<&str>) -> Result<String, String> {
    let t = t.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(DEFAULT_TITLE);
    if t.chars().count() > MAX_TITLE {
        return Err(format!("a title is at most {MAX_TITLE} characters"));
    }
    Ok(t.to_string())
}

/// Permissions without repeats, in a fixed order.
pub fn clean_perms(mut p: Vec<Perm>) -> Vec<Perm> {
    p.sort();
    p.dedup();
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_titles_and_perms() {
        assert_eq!(clean_title(None).unwrap(), "Assistant Coach");
        assert_eq!(clean_title(Some("  ")).unwrap(), "Assistant Coach", "blank means the default");
        assert_eq!(clean_title(Some(" Nutrition Coach ")).unwrap(), "Nutrition Coach");
        assert!(clean_title(Some(&"x".repeat(41))).is_err());
        assert_eq!(clean_perms(vec![Perm::Review, Perm::Nutrition, Perm::Review]), vec![Perm::Nutrition, Perm::Review]);
        let m = Member { user_id: "a".into(), name: "Maria".into(), email: "m@x".into(), title: "Nutrition Coach".into(), perms: vec![Perm::Nutrition], clients: vec!["c1".into()], joined_at: Utc::now() };
        assert!(m.can(Perm::Nutrition) && !m.can(Perm::Messaging));
        assert!(m.assigned("c1") && !m.assigned("c2"));
        assert_eq!(m.display(), "Maria · Nutrition Coach");
        assert_eq!(serde_json::to_value(Perm::Programming).unwrap(), "programming");
    }
}
