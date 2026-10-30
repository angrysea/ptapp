//! Food journal: the client logs a meal as item lines ("chicken 12oz");
//! quantities parse to grams, and the coach attaches a snapshot of a looked-up
//! food (USDA FoodData Central, through the API's /foods/search) so the
//! estimate is `kcal_per` kcal per `per_grams` grams at log time — it never
//! drifts with the food database, and re-logging a known item costs no lookup.

use std::collections::HashMap;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Item lines per meal.
pub const MAX_ITEMS: usize = 12;
/// One item line as typed.
pub const MAX_TEXT: usize = 140;
/// A meal label, "Lunch".
pub const MAX_LABEL: usize = 40;
/// A single item's kcal is never above this (a day can be, across meals).
pub const MAX_KCAL: f64 = 20_000.0;
/// 10 kg of anything in one line is a typo.
pub const MAX_GRAMS: f64 = 10_000.0;
/// Coach's saved staples per coach ("their usual lunch").
pub const MAX_SAVED: usize = 40;
/// Search candidates the coach picks from.
pub const MAX_CANDIDATES: usize = 10;

const G_PER_OZ: f64 = 28.349_523_125;
const G_PER_LB: f64 = 453.592_37;

/// How an item's calories came to be known.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Basis {
    /// The client typed it; nothing estimated yet.
    FreeText,
    /// The coach matched the line to a food and attached its snapshot.
    Searched,
    /// The coach set the number themselves.
    Manual,
    /// Read from another app's export (Cronometer, MyFitnessPal).
    Imported,
}

/// A FoodData Central food as stored at lookup time: the estimate is
/// `kcal_per` kcal (and the macros' grams) per `per_grams` grams.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FoodSnap {
    pub fdc_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub per_grams: f64,
    pub kcal_per: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein_per: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs_per: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat_per: Option<f64>,
}

/// What /foods/search offers the coach to pick from, flattened out of the
/// USDA response (see `normalize_fdc`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub fdc_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grams: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kcal: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat: Option<f64>,
}

impl Candidate {
    /// The coach's snapshot of a chosen candidate. Foods with no calorie
    /// data still snapshot (kcal 0 per 100 g) so the pick sticks; the coach
    /// can then set a number themselves (basis manual).
    pub fn snap(&self) -> FoodSnap {
        FoodSnap {
            fdc_id: self.fdc_id.clone(),
            description: Some(self.name.clone()),
            per_grams: self.grams.unwrap_or(100.0),
            kcal_per: self.kcal.unwrap_or(0.0),
            protein_per: self.protein,
            carbs_per: self.carbs,
            fat_per: self.fat,
        }
    }
}

/// One line of a meal. `grams` is the parsed amount; text keeps the parsed
/// description ("chicken 12oz" becomes text "chicken", grams 340.2).
/// `kcal`/`protein`/`carbs`/`fat` are numbers set by hand (or imported) and
/// each wins over the snapshot's figure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grams: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub food: Option<FoodSnap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kcal: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat: Option<f64>,
    pub basis: Basis,
}

/// kcal and grams of protein, carbohydrate and fat.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Macros {
    pub kcal: f64,
    pub protein: f64,
    pub carbs: f64,
    pub fat: f64,
}

impl Macros {
    fn add(&mut self, o: Macros) {
        self.kcal += o.kcal;
        self.protein += o.protein;
        self.carbs += o.carbs;
        self.fat += o.fat;
    }
}

impl Item {
    /// The item's calories: a coach's number wins, else the snapshot scaled
    /// to the parsed amount, else unknown (0, shown as "not logged yet").
    pub fn energy(&self) -> f64 {
        self.macros().kcal
    }

