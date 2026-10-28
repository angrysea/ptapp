//! Coach profiles and the public coach directory (tasks/coaches/plan.md).
//! A coach's full profile holds private business details (for the coach and
//! admins); `CoachProfile::public` copies out only what anyone may see, and
//! that copy is all the directory ever stores.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Specialties offered on the form (the form also allows "other" text).
pub const SPECIALTIES: &[&str] = &[
    "Strength training",
    "Weight loss",
    "Muscle gain",
    "General fitness",
    "Mobility and flexibility",
    "Sports performance",
    "Injury recovery",
    "Pre- and postnatal",
    "Older adults",
    "Youth athletes",
    "Powerlifting",
    "Olympic weightlifting",
    "Endurance",
    "Nutrition coaching",
];

/// Shortest bio a listed profile may have, in characters.
pub const MIN_BIO: usize = 80;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TrainingFormat {
    InPerson,
    Online,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub line1: String,
    #[serde(default)]
    pub line2: String,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Certification {
    /// e.g. "NASM Certified Personal Trainer"
    pub name: String,
    /// e.g. "NASM"
    pub issuer: String,
    /// Private: for checking with the issuer.
    #[serde(default)]
    pub number: String,
    pub expires: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Insurance {
    pub provider: String,
    pub policy_number: String,
    pub expires: Option<NaiveDate>,
}

/// Stored at `USER#{coach}` / `COACHPROFILE`; read only by the coach and admins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoachProfile {
    // ---- private
    pub legal_name: String,
    pub phone: String,
    pub address: Address,
    pub cpr_expires: Option<NaiveDate>,
    pub insurance: Option<Insurance>,

    // ---- public
    pub display_name: String,
    pub headline: String,
    pub bio: String,
    /// Media key of the profile photo (`media/{coach}/profile/...`).
    pub photo_key: Option<String>,
    pub specialties: Vec<String>,
    pub certifications: Vec<Certification>,
    pub years_experience: u32,
    pub formats: Vec<TrainingFormat>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub rates: String,
    pub contact_email: String,
    #[serde(default)]
    pub website: String,
    #[serde(default)]
    pub instagram: String,
    pub accepting_clients: bool,
    /// The coach's own choice to appear in the directory.
    pub listed: bool,
    /// Set by an admin (e.g. until certifications are checked); wins over `listed`.
    #[serde(default)]
    pub hidden_by_admin: bool,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

/// A certification as the public sees it: no number.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicCertification {
    pub name: String,
    pub issuer: String,
}

/// The directory entry, `DIRECTORY` / `COACH#{coach}`: public fields only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicCoach {
    pub coach_id: String,
    pub display_name: String,
    pub headline: String,
    pub bio: String,
    pub photo_key: Option<String>,
    pub specialties: Vec<String>,
    pub certifications: Vec<PublicCertification>,
    pub years_experience: u32,
    pub formats: Vec<TrainingFormat>,
    pub languages: Vec<String>,
    pub rates: String,
    pub contact_email: String,
    pub website: String,
    pub instagram: String,
    pub city: String,
    pub state: String,
    pub accepting_clients: bool,
    pub updated_at: DateTime<Utc>,
}

fn blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// A loose check: something@something.something, no spaces.
pub fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    match s.split_once('@') {
        Some((user, domain)) => !user.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !s.contains(char::is_whitespace),
        None => false,
    }
}

impl CoachProfile {
    /// Trims every text field, drops empty list entries and empty
    /// certification rows, and lowercases the contact email.
    pub fn normalize(&mut self) {
        let t = |s: &mut String| *s = s.trim().to_string();
        for s in [
            &mut self.legal_name, &mut self.phone, &mut self.display_name, &mut self.headline, &mut self.bio, &mut self.rates,
            &mut self.contact_email, &mut self.website, &mut self.instagram,
            &mut self.address.line1, &mut self.address.line2, &mut self.address.city, &mut self.address.state,
            &mut self.address.postal_code, &mut self.address.country,
        ] {
            t(s);
        }
        self.contact_email = self.contact_email.to_lowercase();
        for list in [&mut self.specialties, &mut self.languages] {
            list.iter_mut().for_each(t);
            list.retain(|s| !s.is_empty());
            list.dedup();
        }
        for c in &mut self.certifications {
            t(&mut c.name);
            t(&mut c.issuer);
            t(&mut c.number);
        }
        self.certifications.retain(|c| !(blank(&c.name) && blank(&c.issuer) && blank(&c.number)));
        if let Some(i) = &mut self.insurance {
            t(&mut i.provider);
            t(&mut i.policy_number);
        }
        if self.insurance.as_ref().is_some_and(|i| blank(&i.provider) && blank(&i.policy_number) && i.expires.is_none()) {
            self.insurance = None;
        }
        self.formats.dedup();
    }

