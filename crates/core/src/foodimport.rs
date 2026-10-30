//! Import a food diary exported from another app (BZ-4): Cronometer's
//! "Food & Recipe Entries" CSV (one row per food) or MyFitnessPal's
//! "Nutrition" CSV (one row per meal). Neither has an open API, so the
//! client exports a file and uploads it. Columns are found by header name,
//! not position, so other apps' exports with a date and a calorie column work
//! too; rows become meals with `Basis::Imported` items.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::nutrition::{parse_item, Basis, Item, Meal, MAX_ITEMS, MAX_KCAL, MAX_LABEL, MAX_TEXT};

/// Rows read from one file; a year of Cronometer entries is ~3 000.
pub const MAX_ROWS: usize = 5_000;
/// How far back an import reaches: as far as the journal lists.
pub const MAX_DAYS_BACK: i64 = 90;

#[derive(Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub meals: Vec<Meal>,
    /// Rows with no date, no numbers, or a date outside the window.
    pub skipped: usize,
    /// "cronometer", "myfitnesspal" or "csv": what the headers looked like.
    pub source: String,
}

/// Split CSV text into rows of fields: quoted fields may hold commas,
/// newlines and doubled quotes; a leading BOM and CRLF endings are fine.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = vec![];
    let mut row: Vec<String> = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

/// "Energy (kcal)" → ("energy", "kcal"); "Protein" → ("protein", "").
fn header(h: &str) -> (String, String) {
    let h = h.trim().to_lowercase();
    match h.split_once('(') {
        Some((base, unit)) => (base.trim().to_string(), unit.trim_end_matches(')').trim().to_string()),
        None => (h, String::new()),
    }
}

struct Columns {
    date: usize,
    label: Option<usize>,
    name: Option<usize>,
    amount: Option<usize>,
    kcal: Option<usize>,
    protein: Option<usize>,
    carbs: Option<usize>,
    fat: Option<usize>,
}

fn columns(head: &[String]) -> Option<Columns> {
    let find = |names: &[&str]| head.iter().position(|h| {
        let (base, unit) = header(h);
        names.contains(&base.as_str()) && unit != "kj"
    });
    Some(Columns {
        date: find(&["day", "date"])?,
        label: find(&["group", "meal"]),
        name: find(&["food name", "food", "description", "item"]),
        amount: find(&["amount", "quantity", "serving"]),
        kcal: find(&["energy", "calories", "kcal"]),
        protein: find(&["protein"]),
        carbs: find(&["carbs", "carbohydrates", "carbohydrate", "total carbohydrate"]),
        fat: find(&["fat", "total fat", "total lipid"]),
    })
}

/// ISO dates, and the US month/day/year some exports use.
fn date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    let s = s.split_whitespace().next().unwrap_or(s);
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().or_else(|| NaiveDate::parse_from_str(s, "%m/%d/%Y").ok())
}

/// A number in a cell: "1,234.5" and "" (none) both handled; out-of-range
/// figures are dropped rather than refusing the whole file.
fn figure(row: &[String], col: Option<usize>) -> Option<f64> {
    let cell = row.get(col?)?.trim().replace(',', "");
    let n: f64 = cell.parse().ok()?;
    (n.is_finite() && (0.0..=MAX_KCAL).contains(&n)).then_some(n)
}

fn cell(row: &[String], col: Option<usize>) -> Option<String> {
    row.get(col?).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Lowercase letters and digits only, for stable meal ids.
fn slug(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).take(16).collect()
}

