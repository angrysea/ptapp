//! Skinfold body-composition equations (BM-3, BM-4).
//!
//! Body density (g/cc) from:
//! * Jackson & Pollock 3-site (1978 men / 1980 women)
//! * Jackson & Pollock 7-site
//! * Durnin & Womersley 4-site (1974), age- and sex-specific coefficients
//!
//! converted to body-fat % with Siri (1961) or Brozek (1963).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::Sex;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkinfoldMethod {
    JacksonPollock3,
    JacksonPollock7,
    DurninWomersley4,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DensityEquation {
    Siri,
    Brozek,
}

/// Site names used as keys in `sites_mm` maps.
pub mod site {
    pub const CHEST: &str = "chest";
    pub const ABDOMEN: &str = "abdomen";
    pub const THIGH: &str = "thigh";
    pub const TRICEPS: &str = "triceps";
    pub const SUPRAILIAC: &str = "suprailiac";
    pub const MIDAXILLARY: &str = "midaxillary";
    pub const SUBSCAPULAR: &str = "subscapular";
    pub const BICEPS: &str = "biceps";
}

/// The sites a method needs for a given sex.
pub fn required_sites(method: SkinfoldMethod, sex: Sex) -> &'static [&'static str] {
    use site::*;
    match (method, sex) {
        (SkinfoldMethod::JacksonPollock3, Sex::Male) => &[CHEST, ABDOMEN, THIGH],
        (SkinfoldMethod::JacksonPollock3, Sex::Female) => &[TRICEPS, SUPRAILIAC, THIGH],
        (SkinfoldMethod::JacksonPollock7, _) => {
            &[CHEST, MIDAXILLARY, TRICEPS, SUBSCAPULAR, ABDOMEN, SUPRAILIAC, THIGH]
        }
        (SkinfoldMethod::DurninWomersley4, _) => &[BICEPS, TRICEPS, SUBSCAPULAR, SUPRAILIAC],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BodyCompResult {
    pub sum_mm: f64,
    pub body_density: f64,
    pub body_fat_pct: f64,
    pub fat_mass_kg: Option<f64>,
    pub lean_mass_kg: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BodyCompError {
    MissingSite(&'static str),
    InvalidReading(String),
    AgeOutOfRange(f64),
}

impl std::fmt::Display for BodyCompError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSite(s) => write!(f, "missing skinfold site: {s}"),
            Self::InvalidReading(s) => write!(f, "invalid reading at {s}: must be 1-80 mm"),
            Self::AgeOutOfRange(a) => write!(f, "age {a:.0} is outside the range this equation supports"),
        }
    }
}

impl std::error::Error for BodyCompError {}

pub struct SkinfoldInput<'a> {
    pub method: SkinfoldMethod,
    pub equation: DensityEquation,
    pub sex: Sex,
    pub age_years: f64,
    pub body_weight_kg: Option<f64>,
    /// Site -> one or more readings in mm; readings are averaged (BM-5).
    pub sites_mm: &'a BTreeMap<String, Vec<f64>>,
}