    /// Each figure on its own: a number set by hand wins, else the snapshot's
    /// figure scaled to the parsed amount, else 0.
    pub fn macros(&self) -> Macros {
        let scaled = |per: Option<f64>| -> f64 {
            match (&self.food, self.grams, per) {
                (Some(f), Some(g), Some(p)) if f.per_grams > 0.0 => p * g / f.per_grams,
                _ => 0.0,
            }
        };
        let snap = |pick: fn(&FoodSnap) -> Option<f64>| self.food.as_ref().and_then(pick);
        Macros {
            kcal: self.kcal.unwrap_or_else(|| scaled(snap(|f| Some(f.kcal_per)))),
            protein: self.protein.unwrap_or_else(|| scaled(snap(|f| f.protein_per))),
            carbs: self.carbs.unwrap_or_else(|| scaled(snap(|f| f.carbs_per))),
            fat: self.fat.unwrap_or_else(|| scaled(snap(|f| f.fat_per))),
        }
    }

    pub fn estimated(&self) -> bool {
        self.kcal.is_some() || self.food.is_some()
    }
}

/// `CLIENT#{id}` / `MEAL#{date}#{id}` — the whole meal as one item; a later
/// write wins (like a habit day's value), `updated_at` is for showing when.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Meal {
    pub id: String,
    pub date: NaiveDate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub items: Vec<Item>,
    pub updated_at: DateTime<Utc>,
}

impl Meal {
    pub fn validate(&mut self) -> Result<(), String> {
        self.label = self.label.take().map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
        if self.label.as_ref().is_some_and(|l| l.chars().count() > MAX_LABEL) {
            return Err("a meal label must be at most 40 characters".into());

        }
        if self.items.is_empty() || self.items.len() > MAX_ITEMS {
            return Err(format!("a meal has 1-{MAX_ITEMS} item lines"));
        }
        for it in self.items.iter_mut() {
            it.text = it.text.trim().to_string();
            if it.text.is_empty() || it.text.chars().count() > MAX_TEXT {
                return Err(format!("an item line must be 1-{MAX_TEXT} characters"));
            }
            if it.grams.as_ref().is_some_and(|g| !(*g > 0.0 && *g <= MAX_GRAMS)) {
                return Err("an item's amount must be above 0 g".into());
            }
            for bound in [it.kcal, it.protein, it.carbs, it.fat] {
                if bound.as_ref().is_some_and(|n| !(*n >= 0.0 && *n <= MAX_KCAL)) {
                    return Err("an item's estimate must be 0-20000".into());
                }
            }
            let sane = |n: &f64| *n >= 0.0 && *n <= MAX_KCAL;
            if let Some(f) = it.food.as_ref()
                && (!(f.per_grams > 0.0 && f.per_grams <= MAX_GRAMS) || !sane(&f.kcal_per) || ![f.protein_per, f.carbs_per, f.fat_per].iter().flatten().all(sane))
            {
                return Err("a food snapshot needs a positive amount and a sane kcal figure".into());
            }
            // A typed line carries its own quantity: "chicken 12oz" parses to
            // grams so the coach's estimate scales without re-reading the text.
            if it.grams.is_none() {
                let (grams, rest) = parse_item(it.text.clone());
                if grams.is_some() {
                    it.grams = grams;
                    if !rest.is_empty() {
                        it.text = rest;
                    }
                }
            }
        }
        Ok(())
    }
}

/// A coach's saved food ("their usual lunch"): `COACH#{coach}` / `FOOD#{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<FoodSnap>,
    pub created_at: DateTime<Utc>,
}

impl Saved {
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > 60 {
            return Err("a saved food's name must be 1-60 characters".into());
        }
        Ok(())
    }
}

/// Per-day kcal and macros summed over whole meals, in date order. Days
/// nothing was logged stay out: an empty day is not a zero day.
pub fn totals(meals: &[Meal]) -> Vec<(NaiveDate, Macros)> {
    let mut by_day: HashMap<NaiveDate, Macros> = HashMap::new();
    for m in meals {
        let day = by_day.entry(m.date).or_default();
        for i in &m.items {
            day.add(i.macros());
        }
    }
    let mut out: Vec<(NaiveDate, Macros)> = by_day.into_iter().collect();
    out.sort_by_key(|(d, _)| *d);
    out
}

/// `CLIENT#{id}` / `NUTRITION_TARGET`: the coach's daily targets. Any of the
/// four can be left out ("just hit your protein").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kcal: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

