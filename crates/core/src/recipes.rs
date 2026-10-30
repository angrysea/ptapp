//! Recipes, recipe books and meal plans (nutrition). A recipe is ingredient
//! lines — the food journal's `Item`s, so each can carry a USDA snapshot or
//! numbers set by hand — plus steps and a number of servings; its macros per
//! serving add up from the ingredients unless set by hand. A coach keeps
//! recipes in books shared with all or chosen clients; a client keeps their
//! own. A meal plan lays recipes out by week and day, and is assigned to a
//! client with the recipes it uses copied in, so the client can open them.
//! Importing (from text, a web page or a PDF's text) is `recipeimport.rs`.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::nutrition::{Item, Macros};

pub const MAX_INGREDIENTS: usize = 60;
pub const MAX_STEPS: usize = 60;
pub const MAX_STEP: usize = 2000;
pub const MAX_TITLE: usize = 120;

/// `COACH#{coach}` or `CLIENT#{client}` / `RECIPE#{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The coach's book it's filed in; none for a client's own recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_id: Option<String>,
    pub servings: f64,
    #[serde(default)]
    pub ingredients: Vec<Item>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prep_min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cook_min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Per-serving numbers set by hand (or stated by an imported page),
    /// used instead of the ingredients' sum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_serving: Option<Macros>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

impl Recipe {
    /// The whole recipe's macros, from its ingredients.
    pub fn total(&self) -> Macros {
        let mut t = Macros::default();
        for i in &self.ingredients {
            let m = i.macros();
            t.kcal += m.kcal;
            t.protein += m.protein;
            t.carbs += m.carbs;
            t.fat += m.fat;
        }
        t
    }

    /// One serving: the numbers set by hand, else the total over servings.
    pub fn serving(&self) -> Macros {
        if let Some(m) = self.per_serving {
            return m;
        }
        let t = self.total();
        let n = if self.servings > 0.0 { self.servings } else { 1.0 };
        Macros { kcal: t.kcal / n, protein: t.protein / n, carbs: t.carbs / n, fat: t.fat / n }
    }

    /// Whether every ingredient has an estimate, so `serving()` is complete.
    pub fn fully_estimated(&self) -> bool {
        self.per_serving.is_some() || self.ingredients.iter().all(|i| i.estimated())
    }

    pub fn validate(&mut self) -> Result<(), String> {
        self.title = self.title.trim().to_string();
        self.description = self.description.trim().chars().take(2000).collect();
        if self.title.is_empty() || self.title.chars().count() > MAX_TITLE {
            return Err(format!("a recipe's title must be 1-{MAX_TITLE} characters"));
        }
        if !(self.servings > 0.0 && self.servings <= 100.0) {
            return Err("servings must be above 0 and at most 100".into());
        }
        if self.ingredients.is_empty() || self.ingredients.len() > MAX_INGREDIENTS {
            return Err(format!("a recipe has 1-{MAX_INGREDIENTS} ingredients"));
        }
        // An ingredient is a food-journal line: same bounds, same parsing.
        let mut as_meal = crate::nutrition::Meal {
            id: String::new(),
            date: NaiveDate::MIN,
            label: None,
            items: std::mem::take(&mut self.ingredients),
            updated_at: Utc::now(),
        };
        // Meals stop at 12 lines; recipes go to 60, checked a dozen at a time.
        let mut checked = vec![];
        for chunk in as_meal.items.chunks(crate::nutrition::MAX_ITEMS) {
            let mut part = crate::nutrition::Meal { items: chunk.to_vec(), ..as_meal.clone() };
            part.validate().map_err(|e| e.replace("a meal has", "a recipe has").replace("an item", "an ingredient"))?;
            checked.extend(part.items);
        }
        as_meal.items = checked;
        self.ingredients = as_meal.items;
        self.steps = self.steps.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if self.steps.len() > MAX_STEPS || self.steps.iter().any(|s| s.chars().count() > MAX_STEP) {
            return Err(format!("up to {MAX_STEPS} steps of {MAX_STEP} characters"));
        }
        self.tags = self.tags.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty() && t.chars().count() <= 30).take(10).collect();
        if self.source_url.as_ref().is_some_and(|u| u.len() > 500 || !(u.starts_with("https://") || u.starts_with("http://"))) {
            return Err("a source link must be an http(s) address".into());
        }
        if self.prep_min.is_some_and(|m| m > 24 * 60) || self.cook_min.is_some_and(|m| m > 7 * 24 * 60) {
            return Err("prep and cook times look wrong".into());
        }
        if let Some(m) = self.per_serving
            && [m.kcal, m.protein, m.carbs, m.fat].iter().any(|n| !(0.0..=crate::nutrition::MAX_KCAL).contains(n))
        {
            return Err("per-serving numbers must be 0-20000".into());
        }
        Ok(())
    }
}