pub fn compute(input: &SkinfoldInput) -> Result<BodyCompResult, BodyCompError> {
    if !(10.0..=100.0).contains(&input.age_years) {
        return Err(BodyCompError::AgeOutOfRange(input.age_years));
    }
    let mut sum = 0.0;
    for &s in required_sites(input.method, input.sex) {
        let readings = input.sites_mm.get(s).filter(|r| !r.is_empty()).ok_or(BodyCompError::MissingSite(s))?;
        if readings.iter().any(|r| !(1.0..=80.0).contains(r)) {
            return Err(BodyCompError::InvalidReading(s.to_string()));
        }
        sum += readings.iter().sum::<f64>() / readings.len() as f64;
    }

    let age = input.age_years;
    let density = match (input.method, input.sex) {
        (SkinfoldMethod::JacksonPollock3, Sex::Male) => {
            1.10938 - 0.0008267 * sum + 0.0000016 * sum * sum - 0.0002574 * age
        }
        (SkinfoldMethod::JacksonPollock3, Sex::Female) => {
            1.0994921 - 0.0009929 * sum + 0.0000023 * sum * sum - 0.0001392 * age
        }
        (SkinfoldMethod::JacksonPollock7, Sex::Male) => {
            1.112 - 0.00043499 * sum + 0.00000055 * sum * sum - 0.00028826 * age
        }
        (SkinfoldMethod::JacksonPollock7, Sex::Female) => {
            1.097 - 0.00046971 * sum + 0.00000056 * sum * sum - 0.00012828 * age
        }
        (SkinfoldMethod::DurninWomersley4, sex) => {
            let (c, m) = durnin_womersley_coefficients(sex, age);
            c - m * sum.log10()
        }
    };

    let body_fat_pct = match input.equation {
        DensityEquation::Siri => 495.0 / density - 450.0,
        DensityEquation::Brozek => 457.0 / density - 414.2,
    };
    let fat_mass_kg = input.body_weight_kg.map(|w| w * body_fat_pct / 100.0);
    let lean_mass_kg = input.body_weight_kg.zip(fat_mass_kg).map(|(w, f)| w - f);

    Ok(BodyCompResult {
        sum_mm: round(sum, 1),
        body_density: round(density, 5),
        body_fat_pct: round(body_fat_pct, 1),
        fat_mass_kg: fat_mass_kg.map(|v| round(v, 1)),
        lean_mass_kg: lean_mass_kg.map(|v| round(v, 1)),
    })
}

/// Durnin & Womersley (1974) Table 4 coefficients: density = c - m * log10(sum of 4).
/// The paper's youngest band is 17-19; clients below 17 use it (compute() has
/// already restricted ages to 10-100).
fn durnin_womersley_coefficients(sex: Sex, age: f64) -> (f64, f64) {
    match sex {
        Sex::Male => match age {
            a if a < 20.0 => (1.1620, 0.0630),
            a if a < 30.0 => (1.1631, 0.0632),
            a if a < 40.0 => (1.1422, 0.0544),
            a if a < 50.0 => (1.1620, 0.0700),
            _ => (1.1715, 0.0779),
        },
        Sex::Female => match age {
            a if a < 20.0 => (1.1549, 0.0678),
            a if a < 30.0 => (1.1599, 0.0717),
            a if a < 40.0 => (1.1423, 0.0632),
            a if a < 50.0 => (1.1333, 0.0612),
            _ => (1.1339, 0.0645),
        },
    }
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

// ---------------------------------------------------------------- other estimates (BM-6)

/// Where a body-fat reading came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BodyFatSource {
    /// Worked out from tape measurements (US Navy formula).
    Navy,
    Dexa,
    /// Bioimpedance (BIA) device.
    Bia,
    /// A smart scale.
    Scale,
    Other,
}

/// The tape measurements behind a Navy estimate, in cm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NavyTape {
    pub height_cm: f64,
    pub neck_cm: f64,
    pub waist_cm: f64,
    /// Needed for women.
    #[serde(default)]
    pub hip_cm: Option<f64>,
}

/// A body-fat percentage that isn't from calipers (those are SkinfoldRecord).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BodyFatReading {
    pub source: BodyFatSource,
    pub pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navy: Option<NavyTape>,
}