    /// What's missing or wrong, in form order; empty when the profile is complete.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        let mut need = |ok: bool, msg: &str| {
            if !ok {
                p.push(msg.to_string());
            }
        };
        need(!blank(&self.legal_name), "Legal name is required");
        need(self.phone.chars().filter(char::is_ascii_digit).count() >= 10, "Phone needs at least 10 digits");
        let a = &self.address;
        need(!blank(&a.line1) && !blank(&a.city) && !blank(&a.state) && !blank(&a.postal_code) && !blank(&a.country), "Full address is required (street, city, state, ZIP and country)");
        need(self.cpr_expires.is_some(), "CPR/AED certification expiry date is required");
        if let Some(i) = &self.insurance {
            need(!blank(&i.provider) && !blank(&i.policy_number), "Insurance needs a provider and a policy number");
        }
        need(!blank(&self.display_name), "Display name is required");
        need(!blank(&self.headline), "Headline is required");
        need(self.headline.chars().count() <= 120, "Headline must be 120 characters or fewer");
        need(self.bio.chars().count() >= MIN_BIO, &format!("Bio needs at least {MIN_BIO} characters"));
        need(self.bio.chars().count() <= 4000, "Bio must be 4000 characters or fewer");
        need(!self.specialties.is_empty(), "Choose at least one specialty");
        need(!self.certifications.is_empty(), "Add at least one certification");
        need(self.certifications.iter().all(|c| !blank(&c.name) && !blank(&c.issuer)), "Each certification needs a name and an issuer");
        need(self.years_experience <= 70, "Years of experience looks wrong");
        need(!self.formats.is_empty(), "Choose in person, online or both");
        need(looks_like_email(&self.contact_email), "Contact email doesn't look right");
        need(blank(&self.website) || self.website.starts_with("https://") || self.website.starts_with("http://"), "Website must start with https://");
        p
    }

    /// Whether this profile belongs in the public directory.
    pub fn shows_in_directory(&self) -> bool {
        self.listed && !self.hidden_by_admin && self.problems().is_empty()
    }

    /// The public copy of this profile, for the directory.
    pub fn public(&self, coach_id: &str) -> PublicCoach {
        PublicCoach {
            coach_id: coach_id.to_string(),
            display_name: self.display_name.clone(),
            headline: self.headline.clone(),
            bio: self.bio.clone(),
            photo_key: self.photo_key.clone(),
            specialties: self.specialties.clone(),
            certifications: self.certifications.iter().map(|c| PublicCertification { name: c.name.clone(), issuer: c.issuer.clone() }).collect(),
            years_experience: self.years_experience,
            formats: self.formats.clone(),
            languages: self.languages.clone(),
            rates: self.rates.clone(),
            contact_email: self.contact_email.clone(),
            website: self.website.clone(),
            instagram: self.instagram.trim_start_matches('@').to_string(),
            city: self.address.city.clone(),
            state: self.address.state.clone(),
            accepting_clients: self.accepting_clients,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn complete() -> CoachProfile {
        CoachProfile {
            legal_name: "Jordan Q. Lee".into(),
            phone: "(512) 555-0100".into(),
            address: Address {
                line1: "1 Main St".into(),
                line2: String::new(),
                city: "Austin".into(),
                state: "TX".into(),
                postal_code: "78701".into(),
                country: "US".into(),
            },
            cpr_expires: NaiveDate::from_ymd_opt(2027, 5, 1),
            insurance: None,
            display_name: "Coach Jordan".into(),
            headline: "Strength for busy parents".into(),
            bio: "I help busy parents get strong in three short sessions a week, with plans that fit around real life.".into(),
            photo_key: None,
            specialties: vec!["Strength training".into()],
            certifications: vec![Certification { name: "CSCS".into(), issuer: "NSCA".into(), number: "123456".into(), expires: None }],
            years_experience: 8,
            formats: vec![TrainingFormat::InPerson, TrainingFormat::Online],
            languages: vec!["English".into()],
            rates: "From $60/session".into(),
            contact_email: "jordan@example.com".into(),
            website: String::new(),
            instagram: "@coachjordan".into(),
            accepting_clients: true,
            listed: true,
            hidden_by_admin: false,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn a_complete_profile_has_no_problems_and_is_listed() {
        let p = complete();
        assert_eq!(p.problems(), Vec::<String>::new());
        assert!(p.shows_in_directory());
    }

    #[test]
    fn missing_required_fields_are_each_reported() {
        let mut p = complete();
        p.legal_name = " ".into();
        p.phone = "555-0100".into();
        p.address.city.clear();
        p.cpr_expires = None;
        p.bio = "Too short.".into();
        p.specialties.clear();
        p.certifications.clear();
        p.formats.clear();
        p.contact_email = "nope".into();
        let problems = p.problems();
        assert_eq!(problems.len(), 9, "{problems:?}");
        assert!(!p.shows_in_directory());
    }

    #[test]
    fn hidden_or_unlisted_profiles_stay_out_of_the_directory() {
        let mut p = complete();
        p.listed = false;
        assert!(!p.shows_in_directory());
        p.listed = true;
        p.hidden_by_admin = true;
        assert!(!p.shows_in_directory());
    }

    #[test]
    fn the_public_copy_has_no_private_fields() {
        let p = complete();
        let json = serde_json::to_string(&p.public("c1")).unwrap();
        for private in ["Jordan Q. Lee", "555-0100", "1 Main St", "78701", "123456", "cprExpires", "legalName", "phone", "insurance"] {
            assert!(!json.contains(private), "{private} leaked: {json}");
        }
        assert!(json.contains("\"city\":\"Austin\"") && json.contains("\"instagram\":\"coachjordan\""), "{json}");
    }

    #[test]
    fn normalize_trims_and_drops_empty_rows() {
        let mut p = complete();
        p.contact_email = "  Jordan@Example.COM ".into();
        p.specialties = vec![" Strength training ".into(), "".into()];
        p.certifications.push(Certification { name: " ".into(), issuer: "".into(), number: "".into(), expires: None });
        p.insurance = Some(Insurance { provider: " ".into(), policy_number: "".into(), expires: None });
        p.normalize();
        assert_eq!(p.contact_email, "jordan@example.com");
        assert_eq!(p.specialties, vec!["Strength training".to_string()]);
        assert_eq!(p.certifications.len(), 1);
        assert_eq!(p.insurance, None);
    }

    #[test]
    fn email_check_is_loose_but_catches_typos() {
        assert!(looks_like_email("a@b.co"));
        for bad in ["", "a", "a@b", "@b.co", "a@.co", "a b@c.co", "a@b.co."] {
            assert!(!looks_like_email(bad), "{bad}");
        }
    }
}