/// Who can see a coach's book.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", tag = "with")]
pub enum Sharing {
    /// The coach only.
    #[default]
    Nobody,
    /// Every client of the coach.
    All,
    /// These clients.
    Clients { ids: Vec<String> },
}

impl Sharing {
    pub fn includes(&self, client_id: &str) -> bool {
        match self {
            Sharing::Nobody => false,
            Sharing::All => true,
            Sharing::Clients { ids } => ids.iter().any(|i| i == client_id),
        }
    }
}

/// `COACH#{coach}` / `BOOK#{id}`: a named collection of the coach's recipes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub shared: Sharing,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

impl Book {
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_string();
        self.description = self.description.trim().chars().take(500).collect();
        if self.name.is_empty() || self.name.chars().count() > 80 {
            return Err("a book's name must be 1-80 characters".into());
        }
        if let Sharing::Clients { ids } = &self.shared
            && ids.len() > 500
        {
            return Err("share with up to 500 clients, or all".into());
        }
        Ok(())
    }
}

/// One meal in a plan's day: a recipe and how many servings of it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanMeal {
    /// "Breakfast", "Lunch", "Dinner", "Snack".
    pub label: String,
    pub recipe_id: String,
    pub servings: f64,
}

/// A day of a plan: week 1.., day 1-7 (Monday first).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanDay {
    pub week: u32,
    pub day: u32,
    #[serde(default)]
    pub meals: Vec<PlanMeal>,
}

/// `COACH#{coach}` / `MEALPLAN#{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MealPlan {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// How many weeks it lasts before repeating (assignments repeat it).
    pub weeks: u32,
    #[serde(default)]
    pub days: Vec<PlanDay>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

impl MealPlan {
    pub fn validate(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_string();
        self.description = self.description.trim().chars().take(1000).collect();
        if self.name.is_empty() || self.name.chars().count() > 80 {
            return Err("a meal plan's name must be 1-80 characters".into());
        }
        if !(1..=12).contains(&self.weeks) {
            return Err("a meal plan lasts 1-12 weeks".into());
        }
        self.days.retain(|d| !d.meals.is_empty());
        for d in &mut self.days {
            if d.week < 1 || d.week > self.weeks || !(1..=7).contains(&d.day) {
                return Err(format!("week {} day {} is outside the plan", d.week, d.day));
            }
            if d.meals.len() > 8 {
                return Err("up to 8 meals a day".into());
            }
            for m in &mut d.meals {
                m.label = m.label.trim().chars().take(40).collect();
                if m.label.is_empty() {
                    m.label = "Meal".into();
                }
                if !(m.servings > 0.0 && m.servings <= 20.0) {
                    return Err("a meal is above 0 and at most 20 servings".into());
                }
            }
        }
        self.days.sort_by_key(|d| (d.week, d.day));
        if self.days.windows(2).any(|w| (w[0].week, w[0].day) == (w[1].week, w[1].day)) {
            return Err("a day appears twice in the plan".into());
        }
        Ok(())
    }

    pub fn recipe_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.days.iter().flat_map(|d| d.meals.iter().map(|m| m.recipe_id.clone())).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Calories and macros for each planned day, from the recipes.
    pub fn day_totals(&self, recipes: &HashMap<String, Recipe>) -> Vec<(u32, u32, Macros)> {
        self.days
            .iter()
            .map(|d| {
                let mut t = Macros::default();
                for m in &d.meals {
                    if let Some(r) = recipes.get(&m.recipe_id) {
                        let s = r.serving();
                        t.kcal += s.kcal * m.servings;
                        t.protein += s.protein * m.servings;
                        t.carbs += s.carbs * m.servings;
                        t.fat += s.fat * m.servings;
                    }
                }
                (d.week, d.day, t)
            })
            .collect()
    }
}

/// `CLIENT#{client}` / `MEALPLAN`: the plan a client follows, with the
/// recipes it uses copied in so the client can open them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssignedPlan {
    pub plan: MealPlan,
    pub recipes: HashMap<String, Recipe>,
    /// Week 1 day 1 falls on the Monday of this date's week.
    pub start_date: NaiveDate,
    pub assigned_at: DateTime<Utc>,
}

impl AssignedPlan {
    /// The plan's week and day for `date`, repeating after the last week;
    /// none before it starts.
    pub fn slot(&self, date: NaiveDate) -> Option<(u32, u32)> {
        use chrono::Datelike;
        let monday = self.start_date - chrono::Duration::days(self.start_date.weekday().num_days_from_monday() as i64);
        if date < self.start_date {
            return None;
        }
        let days = (date - monday).num_days();
        let week = (days / 7) as u32 % self.plan.weeks + 1;
        Some((week, date.weekday().num_days_from_monday() + 1))
    }