/// US Navy circumference estimate (Hodgdon & Beckett), metric form, to 0.1%.
/// Men: waist at the navel; women: waist at its narrowest, plus hips.
pub fn navy_body_fat(sex: Sex, t: &NavyTape) -> Result<f64, String> {
    let ok = |v: f64| (10.0..=300.0).contains(&v);
    if !(100.0..=250.0).contains(&t.height_cm) || !ok(t.neck_cm) || !ok(t.waist_cm) {
        return Err("height must be 100-250 cm and the tape measurements 10-300 cm".into());
    }
    let pct = match sex {
        Sex::Male => {
            let d = t.waist_cm - t.neck_cm;
            if d <= 0.0 {
                return Err("the waist must measure more than the neck".into());
            }
            495.0 / (1.0324 - 0.19077 * d.log10() + 0.15456 * t.height_cm.log10()) - 450.0
        }
        Sex::Female => {
            let hip = t.hip_cm.filter(|h| ok(*h)).ok_or("women's estimates need a hip measurement")?;
            let d = t.waist_cm + hip - t.neck_cm;
            495.0 / (1.29579 - 0.35004 * d.log10() + 0.22100 * t.height_cm.log10()) - 450.0
        }
    };
    if !(2.0..=70.0).contains(&pct) {
        return Err("those measurements don't give a believable body-fat percentage; check them".into());
    }
    Ok((pct * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sites(pairs: &[(&str, &[f64])]) -> BTreeMap<String, Vec<f64>> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_vec())).collect()
    }

    fn run(method: SkinfoldMethod, eq: DensityEquation, sex: Sex, age: f64, s: &BTreeMap<String, Vec<f64>>) -> BodyCompResult {
        compute(&SkinfoldInput { method, equation: eq, sex, age_years: age, body_weight_kg: Some(80.0), sites_mm: s }).unwrap()
    }

    #[test]
    fn jp3_male_siri() {
        // sum = 60, age 30 -> BD = 1.10938 - 0.049602 + 0.00576 - 0.007722 = 1.057816
        let s = sites(&[("chest", &[15.0]), ("abdomen", &[25.0]), ("thigh", &[20.0])]);
        let r = run(SkinfoldMethod::JacksonPollock3, DensityEquation::Siri, Sex::Male, 30.0, &s);
        assert_eq!(r.sum_mm, 60.0);
        assert!((r.body_density - 1.05782).abs() < 1e-5);
        assert_eq!(r.body_fat_pct, 17.9); // 495/1.057816 - 450 = 17.94
        assert_eq!(r.fat_mass_kg, Some(14.4));
        assert_eq!(r.lean_mass_kg, Some(65.6));
    }

    #[test]
    fn jp3_female_brozek() {
        // sum = 70, age 25 -> BD = 1.0994921 - 0.069503 + 0.01127 - 0.00348 = 1.0377791
        let s = sites(&[("triceps", &[20.0]), ("suprailiac", &[20.0]), ("thigh", &[30.0])]);
        let r = run(SkinfoldMethod::JacksonPollock3, DensityEquation::Brozek, Sex::Female, 25.0, &s);
        assert!((r.body_density - 1.03778).abs() < 1e-5);
        assert_eq!(r.body_fat_pct, 26.2); // 457/1.0377791 - 414.2 = 26.16
    }

    #[test]
    fn jp7_male() {
        // sum = 105, age 40 -> BD = 1.112 - 0.04567395 + 0.00606375 - 0.0115304 = 1.0608594
        let s = sites(&[
            ("chest", &[10.0]), ("midaxillary", &[15.0]), ("triceps", &[12.0]), ("subscapular", &[18.0]),
            ("abdomen", &[25.0]), ("suprailiac", &[15.0]), ("thigh", &[10.0]),
        ]);
        let r = run(SkinfoldMethod::JacksonPollock7, DensityEquation::Siri, Sex::Male, 40.0, &s);
        assert!((r.body_density - 1.06086).abs() < 1e-5);
        assert_eq!(r.body_fat_pct, 16.6); // 495/1.0608594 - 450 = 16.60
    }

    #[test]
    fn jp7_female() {
        // Constants from the Jackson & Pollock women's 7-site equation
        // (Med Sci Sports Exerc, 1980;12(3):175-81, PMID 7402053). The paper
        // itself is paywalled; re-verify these digits against it before
        // trusting body-fat numbers for women to one decimal.
        // sum = 100, age 25 -> BD = 1.097 - 0.046971 + 0.0056 - 0.003207 = 1.052422
        let s = sites(&[
            ("chest", &[10.0]), ("midaxillary", &[15.0]), ("triceps", &[10.0]), ("subscapular", &[15.0]),
            ("abdomen", &[20.0]), ("suprailiac", &[15.0]), ("thigh", &[15.0]),
        ]);
        let r = run(SkinfoldMethod::JacksonPollock7, DensityEquation::Siri, Sex::Female, 25.0, &s);
        assert_eq!(r.sum_mm, 100.0);
        assert!((r.body_density - 1.05242).abs() < 1e-5);
        assert_eq!(r.body_fat_pct, 20.3); // 495/1.052422 - 450 = 20.34
    }

    #[test]
    fn dw_under_17_uses_first_band() {
        // age 14 female -> 17-19 band: BD = 1.1549 - 0.0678 * log10(50) = 1.03971
        let s = sites(&[("biceps", &[8.0]), ("triceps", &[16.0]), ("subscapular", &[12.0]), ("suprailiac", &[14.0])]);
        let r = run(SkinfoldMethod::DurninWomersley4, DensityEquation::Siri, Sex::Female, 14.0, &s);
        assert!((r.body_density - 1.03971).abs() < 1e-5);
    }

    #[test]
    fn durnin_womersley_female_30s() {
        // sum = 50, age 35 -> BD = 1.1423 - 0.0632 * log10(50) = 1.1423 - 0.107373 = 1.034927
        let s = sites(&[("biceps", &[8.0]), ("triceps", &[16.0]), ("subscapular", &[12.0]), ("suprailiac", &[14.0])]);
        let r = run(SkinfoldMethod::DurninWomersley4, DensityEquation::Siri, Sex::Female, 35.0, &s);
        assert!((r.body_density - 1.03493).abs() < 1e-5);
        assert_eq!(r.body_fat_pct, 28.3); // 495/1.034927 - 450 = 28.29
    }

    #[test]
    fn navy_estimates() {
        // 180 cm man, neck 38, waist 85: 16.1% (checked against the published formula)
        let man = NavyTape { height_cm: 180.0, neck_cm: 38.0, waist_cm: 85.0, hip_cm: None };
        assert_eq!(navy_body_fat(Sex::Male, &man), Ok(16.1));
        // 165 cm woman, neck 32, waist 70, hips 95: 24.9%
        let woman = NavyTape { height_cm: 165.0, neck_cm: 32.0, waist_cm: 70.0, hip_cm: Some(95.0) };
        assert_eq!(navy_body_fat(Sex::Female, &woman), Ok(24.9));
        assert!(navy_body_fat(Sex::Female, &NavyTape { hip_cm: None, ..woman }).is_err());
        assert!(navy_body_fat(Sex::Male, &NavyTape { waist_cm: 30.0, ..man }).is_err(), "waist under neck");
    }

    #[test]
    fn averages_multiple_readings() {
        let s = sites(&[("chest", &[14.0, 16.0]), ("abdomen", &[24.0, 26.0]), ("thigh", &[20.0, 20.0, 20.0])]);
        let r = run(SkinfoldMethod::JacksonPollock3, DensityEquation::Siri, Sex::Male, 30.0, &s);
        assert_eq!(r.sum_mm, 60.0);
    }

    #[test]
    fn missing_site_and_bad_reading_are_errors() {
        let s = sites(&[("chest", &[15.0]), ("abdomen", &[25.0])]);
        let err = compute(&SkinfoldInput {
            method: SkinfoldMethod::JacksonPollock3, equation: DensityEquation::Siri, sex: Sex::Male,
            age_years: 30.0, body_weight_kg: None, sites_mm: &s,
        });
        assert_eq!(err, Err(BodyCompError::MissingSite("thigh")));

        let s = sites(&[("chest", &[15.0]), ("abdomen", &[250.0]), ("thigh", &[20.0])]);
        let err = compute(&SkinfoldInput {
            method: SkinfoldMethod::JacksonPollock3, equation: DensityEquation::Siri, sex: Sex::Male,
            age_years: 30.0, body_weight_kg: None, sites_mm: &s,
        });
        assert!(matches!(err, Err(BodyCompError::InvalidReading(_))));
    }

    #[test]
    fn no_body_weight_means_no_masses() {
        let s = sites(&[("chest", &[15.0]), ("abdomen", &[25.0]), ("thigh", &[20.0])]);
        let r = compute(&SkinfoldInput {
            method: SkinfoldMethod::JacksonPollock3, equation: DensityEquation::Siri, sex: Sex::Male,
            age_years: 30.0, body_weight_kg: None, sites_mm: &s,
        }).unwrap();
        assert_eq!(r.fat_mass_kg, None);
        assert_eq!(r.lean_mass_kg, None);
    }
}