/// Read an export into meals dated `today - 89 … today`. Meals group rows by
/// day and meal name, in file order; a meal with more than `MAX_ITEMS` foods
/// continues in "Lunch (2)". Ids are derived from day, meal and part, so
/// importing the same file again replaces those meals instead of doubling them.
pub fn import(text: &str, today: NaiveDate) -> Result<Imported, String> {
    let rows = parse_csv(text);
    let Some((head, body)) = rows.split_first() else { return Err("the file is empty".into()) };
    let Some(cols) = columns(head) else { return Err("no Date or Day column: export a food diary as CSV from Cronometer or MyFitnessPal".into()) };
    if cols.kcal.is_none() && cols.name.is_none() {
        return Err("no calories or food name column found".into());
    }
    if body.len() > MAX_ROWS {
        return Err(format!("up to {MAX_ROWS} rows at a time; export a shorter date range"));
    }
    let source = match (cols.name.is_some(), cols.label.is_some()) {
        (true, true) if head.iter().any(|h| header(h).0 == "group") => "cronometer",
        (false, true) => "myfitnesspal",
        _ => "csv",
    };
    let from = today - Duration::days(MAX_DAYS_BACK - 1);
    let mut order: Vec<(NaiveDate, String)> = vec![];
    let mut groups: HashMap<(NaiveDate, String), Vec<Item>> = HashMap::new();
    let mut skipped = 0;
    for row in body {
        let Some(d) = cell(row, Some(cols.date)).and_then(|s| date(&s)) else { skipped += 1; continue };
        let (kcal, protein, carbs, fat) = (figure(row, cols.kcal), figure(row, cols.protein), figure(row, cols.carbs), figure(row, cols.fat));
        let name = cell(row, cols.name);
        if d < from || d > today || (kcal.is_none() && name.is_none()) {
            skipped += 1;
            continue;
        }
        let label: String = cell(row, cols.label).unwrap_or_else(|| "Imported".into()).chars().take(MAX_LABEL).collect();
        let amount = cell(row, cols.amount);
        let grams = amount.as_ref().and_then(|a| parse_item(a.clone()).0);
        let mut text = name.unwrap_or_else(|| format!("{label} total"));
        if grams.is_none() && let Some(a) = amount {
            // "1.00 cup" can't become grams; keep it readable on the line.
            text = format!("{text}, {a}");
        }
        let item = Item {
            text: text.chars().take(MAX_TEXT).collect(),
            grams: grams.filter(|g| *g > 0.0 && *g <= crate::nutrition::MAX_GRAMS),
            food: None,
            kcal,
            protein,
            carbs,
            fat,
            basis: Basis::Imported,
        };
        let key = (d, label);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(item);
    }
    let now = Utc::now();
    let mut meals: Vec<Meal> = vec![];
    for key in order {
        let items = groups.remove(&key).unwrap_or_default();
        let (d, label) = key;
        for (part, chunk) in items.chunks(MAX_ITEMS).enumerate() {
            let id = format!("imp{}{}{}", d.format("%Y%m%d"), slug(&label), part);
            let label = if part == 0 { label.clone() } else { format!("{label} ({})", part + 1).chars().take(MAX_LABEL).collect() };
            let mut meal = Meal { id, date: d, label: Some(label), items: chunk.to_vec(), updated_at: now };
            meal.validate()?;
            meals.push(meal);
        }
    }
    Ok(Imported { meals, skipped, source: source.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn csv_fields_keep_quoted_commas_and_quotes() {
        let rows = parse_csv("\u{feff}a,b,c\r\n\"x, y\",\"say \"\"hi\"\"\",3\n\n1,2,3");
        assert_eq!(rows.len(), 3, "blank lines are dropped");
        assert_eq!(rows[0], vec!["a", "b", "c"], "the BOM is gone");
        assert_eq!(rows[1], vec!["x, y", "say \"hi\"", "3"]);
        assert_eq!(rows[2], vec!["1", "2", "3"], "a last row with no newline");
    }

    #[test]
    fn cronometer_rows_become_food_lines_per_meal() {
        let csv = "Day,Time,Group,Food Name,Amount,Energy (kcal),Alcohol (g),Carbs (g),Fiber (g),Fat (g),Saturated (g),Protein (g),Category\n\
            2026-10-28,12:01 PM,Lunch,\"Chicken Breast, Roasted\",150.00 g,247.50,0.00,0.00,0.00,5.40,1.52,46.50,Poultry\n\
            2026-10-28,12:02 PM,Lunch,\"Rice, White, Cooked\",1.00 cup,205.4,0,44.5,0.6,0.4,0.1,4.3,Grains\n\
            2026-10-28,8:00 AM,Breakfast,Banana,118 g,105,0,27,3.1,0.4,0.1,1.3,Fruit\n\
            2026-07-01,8:00 AM,Breakfast,Too old,100 g,100,0,0,0,0,0,0,x\n\
            2026-10-29,8:00 AM,Breakfast,Kept,100 g,,0,,0,,0,,x\n";
        let out = import(csv, d(29)).unwrap();
        assert_eq!(out.source, "cronometer");
        assert_eq!(out.skipped, 1, "outside the 90-day window");
        assert_eq!(out.meals.len(), 3);
        let lunch = &out.meals[0];
        assert_eq!((lunch.date, lunch.label.as_deref()), (d(28), Some("Lunch")));
        assert_eq!(lunch.items[0].text, "Chicken Breast, Roasted");
        assert_eq!(lunch.items[0].grams, Some(150.0));
        assert_eq!(lunch.items[0].macros().protein, 46.5, "Protein (g), not Saturated");
        assert_eq!(lunch.items[0].fat, Some(5.4), "Fat (g), not Saturated (g)");
        assert_eq!(lunch.items[1].text, "Rice, White, Cooked, 1.00 cup", "a cup stays text");
        assert_eq!(lunch.items[1].carbs, Some(44.5));
        assert_eq!(lunch.items[0].basis, Basis::Imported);
        assert_eq!(out.meals[2].items[0].kcal, None, "a named food with blank numbers is still a line");
        let again = import(csv, d(29)).unwrap();
        assert_eq!(again.meals[0].id, lunch.id, "the same file gives the same ids");
    }

    #[test]
    fn myfitnesspal_rows_are_meal_totals() {
        let csv = "Date,Meal,Time,Calories,Fat (g),Saturated Fat,Polyunsaturated Fat,Cholesterol,Sodium (mg),Carbohydrates (g),Fiber,Sugar,Protein (g),Note\n\
            2026-10-27,Breakfast,8:15 AM,\"1,050.0\",30.1,9,2,200,800,120.5,6,20,55.2,\n\
            2026-10-27,Dinner,7:00 PM,640,22,7,3,90,700,60,5,8,45,\n\
            not a date,Dinner,,100,1,1,1,1,1,1,1,1,1,\n";
        let out = import(csv, d(29)).unwrap();
        assert_eq!(out.source, "myfitnesspal");
        assert_eq!(out.skipped, 1);
        assert_eq!(out.meals.len(), 2);
        let b = &out.meals[0].items[0];
        assert_eq!(b.text, "Breakfast total");
        assert_eq!(b.kcal, Some(1050.0), "thousands separators are read");
        assert_eq!((b.fat, b.carbs, b.protein), (Some(30.1), Some(120.5), Some(55.2)));
    }

    #[test]
    fn long_meals_continue_and_bad_files_say_why() {
        let mut csv = String::from("Date,Meal,Food,Calories\n");
        for i in 0..(MAX_ITEMS + 3) {
            csv.push_str(&format!("10/28/2026,Dinner,item {i},10\n"));
        }
        let out = import(&csv, d(29)).unwrap();
        assert_eq!(out.meals.len(), 2, "US dates read too");
        assert_eq!(out.meals[1].label.as_deref(), Some("Dinner (2)"));
        assert_eq!(out.meals[1].items.len(), 3);
        assert_ne!(out.meals[0].id, out.meals[1].id);
        assert!(import("", d(29)).is_err());
        assert!(import("Food,Calories\nx,1\n", d(29)).unwrap_err().contains("Date"));
        assert!(import("Date,Note\n2026-10-28,x\n", d(29)).is_err(), "nothing to import from");
    }
}
