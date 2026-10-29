//! Weekly check-ins (FB-3). Each coach has one list of questions (a default
//! until they edit it); a client answers once a week, and the coach compares
//! the last few weeks side by side. Each answered check-in keeps a copy of
//! the questions it answered, so editing the list later doesn't garble old weeks.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_QUESTIONS: usize = 20;
const MAX_LABEL: usize = 120;
const MAX_TEXT_ANSWER: usize = 2000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    /// 1-10.
    Scale,
    Number,
    YesNo,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    pub kind: QuestionKind,
    pub label: String,
    /// For numbers, e.g. "lb" or "hours".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// `COACH#{coach}` / `CHECKIN_TEMPLATE`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub questions: Vec<Question>,
}

/// `CLIENT#{id}` / `CHECKIN#{weekStart}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkin {
    /// The Monday of the week it covers.
    pub week_start: NaiveDate,
    pub questions: Vec<Question>,
    /// Answers by question id: a number for scales and numbers, a bool for
    /// yes/no, a string for text.
    pub answers: BTreeMap<String, Value>,
    pub submitted_at: DateTime<Utc>,
}

fn q(id: &str, kind: QuestionKind, label: &str, unit: Option<&str>, required: bool) -> Question {
    Question { id: id.into(), kind, label: label.into(), unit: unit.map(Into::into), required }
}

/// The questions a coach starts with.
pub fn default_template() -> Template {
    use QuestionKind::*;
    Template {
        questions: vec![
            q("overall", Scale, "How was your week overall?", None, true),
            q("energy", Scale, "Energy", None, true),
            q("sleep", Scale, "Sleep quality", None, true),
            q("stress", Scale, "Stress (10 = very stressed)", None, true),
            q("nutrition", Scale, "How well did you stick to your nutrition?", None, false),
            q("bodyweight", Number, "Bodyweight this morning", Some("lb"), false),
            q("wins", Text, "Wins this week", None, false),
            q("help", Text, "Anything you need from me?", None, false),
        ],
    }
}

/// The Monday starting the week that `date` is in.
pub fn week_of(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
}

impl Template {
    pub fn validate(&self) -> Result<(), String> {
        if self.questions.is_empty() || self.questions.len() > MAX_QUESTIONS {
            return Err(format!("a check-in needs 1-{MAX_QUESTIONS} questions"));
        }
        let mut ids = std::collections::HashSet::new();
        for q in &self.questions {
            if q.id.is_empty() || q.id.len() > 40 || !ids.insert(q.id.as_str()) {
                return Err("each question needs its own id".into());
            }
            if q.label.trim().is_empty() || q.label.chars().count() > MAX_LABEL {
                return Err(format!("questions must be 1-{MAX_LABEL} characters"));
            }
            if q.unit.as_ref().is_some_and(|u| u.chars().count() > 20) {
                return Err("units must be at most 20 characters".into());
            }
        }
        Ok(())
    }

    /// Checks `answers` against the questions and drops empty or unknown ones.
    pub fn clean_answers(&self, answers: BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, String> {
        let mut out = BTreeMap::new();
        for q in &self.questions {
            let v = answers.get(&q.id).filter(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()));
            let Some(v) = v else {
                if q.required {
                    return Err(format!("“{}” needs an answer", q.label));
                }
                continue;
            };
            let ok = match q.kind {
                QuestionKind::Scale => v.as_f64().is_some_and(|n| (1.0..=10.0).contains(&n) && n.fract() == 0.0),
                QuestionKind::Number => v.as_f64().is_some_and(|n| n.is_finite() && n.abs() < 1e6),
                QuestionKind::YesNo => v.is_boolean(),
                QuestionKind::Text => v.as_str().is_some_and(|s| s.chars().count() <= MAX_TEXT_ANSWER),
            };
            if !ok {
                return Err(format!("“{}”: that answer doesn't fit the question", q.label));
            }
            let v = match v {
                Value::String(s) => Value::String(s.trim().to_string()),
                other => other.clone(),
            };
            out.insert(q.id.clone(), v);
        }
        Ok(out)
    }
}

/// Whether `week_start` is a Monday the client may answer for: this week or last.
pub fn answerable(week_start: NaiveDate, today: NaiveDate) -> bool {
    week_start.weekday() == Weekday::Mon && week_start <= week_of(today) && week_start >= week_of(today) - chrono::Duration::days(7)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn default_is_valid_and_weeks_start_monday() {
        assert!(default_template().validate().is_ok());
        assert_eq!(week_of(d(10, 28)), d(10, 26)); // a Wednesday
        assert_eq!(week_of(d(10, 26)), d(10, 26));
        assert_eq!(week_of(d(11, 1)), d(10, 26)); // Sunday
        assert!(answerable(d(10, 26), d(10, 28)));
        assert!(answerable(d(10, 19), d(10, 28)), "last week, if they're late");
        assert!(!answerable(d(10, 12), d(10, 28)));
        assert!(!answerable(d(11, 2), d(10, 28)), "not next week");
        assert!(!answerable(d(10, 27), d(10, 28)), "a Tuesday");
    }

    #[test]
    fn answers_are_checked_against_the_questions() {
        let t = default_template();
        let good = BTreeMap::from([
            ("overall".to_string(), json!(8)),
            ("energy".to_string(), json!(7)),
            ("sleep".to_string(), json!(6)),
            ("stress".to_string(), json!(4)),
            ("bodyweight".to_string(), json!(182.4)),
            ("wins".to_string(), json!("  Hit a squat PR  ")),
            ("help".to_string(), json!("  ")),
            ("unknown".to_string(), json!("dropped")),
        ]);
        let clean = t.clean_answers(good.clone()).unwrap();
        assert_eq!(clean["wins"], "Hit a squat PR");
        assert!(!clean.contains_key("help") && !clean.contains_key("unknown"));
        let mut missing = good.clone();
        missing.remove("energy");
        assert!(t.clean_answers(missing).unwrap_err().contains("Energy"));
        let mut bad = good.clone();
        bad.insert("overall".into(), json!(11));
        assert!(t.clean_answers(bad).is_err());
        let mut half = good;
        half.insert("sleep".into(), json!(6.5));
        assert!(t.clean_answers(half).is_err(), "scales are whole numbers");
    }

    #[test]
    fn templates_need_distinct_ids() {
        let mut t = default_template();
        t.questions[1].id = "overall".into();
        assert!(t.validate().is_err());
        assert!(Template { questions: vec![] }.validate().is_err());
    }
}