    /// The meals planned for `date`.
    pub fn meals_on(&self, date: NaiveDate) -> Vec<&PlanMeal> {
        let Some((week, day)) = self.slot(date) else { return vec![] };
        self.plan.days.iter().filter(|d| d.week == week && d.day == day).flat_map(|d| d.meals.iter()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nutrition::{Basis, FoodSnap};

    fn line(text: &str, grams: Option<f64>, kcal_per_100: Option<f64>) -> Item {
        Item {
            text: text.into(),
            grams,
            food: kcal_per_100.map(|k| FoodSnap { fdc_id: "1".into(), description: None, per_grams: 100.0, kcal_per: k, protein_per: Some(10.0), carbs_per: None, fat_per: None }),
            kcal: None,
            protein: None,
            carbs: None,
            fat: None,
            basis: if kcal_per_100.is_some() { Basis::Searched } else { Basis::FreeText },
        }
    }

    fn recipe() -> Recipe {
        Recipe {
            id: "r".into(),
            title: " Overnight oats ".into(),
            description: String::new(),
            book_id: None,
            servings: 2.0,
            ingredients: vec![line("oats", Some(100.0), Some(380.0)), line("milk", Some(200.0), Some(60.0)), line("cinnamon 1 tsp", None, None)],
            steps: vec!["Mix".into(), "  ".into(), "Chill overnight".into()],
            prep_min: Some(5),
            cook_min: None,
            source_url: None,
            tags: vec![" Breakfast ".into()],
            per_serving: None,
            updated_at: None,
        }
    }

    #[test]
    fn a_recipe_sums_its_ingredients_per_serving() {
        let mut r = recipe();
        assert!(r.validate().is_ok());
        assert_eq!((r.title.as_str(), r.steps.len(), r.tags[0].as_str()), ("Overnight oats", 2, "breakfast"));
        assert_eq!(r.total().kcal, 500.0, "380 + 120");
        assert_eq!(r.serving().kcal, 250.0);
        assert_eq!(r.serving().protein, 15.0, "10 g per 100 g of 300 g, over 2");
        assert!(!r.fully_estimated(), "the cinnamon has no estimate");
        r.per_serving = Some(Macros { kcal: 300.0, protein: 12.0, carbs: 40.0, fat: 8.0 });
        assert_eq!(r.serving().kcal, 300.0, "numbers set by hand win");
        assert!(r.fully_estimated());
    }

    #[test]
    fn recipes_validate() {
        let mut none = Recipe { ingredients: vec![], ..recipe() };
        assert!(none.validate().is_err());
        let mut zero = Recipe { servings: 0.0, ..recipe() };
        assert!(zero.validate().is_err());
        let mut bad_url = Recipe { source_url: Some("javascript:alert(1)".into()), ..recipe() };
        assert!(bad_url.validate().is_err());
        let mut long = Recipe { ingredients: (0..30).map(|i| line(&format!("item {i} 10 g"), None, None)).collect(), ..recipe() };
        assert!(long.validate().is_ok(), "more ingredients than a meal's 12");
        assert_eq!(long.ingredients[0].grams, Some(10.0), "lines are parsed like the food journal's");
    }

    #[test]
    fn sharing_and_plans() {
        assert!(Sharing::All.includes("c1") && !Sharing::Nobody.includes("c1"));
        assert!(Sharing::Clients { ids: vec!["c1".into()] }.includes("c1"));
        let meal = |r: &str, s: f64| PlanMeal { label: "Lunch".into(), recipe_id: r.into(), servings: s };
        let mut plan = MealPlan {
            id: "p".into(), name: "Cut".into(), description: String::new(), weeks: 2, updated_at: None,
            days: vec![PlanDay { week: 1, day: 1, meals: vec![meal("r", 1.0), meal("r", 0.5)] }, PlanDay { week: 2, day: 3, meals: vec![meal("x", 1.0)] }, PlanDay { week: 1, day: 2, meals: vec![] }],
        };
        assert!(plan.validate().is_ok());
        assert_eq!(plan.days.len(), 2, "empty days are dropped");
        assert_eq!(plan.recipe_ids(), vec!["r".to_string(), "x".to_string()]);
        let mut r = recipe();
        r.validate().unwrap();
        let totals = plan.day_totals(&HashMap::from([("r".to_string(), r.clone())]));
        assert_eq!(totals[0].2.kcal, 375.0, "1.5 servings of 250");
        let mut out = MealPlan { weeks: 1, ..plan.clone() };
        assert!(out.validate().is_err(), "week 2 is outside a 1-week plan");

        let a = AssignedPlan { plan, recipes: HashMap::new(), start_date: NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(), assigned_at: Utc::now() };
        let d = |day: u32| NaiveDate::from_ymd_opt(2026, 11, day).unwrap();
        assert_eq!(a.slot(d(3)), None, "before it starts");
        assert_eq!(a.slot(d(4)), Some((1, 3)), "a Wednesday start is week 1 day 3");
        assert_eq!(a.slot(d(11)), Some((2, 3)));
        assert_eq!(a.slot(d(16)), Some((1, 1)), "repeats after week 2");
        assert_eq!(a.meals_on(d(16)).len(), 2);
    }
}