impl Target {
    pub fn validate(&mut self) -> Result<(), String> {
        self.note = self.note.take().map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        if self.note.as_ref().is_some_and(|n| n.chars().count() > 280) {
            return Err("a target note must be at most 280 characters".into());
        }
        if self.kcal.is_some_and(|k| !(500.0..=10_000.0).contains(&k)) {
            return Err("a calorie target must be 500-10000 kcal".into());
        }
        if [self.protein, self.carbs, self.fat].into_iter().flatten().any(|g| !(0.0..=1_000.0).contains(&g)) {
            return Err("a macro target must be 0-1000 g".into());
        }
        Ok(())
    }
}

/// The last `days` days ending `today`: how many had anything logged and the
/// average per logged day (unlogged days would drag an average to nothing).
/// `on_target` counts logged days within 10% of the kcal target.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub days: i64,
    pub logged: usize,
    pub average: Macros,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_target: Option<usize>,
}

pub fn summary(day_totals: &[(NaiveDate, Macros)], today: NaiveDate, days: i64, target: Option<&Target>) -> Summary {
    let from = today - Duration::days(days - 1);
    let window: Vec<Macros> = day_totals.iter().filter(|(d, _)| *d >= from && *d <= today).map(|(_, m)| *m).collect();
    let mut sum = Macros::default();
    for m in &window {
        sum.add(*m);
    }
    let n = window.len().max(1) as f64;
    let average = Macros { kcal: sum.kcal / n, protein: sum.protein / n, carbs: sum.carbs / n, fat: sum.fat / n };
    let on_target = target.and_then(|t| t.kcal).map(|k| window.iter().filter(|m| (m.kcal - k).abs() <= k * 0.1).count());
    Summary { days, logged: window.len(), average, on_target }
}

/// Lowercase a word, stopping at the first period or comma, so "oz." reads as
/// "oz" ("1.5" only ever reaches this after its number is split off).
fn tidy(word: String) -> String {
    let out: String = word.trim().to_lowercase().chars().take_while(|c| !(*c == '.' || *c == ',')).collect();
    out
}

/// Grams for one unit word, so a bare "oz" weighs this much ("12 oz" of it:
/// × 12). Cups and tablespoons stay unmeasurable — per-food cup weights belong
/// in a later pass.
fn unit_grams(word: String) -> Option<f64> {
    match tidy(word).as_str() {
        "g" | "gram" | "grams" | "gm" => Some(1.0),
        "oz" | "ounce" | "ounces" => Some(G_PER_OZ),
        "lb" | "lbs" | "pound" | "pounds" => Some(G_PER_LB),
        "kg" | "kilo" | "kilos" => Some(1000.0),
        _ => None,
    }
}

/// Split a word's leading number from what follows: "12oz" is (12, "oz"),
/// "1.5" is (1.5, ""), and a word starting with a letter is (0, itself).
fn lead_number(word: String) -> (f64, String) {
    let head: String = word.chars().take_while(|c| c.is_numeric() || *c == '.').collect();
    if head.is_empty() {
        return (0.0, word);
    }
    let head_len: usize = head.chars().count();
    let tail: String = word.chars().skip(head_len).collect();
    (head.parse().unwrap_or(0.0), tail.to_lowercase())
}

