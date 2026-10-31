//! Team and group conversations, beside each client's private thread with
//! their coach (model::Comment). A head coach's team (team.rs) message each
//! other one to one, and a coach starts named groups of teammates and the
//! clients they can reach; everyone in a group sees and replies to everyone.
//! A conversation lives in its head coach's business: the summary at
//! `COACH#{head}` / `CONV#{id}` (members, last message, each member's unread
//! count) and the messages at `CONV#{id}` / `MSG#{at}#{id}`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Attachment, AttachmentKind, Role};

pub const MAX_NAME: usize = 60;
/// Most people in a group, the starter included (one update counts a new
/// message as unread for all of them, within DynamoDB's 4 KB expression
/// limit). Larger audiences use a broadcast.
pub const MAX_MEMBERS: usize = 60;
/// Most clients one broadcast goes to.
pub const MAX_BROADCAST: usize = 500;
pub const MAX_TEXT: usize = 4000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConvKind {
    /// Two teammates; there's one per pair (`direct_id`).
    Direct,
    /// Named, any mix of teammates and clients.
    Group,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConvMember {
    pub user_id: String,
    /// As others see them: an assistant's carries their title.
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub kind: ConvKind,
    /// A group's name; empty for a direct conversation.
    #[serde(default)]
    pub name: String,
    pub members: Vec<ConvMember>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_text: String,
    #[serde(default)]
    pub last_author: String,
    /// Messages each member hasn't seen, by user id.
    #[serde(default)]
    pub unread: BTreeMap<String, u32>,
}

impl Conversation {
    pub fn member(&self, user_id: &str) -> Option<&ConvMember> {
        self.members.iter().find(|m| m.user_id == user_id)
    }

    /// What `user_id` sees it called: a group's name, or the other person.
    pub fn title_for(&self, user_id: &str) -> String {
        match self.kind {
            ConvKind::Group => self.name.clone(),
            ConvKind::Direct => self.members.iter().find(|m| m.user_id != user_id).map(|m| m.name.clone()).unwrap_or_else(|| "Former team member".into()),
        }
    }

    pub fn unread_for(&self, user_id: &str) -> u32 {
        self.unread.get(user_id).copied().unwrap_or(0)
    }

    /// Sets the members, keeping the unread counts of those who stay.
    pub fn set_members(&mut self, members: Vec<ConvMember>) {
        self.unread = members.iter().map(|m| (m.user_id.clone(), self.unread_for(&m.user_id))).collect();
        self.members = members;
    }
}

/// The one direct conversation between two teammates, whoever starts it.
pub fn direct_id(a: &str, b: &str) -> String {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    format!("dm-{x}-{y}")
}

/// A group's name as typed, or one made from the others' first names.
pub fn group_name(typed: Option<&str>, members: &[ConvMember], starter: &str) -> Result<String, String> {
    if let Some(t) = typed.map(str::trim).filter(|t| !t.is_empty()) {
        if t.chars().count() > MAX_NAME {
            return Err(format!("a group name is at most {MAX_NAME} characters"));
        }
        return Ok(t.to_string());
    }
    let first = |m: &ConvMember| m.name.split([' ', '·']).find(|w| !w.is_empty()).unwrap_or("").to_string();
    let others: Vec<String> = members.iter().filter(|m| m.user_id != starter).map(first).collect();
    Ok(match others.len() {
        0 => "Group".into(),
        1 => others[0].clone(),
        2 | 3 => format!("{} and {}", others[..others.len() - 1].join(", "), others[others.len() - 1]),
        n => format!("{}, {} and {} others", others[0], others[1], n - 2),
    })
}

/// `CONV#{id}` / `MSG#{at}#{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvMessage {
    pub id: String,
    pub author_id: String,
    pub author_name: String,
    pub author_role: Role,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachment: Option<Attachment>,
    pub created_at: DateTime<Utc>,
}

impl ConvMessage {
    pub fn preview_text(&self) -> String {
        match (&self.attachment, self.text.is_empty()) {
            (_, false) => self.text.clone(),
            (Some(a), true) if a.kind == AttachmentKind::Photo => "📷 Photo".into(),
            (Some(_), true) => "🎤 Voice note".into(),
            (None, true) => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, name: &str, role: Role) -> ConvMember {
        ConvMember { user_id: id.into(), name: name.into(), role }
    }

    #[test]
    fn direct_ids_and_titles() {
        assert_eq!(direct_id("b", "a"), direct_id("a", "b"));
        let mut c = Conversation {
            id: direct_id("a", "b"), kind: ConvKind::Direct, name: String::new(),
            members: vec![m("a", "Alex", Role::Coach), m("b", "Maria · Nutrition Coach", Role::Assistant)],
            created_by: "a".into(), created_at: Utc::now(), last_at: None, last_text: String::new(), last_author: String::new(), unread: BTreeMap::new(),
        };
        assert_eq!(c.title_for("a"), "Maria · Nutrition Coach");
        assert_eq!(c.title_for("b"), "Alex");
        c.unread.insert("a".into(), 3);
        c.set_members(vec![m("a", "Alex", Role::Coach), m("c", "Sam", Role::Client)]);
        assert_eq!((c.unread_for("a"), c.unread_for("c"), c.unread.contains_key("b")), (3, 0, false), "counts kept for those who stay");
    }

    #[test]
    fn group_names() {
        let ms = vec![m("me", "Alex Coach", Role::Coach), m("1", "Sam Client", Role::Client), m("2", "Maria · Nutrition Coach", Role::Assistant), m("3", "Jo", Role::Client), m("4", "Lee", Role::Client)];
        assert_eq!(group_name(Some("  Morning crew "), &ms, "me").unwrap(), "Morning crew");
        assert_eq!(group_name(None, &ms[..2], "me").unwrap(), "Sam");
        assert_eq!(group_name(Some(""), &ms[..4], "me").unwrap(), "Sam, Maria and Jo");
        assert_eq!(group_name(None, &ms, "me").unwrap(), "Sam, Maria and 2 others");
        assert!(group_name(Some(&"x".repeat(61)), &ms, "me").is_err());
    }
}