/// One typed line → (grams, description). "2 chicken breasts 6oz each" is the
/// bare 2 multiplying the one measured amount (340 g); several measured
/// amounts sum; an unmeasured count ("2 apples") stays unmeasured.
pub fn parse_item(text: String) -> (Option<f64>, String) {
    let words: Vec<String> = text.split_whitespace().map(|w| w.to_string()).collect();
    let mut measured: Vec<f64> = vec![];
    let mut bare: Vec<f64> = vec![];
    let mut rest: Vec<String> = vec![];
    let mut i = 0;
    while i < words.len() {
        let w = words[i].clone();
        let flat = tidy(w.clone());
        if flat == "each" || flat == "of" {
            i += 1;
            continue;
        }
        let (n, unit) = lead_number(w.clone());
        if n == 0.0 {
            rest.push(w);
            i += 1;
            continue;
        }
        if unit.is_empty() {
            // "12 oz": the unit may be the next word.
            let next_unit = if i + 1 < words.len() { unit_grams(words[i + 1].clone()) } else { None };
            if let Some(f) = next_unit {
                measured.push(n * f);
                i += 2;
                continue;
            }
            bare.push(n);
            i += 1;
            continue;
        }
        let unit_value: Option<f64> = unit_grams(unit.clone());
        if let Some(f) = unit_value {
            measured.push(n * f);
            i += 1;
            continue;
        }
        // A number with an unknown word ("2 apples") is a count, not grams.
        bare.push(n);
        let rest_word: String = tidy(unit);
        if !rest_word.is_empty() {
            rest.push(rest_word);
        }
        i += 1;
    }
    let grams = match (measured.len(), bare.len()) {
        (0, _) => None,
        (1, 1) => Some(measured[0] * bare[0]),
        _ => Some(measured.iter().sum()),
    };
    (grams, rest.join(" ").trim().to_string())
}

/// Read a number whether the API sent it as a number or a numeric string.
fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_f64().or(x.as_str().and_then(|s| s.trim().parse().ok())))
}

/// The value of the nutrient named `name` in `unit` (e.g. "Energy", "KCAL").
fn nutrient(e: &Value, name: &str, unit: &str) -> Option<f64> {
    e.get("foodNutrients")?.as_array()?.iter().find(|n| {
        let is = |k: &str, want: &str| n.get(k).and_then(|v| v.as_str()).is_some_and(|v| v.eq_ignore_ascii_case(want));
        is("nutrientName", name) && is("unitName", unit)
    }).and_then(|n| num(n.get("value")))
}

/// Flatten a FoodData Central `/foods/search` response (`{"foods": [...]}`)
/// into coach-facing candidates. Search results give nutrients per 100 g for
/// every data type, so `grams` is 100; the label's serving is kept as text.
/// Every lookup is soft: if the USDA shape shifts, candidates lose their
/// numbers rather than the screen breaking.
pub fn normalize_fdc(body: Value) -> Vec<Candidate> {
    let entry = |e: &Value| -> Option<Candidate> {
        let fdc_id = e.get("fdcId").and_then(|v| v.as_i64().map(|n| n.to_string()).or(v.as_str().map(str::to_string)))?;
        let name = e.get("description").and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())?;
        let serving = num(e.get("servingSize")).map(|size| {
            let unit = e.get("servingSizeUnit").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let household = e.get("householdServingFullText").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
            match household {
                Some(h) => format!("{h} ({size} {unit})"),
                None => format!("{size} {unit}"),
            }
        });
        let kcal = nutrient(e, "Energy", "KCAL");
        Some(Candidate {
            fdc_id,
            name,
            serving,
            grams: kcal.map(|_| 100.0),
            kcal,
            protein: nutrient(e, "Protein", "G"),
            carbs: nutrient(e, "Carbohydrate, by difference", "G"),
            fat: nutrient(e, "Total lipid (fat)", "G"),
        })
    };
    let mut out: Vec<Candidate> = vec![];
    for c in body.get("foods").and_then(|v| v.as_array()).into_iter().flatten().filter_map(entry) {
        if !out.iter().any(|o| o.fdc_id == c.fdc_id) {
            out.push(c);
        }
    }
    out.truncate(MAX_CANDIDATES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn line(text: &str) -> Item {
        Item { text: text.into(), grams: None, food: None, kcal: None, protein: None, carbs: None, fat: None, basis: Basis::FreeText }
    }

    #[test]
    fn parse_item_reads_amounts_but_not_everything() {
        let cases = [
            ("chicken 12oz", (Some(340.2), "chicken")),
            ("2 chicken breasts 6oz each", (Some(340.2), "chicken breasts")),
            ("1.5 lb steak", (Some(680.4), "steak")),
            ("broccoli 200 g", (Some(200.0), "broccoli")),
            ("12 oz chicken", (Some(340.2), "chicken")),
            ("rice", (None, "rice")),
            ("2 apples", (None, "apples")),
            ("butter 2 tbsp", (None, "butter tbsp")),
        ];
        for (text, want) in cases {
            let (g, rest) = parse_item(text.to_string());
            match (g, want.0) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 0.05, "{text}: {a} g, want {b}"),
                (None, None) => {},
                (a, b) => panic!("{text}: {a:?} vs want {b:?}"),
            }
            assert_eq!(rest, want.1, "{text}: rest {rest}");
        }
    }

    #[test]
    fn validate_fills_grams_from_a_typed_line() {
        let mut meal = Meal { id: "m".into(), date: d(26), label: Some("  Lunch ".into()), items: vec![line("chicken 12oz")], updated_at: Utc::now() };
        assert!(meal.validate().is_ok());
        assert_eq!(meal.label.as_ref().unwrap(), "Lunch");
        let g: &f64 = meal.items[0].grams.as_ref().unwrap_or(&0.0);
        assert!(*g > 340.0);
        assert_eq!(meal.items[0].text, "chicken");
        let mut big = Meal { id: "m".into(), date: d(26), label: None, items: vec![Item { text: "cake".into(), grams: None, food: None, kcal: Some(25_000.0), protein: None, carbs: None, fat: None, basis: Basis::Manual }], updated_at: Utc::now() };
        assert_eq!(big.validate(), Err("an item's estimate must be 0-20000".into()), "over-bound kcal is refused");
    }

    #[test]
    fn a_snapshot_scales_to_the_parsed_amount() {
        let snap = FoodSnap { fdc_id: "1702589".into(), description: Some("Chicken breast".into()), per_grams: 100.0, kcal_per: 165.0, protein_per: Some(31.0), carbs_per: None, fat_per: None };
        let chicken = Item { text: "chicken".into(), grams: Some(340.2), food: Some(snap), kcal: None, protein: None, carbs: None, fat: None, basis: Basis::Searched };
        let chicken_kcal: f64 = chicken.energy();
        assert!((chicken_kcal - 561.3).abs() < 0.1, "energy {chicken_kcal}");
        let beer = Item { text: "beer".into(), grams: Some(360.0), food: None, kcal: Some(150.0), protein: None, carbs: None, fat: None, basis: Basis::Manual };
        assert_eq!(beer.energy(), 150.0, "a coach's number wins over any guess");
        let raw = line("something");
        assert_eq!(raw.energy(), 0.0, "unestimated items are 0, shown as not logged");
        assert!(!raw.estimated());
    }

    #[test]
    fn totals_sum_whole_meals_per_day() {
        let snap = FoodSnap { fdc_id: "1".into(), description: None, per_grams: 100.0, kcal_per: 165.0, protein_per: None, carbs_per: None, fat_per: None };
        let chicken = Item { text: "chicken".into(), grams: Some(340.2), food: Some(snap), kcal: None, protein: None, carbs: None, fat: None, basis: Basis::Searched };
        let lunch = Meal { id: "l".into(), date: d(26), label: Some("Lunch".into()), items: vec![chicken], updated_at: Utc::now() };
        let dinner = Meal { id: "r".into(), date: d(26), label: None, items: vec![Item { text: "pizza".into(), grams: None, food: None, kcal: Some(700.0), protein: None, carbs: None, fat: None, basis: Basis::Manual }], updated_at: Utc::now() };
        let snack = Meal { id: "s".into(), date: d(25), label: Some("Snack".into()), items: vec![line("apple")], updated_at: Utc::now() };
        let t = totals(&[lunch, dinner, snack]);
        assert_eq!(t.len(), 2, "day 25 has nothing estimated, but it was logged: still out only if no items at all");
        // apple is logged but unestimated (energy 0): the day still counts as logged.
        assert_eq!(t[0].0, d(25), "in date order");
        assert_eq!(t[0].1.kcal, 0.0, "logged but nothing estimated");
        let lunch_dinner: f64 = t[1].1.kcal;
        assert!((lunch_dinner - 1261.3).abs() < 0.1, "day total {lunch_dinner}");
    }

    #[test]
    fn macros_scale_per_figure_and_hand_numbers_win() {
        let snap = FoodSnap { fdc_id: "1".into(), description: None, per_grams: 100.0, kcal_per: 130.0, protein_per: Some(2.7), carbs_per: Some(28.0), fat_per: None };
        let rice = Item { text: "rice".into(), grams: Some(200.0), food: Some(snap), kcal: None, protein: Some(10.0), carbs: None, fat: None, basis: Basis::Searched };
        let m = rice.macros();
        assert_eq!((m.kcal, m.protein, m.carbs, m.fat), (260.0, 10.0, 56.0, 0.0), "protein set by hand; fat unknown is 0");
    }

    #[test]
    fn summary_averages_logged_days_and_counts_days_on_target() {
        let m = |kcal: f64, protein: f64| Macros { kcal, protein, carbs: 0.0, fat: 0.0 };
        let days = [(d(19), m(5000.0, 0.0)), (d(24), m(2000.0, 150.0)), (d(25), m(2300.0, 130.0)), (d(26), m(1500.0, 110.0))];
        let target = Target { kcal: Some(2100.0), ..Default::default() };
        let s = summary(&days, d(26), 7, Some(&target));
        assert_eq!(s.logged, 3, "day 19 is outside the 7 days");
        assert_eq!(s.average.kcal, 1933.3333333333333);
        assert_eq!(s.average.protein, 130.0);
        assert_eq!(s.on_target, Some(2), "2000 and 2300 are within 10% of 2100; 1500 is not");
        assert_eq!(summary(&[], d(26), 7, None), Summary { days: 7, ..Default::default() }, "nothing logged averages to 0, not NaN");
        let mut bad = Target { kcal: Some(100.0), ..Default::default() };
        assert!(bad.validate().is_err());
        let mut ok = Target { kcal: Some(2100.0), protein: Some(160.0), note: Some("  ".into()), ..Default::default() };
        assert!(ok.validate().is_ok());
        assert_eq!(ok.note, None, "a blank note is dropped");
    }

    #[test]
    fn normalize_fdc_reads_the_search_response() {
        // Trimmed from a real /foods/search answer.
        let body = json!({
            "totalHits": 3,
            "foods": [{
                "fdcId": 2187885, "description": "CHICKEN BREAST", "dataType": "Branded",
                "servingSize": 284.0, "servingSizeUnit": "g", "householdServingFullText": "1 breast",
                "foodNutrients": [
                    { "nutrientName": "Protein", "unitName": "G", "value": 20.4 },
                    { "nutrientName": "Total lipid (fat)", "unitName": "G", "value": 3.6 },
                    { "nutrientName": "Carbohydrate, by difference", "unitName": "G", "value": 0 },
                    { "nutrientName": "Energy", "unitName": "kJ", "value": 690 },
                    { "nutrientName": "Energy", "unitName": "KCAL", "value": 165 }
                ]
            }, {
                "fdcId": 2187885, "description": "a duplicate", "foodNutrients": []
            }, {
                "fdcId": 169756, "description": " Rice, white, cooked ", "dataType": "SR Legacy", "foodNutrients": []
            }]
        });
        let cs = normalize_fdc(body);
        assert_eq!(cs.len(), 2, "the duplicate fdcId is dropped");
        assert_eq!(cs[0].fdc_id, "2187885", "a numeric id becomes a string");
        assert_eq!(cs[0].kcal, Some(165.0), "kcal, not kJ");
        assert_eq!(cs[0].protein, Some(20.4));
        assert_eq!((cs[0].fat, cs[0].carbs), (Some(3.6), Some(0.0)));
        assert_eq!(cs[0].grams, Some(100.0), "search nutrients are per 100 g");
        assert_eq!(cs[0].serving.as_deref(), Some("1 breast (284 g)"));
        assert_eq!(cs[1].name, "Rice, white, cooked", "trimmed");
        assert_eq!((cs[1].kcal, cs[1].grams), (None, None), "no nutrients come through with no numbers");
        let snap = cs[1].snap();
        assert_eq!(snap.kcal_per, 0.0, "a pick with no data snapshots as 0 per 100 g");
        assert_eq!(snap.per_grams, 100.0);
        assert!(normalize_fdc(json!({ "unexpected": true })).is_empty());
    }
}
