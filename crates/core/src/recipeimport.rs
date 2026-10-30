//! Turning a recipe someone already has into a `Recipe`, by rules (no AI):
//! pasted text (or the text of a PDF, read on the device), or a web page.
//! Most recipe sites publish the recipe as schema.org JSON-LD, which is read
//! field by field; a page without it is reduced to text and read like a
//! paste. What comes out is a draft for the coach or client to check: lines
//! that couldn't be told apart end up as steps, never dropped.

use serde_json::Value;

use crate::nutrition::{Basis, Item, Macros};
use crate::recipes::{Recipe, MAX_INGREDIENTS, MAX_STEPS, MAX_TITLE};

/// What an import made, with notes on anything it had to guess.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub recipe: Recipe,
    pub notes: Vec<String>,
}

fn ingredient(text: &str) -> Item {
    Item { text: text.chars().take(crate::nutrition::MAX_TEXT).collect(), grams: None, food: None, kcal: None, protein: None, carbs: None, fat: None, basis: Basis::FreeText }
}

fn blank(title: &str) -> Recipe {
    Recipe {
        id: String::new(),
        title: title.chars().take(MAX_TITLE).collect(),
        description: String::new(),
        book_id: None,
        servings: 1.0,
        ingredients: vec![],
        steps: vec![],
        prep_min: None,
        cook_min: None,
        source_url: None,
        tags: vec![],
        per_serving: None,
        updated_at: None,
    }
}

/// "½" → "0.5", "1 1/2" → "1.5", "1/4" → "0.25", so amounts parse.
pub fn decimal_fractions(s: &str) -> String {
    let glyphs = [('½', 0.5), ('⅓', 1.0 / 3.0), ('⅔', 2.0 / 3.0), ('¼', 0.25), ('¾', 0.75), ('⅛', 0.125), ('⅕', 0.2)];
    let words: Vec<String> = s.split_whitespace().map(|w| w.to_string()).collect();
    let mut out: Vec<String> = vec![];
    let fraction = |w: &str| -> Option<f64> {
        if let Some((_, v)) = glyphs.iter().find(|(g, _)| w == g.to_string()) {
            return Some(*v);
        }
        let (a, b) = w.split_once('/')?;
        let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
        (b > 0.0).then_some(a / b)
    };
    let fmt = |v: f64| {
        let r = (v * 100.0).round() / 100.0;
        if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
    };
    for w in words {
        // "1½" glued together
        let glued = glyphs.iter().find(|(g, _)| w.ends_with(*g) && w.len() > g.len_utf8()).and_then(|(g, v)| w.trim_end_matches(*g).parse::<f64>().ok().map(|n| n + v));
        if let Some(v) = glued {
            out.push(fmt(v));
        } else if let Some(f) = fraction(&w) {
            // "1 1/2": add to the whole number before it
            match out.last().and_then(|p| p.parse::<f64>().ok()).filter(|n| n.fract() == 0.0) {
                Some(n) => *out.last_mut().unwrap() = fmt(n + f),
                None => out.push(fmt(f)),
            }
        } else {
            out.push(w);
        }
    }
    out.join(" ")
}

/// A line's list marker removed: "- ", "• ", "* ", "▢ ", "1. ", "1) ", "Step 2:".
fn unmark(line: &str) -> (String, bool) {
    let t = line.trim();
    let bullets = ['-', '•', '*', '▢', '□', '·', '–', '—'];
    if let Some(rest) = t.strip_prefix(|c| bullets.contains(&c)) {
        return (rest.trim().to_string(), true);
    }
    let lower = t.to_lowercase();
    if let Some(rest) = lower.strip_prefix("step") {
        let digits: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let skip = t.len() - rest.trim_start().len() + digits.len();
            return (t[skip..].trim_start_matches([':', '.', ')', ' ']).trim().to_string(), true);
        }
    }
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() && digits.len() <= 2 {
        let rest = &t[digits.len()..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return (r.trim().to_string(), true);
        }
    }
    (t.to_string(), false)
}

#[derive(PartialEq, Clone, Copy)]
enum Section {
    None,
    Ingredients,
    Steps,
    Notes,
}

fn heading(line: &str) -> Option<Section> {
    let h = line.trim().trim_end_matches(':').trim().to_lowercase();
    if h.len() > 40 {
        return None;
    }
    let is = |words: &[&str]| words.iter().any(|w| h == *w || h.starts_with(&format!("{w} ")) || h.ends_with(&format!(" {w}")));
    if is(&["ingredients", "ingredient list", "you will need", "what you need"]) {
        Some(Section::Ingredients)
    } else if is(&["directions", "instructions", "method", "steps", "preparation", "how to make it", "to make"]) {
        Some(Section::Steps)
    } else if is(&["notes", "tips", "recipe notes", "nutrition"]) {
        Some(Section::Notes)
    } else {
        None
    }
}

/// The first number in a line: "Serves 4", "Yield: 6 servings", "Makes 12".
fn first_number(s: &str) -> Option<f64> {
    let s = decimal_fractions(s);
    s.split(|c: char| !(c.is_ascii_digit() || c == '.')).find(|w| !w.is_empty() && w != &".").and_then(|w| w.parse().ok())
}

/// Minutes in "Prep time: 1 hr 15 mins" or "Cook 20 minutes".
fn minutes_in(s: &str) -> Option<u32> {
    let l = s.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| c.is_whitespace() || c == ':').filter(|w| !w.is_empty()).collect();
    let mut total = 0.0;
    let mut found = false;
    for (i, w) in words.iter().enumerate() {
        let (num, unit) = match w.find(|c: char| c.is_alphabetic()) {
            Some(0) => continue,
            Some(p) => (&w[..p], &w[p..]),
            None => (*w, words.get(i + 1).copied().unwrap_or("")),
        };
        let Ok(n) = num.parse::<f64>() else { continue };
        if unit.starts_with('h') {
            total += n * 60.0;
            found = true;
        } else if unit.starts_with("min") || unit == "m" {
            total += n;
            found = true;
        }
    }
    found.then_some(total.round() as u32)
}

/// Whether a line without a heading reads like an ingredient: short, starts
/// with an amount or a list marker, and isn't a sentence.
fn looks_like_ingredient(line: &str, marked: bool) -> bool {
    let t = decimal_fractions(line);
    let starts_with_amount = t.chars().next().is_some_and(|c| c.is_ascii_digit());
    let sentence = t.ends_with('.') && t.split_whitespace().count() > 6;
    t.chars().count() <= 90 && !sentence && (starts_with_amount || (marked && t.split_whitespace().count() <= 8))
}

/// A recipe from pasted text, or a PDF's text.
pub fn from_text(text: &str) -> Draft {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut notes = vec![];
    let mut recipe = blank("");
    let mut section = Section::None;
    let mut saw_headings = false;
    let mut desc: Vec<String> = vec![];
    let mut servings: Option<f64> = None;
    for (i, raw) in lines.iter().enumerate() {
        if let Some(s) = heading(raw) {
            section = s;
            saw_headings = true;
            continue;
        }
        let lower = raw.to_lowercase();
        if (lower.starts_with("serves") || lower.starts_with("servings") || lower.starts_with("yield") || lower.starts_with("makes")) && raw.len() < 60 {
            servings = servings.or_else(|| first_number(raw));
            continue;
        }
        if lower.starts_with("prep") && raw.len() < 60 {
            recipe.prep_min = recipe.prep_min.or_else(|| minutes_in(raw));
            continue;
        }
        if (lower.starts_with("cook") || lower.starts_with("bake")) && raw.len() < 60 && minutes_in(raw).is_some() {
            recipe.cook_min = recipe.cook_min.or_else(|| minutes_in(raw));
            continue;
        }
        if lower.starts_with("total time") && raw.len() < 60 {
            continue;
        }
        if recipe.title.is_empty() && i == 0 && raw.chars().count() <= MAX_TITLE {
            recipe.title = raw.to_string();
            continue;
        }
        let (body, marked) = unmark(raw);
        if body.is_empty() {
            continue;
        }
        match section {
            Section::Ingredients => recipe.ingredients.push(ingredient(&decimal_fractions(&body))),
            Section::Steps => recipe.steps.push(body),
            Section::Notes => desc.push(body),
            Section::None if !saw_headings && looks_like_ingredient(&body, marked) => recipe.ingredients.push(ingredient(&decimal_fractions(&body))),
            // Before any heading, short unmarked lines describe the recipe.
            Section::None if recipe.ingredients.is_empty() && recipe.steps.is_empty() && body.chars().count() <= 300 && !marked => desc.push(body),
            Section::None => recipe.steps.push(body),
        }
    }
    if recipe.title.is_empty() {
        recipe.title = "Imported recipe".into();
        notes.push("No title found; name it before saving.".into());
    }
    match servings {
        Some(n) if n > 0.0 && n <= 100.0 => recipe.servings = n,
        _ => notes.push("No servings found; set to 1.".into()),
    }
    if recipe.ingredients.is_empty() {
        notes.push("No ingredient lines found; add them before saving.".into());
    }
    recipe.description = desc.join("\n");
    finish(recipe, notes)
}

fn finish(mut recipe: Recipe, mut notes: Vec<String>) -> Draft {
    if recipe.ingredients.len() > MAX_INGREDIENTS {
        recipe.ingredients.truncate(MAX_INGREDIENTS);
        notes.push(format!("Only the first {MAX_INGREDIENTS} ingredients were kept."));
    }
    if recipe.steps.len() > MAX_STEPS {
        recipe.steps.truncate(MAX_STEPS);
        notes.push(format!("Only the first {MAX_STEPS} steps were kept."));
    }
    let unmeasured = recipe.ingredients.iter().filter(|i| crate::nutrition::parse_item(i.text.clone()).0.is_none()).count();
    if unmeasured > 0 {
        notes.push(format!("{unmeasured} ingredient(s) aren't in grams, ounces or pounds; add grams to estimate their calories."));
    }
    Draft { recipe, notes }
}

// ---------------------------------------------------------------- web pages

/// The few entities recipe pages use, and numeric ones.
pub fn decode_entities(s: &str) -> String {
    let named = [("&amp;", "&"), ("&quot;", "\""), ("&#39;", "'"), ("&apos;", "'"), ("&lt;", "<"), ("&gt;", ">"), ("&nbsp;", " "), ("&frac12;", "½"), ("&frac14;", "¼"), ("&frac34;", "¾"), ("&deg;", "°"), ("&ndash;", "–"), ("&mdash;", "—"), ("&rsquo;", "’"), ("&lsquo;", "‘"), ("&rdquo;", "”"), ("&ldquo;", "“")];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        if let Some((e, ch)) = named.iter().find(|(e, _)| rest.starts_with(e)) {
            out.push_str(ch);
            rest = &rest[e.len()..];
            continue;
        }
        let num = rest.strip_prefix("&#").and_then(|r| r.find(';').map(|end| (&r[..end], end + 3)));
        if let Some((n, len)) = num {
            let code = n.strip_prefix(['x', 'X']).map(|h| u32::from_str_radix(h, 16)).unwrap_or_else(|| n.parse());
            if let Some(c) = code.ok().and_then(char::from_u32) {
                out.push(c);
                rest = &rest[len..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

/// Tags removed, entities decoded, whitespace tidied.
fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The contents of every `<script type="application/ld+json">` on a page.
fn json_ld_blocks(html: &str) -> Vec<Value> {
    // ASCII lowercase keeps byte offsets lined up with `html`.
    let lower = html.to_ascii_lowercase();
    let mut out = vec![];
    let mut from = 0;
    while let Some(p) = lower[from..].find("<script") {
        let start = from + p;
        let Some(open_end) = lower[start..].find('>').map(|e| start + e + 1) else { break };
        let Some(close) = lower[open_end..].find("</script>").map(|e| open_end + e) else { break };
        if lower[start..open_end].contains("application/ld+json")
            && let Ok(v) = serde_json::from_str::<Value>(html[open_end..close].trim())
        {
            out.push(v);
        }
        from = close + 9;
    }
    out
}

fn is_recipe(v: &Value) -> bool {
    match v.get("@type") {
        Some(Value::String(t)) => t == "Recipe",
        Some(Value::Array(ts)) => ts.iter().any(|t| t == "Recipe"),
        _ => false,
    }
}

/// The first schema.org Recipe object, wherever it's nested (`@graph`, arrays).
fn find_recipe(v: &Value) -> Option<&Value> {
    if is_recipe(v) {
        return Some(v);
    }
    match v {
        Value::Array(a) => a.iter().find_map(find_recipe),
        Value::Object(o) => o.get("@graph").and_then(find_recipe).or_else(|| o.get("mainEntity").and_then(find_recipe)),
        _ => None,
    }
}

fn text_of(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(plain(s)),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => a.iter().find_map(|x| text_of(Some(x))),
        Value::Object(o) => text_of(o.get("text").or_else(|| o.get("name"))),
        _ => None,
    }
}

/// Steps from `recipeInstructions`: a string, strings, HowToSteps, or
/// HowToSections of steps.
fn steps_of(v: Option<&Value>, out: &mut Vec<String>) {
    match v {
        Some(Value::String(s)) => out.extend(s.split(['\n', '\r']).map(plain).filter(|l| !l.is_empty())),
        Some(Value::Array(a)) => a.iter().for_each(|x| steps_of(Some(x), out)),
        Some(Value::Object(o)) => {
            if let Some(items) = o.get("itemListElement") {
                steps_of(Some(items), out);
            } else if let Some(t) = text_of(o.get("text").or_else(|| o.get("name"))) {
                out.push(t);
            }
        }
        _ => {}
    }
}

/// ISO 8601 durations: "PT1H30M" → 90.
pub fn iso_minutes(s: &str) -> Option<u32> {
    let t = s.trim().strip_prefix("P")?;
    let (days, time) = match t.split_once('T') {
        Some((d, t)) => (d, t),
        None => (t, ""),
    };
    let mut mins = 0.0;
    let mut take = |part: &str, units: &[(char, f64)]| -> Option<()> {
        let mut num = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
            } else {
                let f = units.iter().find(|(u, _)| *u == c)?.1;
                mins += num.parse::<f64>().ok()? * f;
                num.clear();
            }
        }
        Some(())
    };
    take(days, &[('D', 1440.0), ('W', 10080.0)])?;
    take(time, &[('H', 60.0), ('M', 1.0), ('S', 1.0 / 60.0)])?;
    Some(mins.round() as u32)
}

/// A recipe from a web page: its schema.org data, else its text.
pub fn from_html(html: &str, url: &str) -> Draft {
    let found = json_ld_blocks(html).iter().find_map(|b| find_recipe(b).cloned());
    let Some(r) = found else {
        let mut d = from_text(&page_text(html));
        d.recipe.source_url = Some(url.to_string());
        d.notes.insert(0, "This page has no recipe data; it was read as text, so check it closely.".into());
        return d;
    };
    let mut notes = vec![];
    let mut recipe = blank(&text_of(r.get("name")).unwrap_or_else(|| "Imported recipe".into()));
    recipe.source_url = Some(url.to_string());
    recipe.description = text_of(r.get("description")).unwrap_or_default().chars().take(2000).collect();
    match text_of(r.get("recipeYield")).and_then(|y| first_number(&y)) {
        Some(n) if n > 0.0 && n <= 100.0 => recipe.servings = n,
        _ => notes.push("No servings found; set to 1.".into()),
    }
    if let Some(Value::Array(ings)) = r.get("recipeIngredient").or_else(|| r.get("ingredients")) {
        recipe.ingredients = ings.iter().filter_map(|i| text_of(Some(i))).filter(|t| !t.is_empty()).map(|t| ingredient(&decimal_fractions(&t))).collect();
    }
    steps_of(r.get("recipeInstructions"), &mut recipe.steps);
    recipe.prep_min = r.get("prepTime").and_then(|v| v.as_str()).and_then(iso_minutes);
    recipe.cook_min = r.get("cookTime").and_then(|v| v.as_str()).and_then(iso_minutes);
    // The page's own nutrition per serving, when it gives calories.
    if let Some(n) = r.get("nutrition") {
        let g = |k: &str| text_of(n.get(k)).and_then(|s| first_number(&s));
        if let Some(kcal) = g("calories") {
            recipe.per_serving = Some(Macros { kcal, protein: g("proteinContent").unwrap_or(0.0), carbs: g("carbohydrateContent").unwrap_or(0.0), fat: g("fatContent").unwrap_or(0.0) });
            notes.push("Calories and macros per serving are the page's own; clear them to work them out from the ingredients.".into());
        }
    }
    let mut cats = vec![];
    for k in ["recipeCategory", "recipeCuisine"] {
        match r.get(k) {
            Some(Value::String(s)) => cats.extend(s.split(',').map(|x| x.trim().to_lowercase())),
            Some(Value::Array(a)) => cats.extend(a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_lowercase())),
            _ => {}
        }
    }
    recipe.tags = cats.into_iter().filter(|c| !c.is_empty()).take(10).collect();
    if recipe.ingredients.is_empty() {
        notes.push("No ingredient lines found; add them before saving.".into());
    }
    finish(recipe, notes)
}

/// A page's readable text: scripts and styles dropped, block tags as lines.
pub fn page_text(html: &str) -> String {
    // One pass: skip each dropped block from its opening tag to its closing
    // one, always moving forward (re-searching from the start for every block
    // made a crafted 3 MB page take far too long). ASCII lowercase keeps
    // byte offsets lined up with `html`.
    const DROP: [&str; 7] = ["script", "style", "noscript", "svg", "nav", "footer", "header"];
    let lower = html.to_ascii_lowercase();
    let mut s = String::with_capacity(html.len());
    let mut at = 0;
    while let Some(p) = lower[at..].find('<').map(|p| at + p) {
        s.push_str(&html[at..p]);
        let after = &lower[p + 1..];
        let dropped = DROP.iter().find(|t| after.starts_with(**t) && after[t.len()..].starts_with(|c: char| c == '>' || c == '/' || c.is_ascii_whitespace()));
        match dropped {
            Some(t) => {
                at = lower[p..].find(&format!("</{t}>")).map(|e| p + e + t.len() + 3).unwrap_or(html.len());
                s.push('\n');
            }
            None => {
                s.push('<');
                at = p + 1;
            }
        }
    }
    s.push_str(&html[at..]);
    let mut out = String::new();
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < s.len() {
        if bytes[i] == b'<' {
            let end = s[i..].find('>').map(|e| i + e + 1).unwrap_or(s.len());
            let tag = s[i..end].to_lowercase();
            if ["<br", "</p", "</li", "</h1", "</h2", "</h3", "</h4", "</div", "<li", "</tr"].iter().any(|t| tag.starts_with(t)) {
                out.push('\n');
            } else {
                out.push(' ');
            }
            i = end;
        } else {
            let next = s[i..].find('<').map(|e| i + e).unwrap_or(s.len());
            out.push_str(&s[i..next]);
            i = next;
        }
    }
    decode_entities(&out).lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_become_decimals() {
        assert_eq!(decimal_fractions("1 1/2 cups oats"), "1.5 cups oats");
        assert_eq!(decimal_fractions("½ cup milk"), "0.5 cup milk");
        assert_eq!(decimal_fractions("1½ tsp salt"), "1.5 tsp salt");
        assert_eq!(decimal_fractions("3/4 lb beef"), "0.75 lb beef");
        assert_eq!(decimal_fractions("and/or herbs"), "and/or herbs", "words with a slash stay");
    }

    #[test]
    fn pasted_text_with_headings() {
        let text = "Chicken Rice Bowl\nA quick lunch.\nServes 4\nPrep time: 15 mins\nCook time: 1 hr 5 min\n\nIngredients:\n- 1 lb chicken breast\n• 2 cups rice\n▢ 1/2 cup soy sauce\nsalt to taste\n\nInstructions\n1. Cook the rice.\n2) Grill the chicken until done.\nStep 3: Slice and serve over rice.\nNotes\nKeeps 3 days.";
        let d = from_text(text);
        let r = &d.recipe;
        assert_eq!(r.title, "Chicken Rice Bowl");
        assert_eq!(r.servings, 4.0);
        assert_eq!((r.prep_min, r.cook_min), (Some(15), Some(65)));
        let ings: Vec<&str> = r.ingredients.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(ings, vec!["1 lb chicken breast", "2 cups rice", "0.5 cup soy sauce", "salt to taste"]);
        assert_eq!(r.steps, vec!["Cook the rice.", "Grill the chicken until done.", "Slice and serve over rice."]);
        assert_eq!(r.description, "A quick lunch.\nKeeps 3 days.");
        assert!(d.notes.iter().any(|n| n.contains("3 ingredient(s)")), "{:?}", d.notes);
    }

    #[test]
    fn pasted_text_without_headings_is_split_by_shape() {
        let d = from_text("Protein pancakes\n2 eggs\n1 banana\n40 g oats\nMash the banana and whisk it with the eggs and the oats until smooth.\nCook in a hot pan for two minutes a side.");
        assert_eq!(d.recipe.ingredients.len(), 3);
        assert_eq!(d.recipe.steps.len(), 2);
        assert_eq!(d.recipe.servings, 1.0);
        assert!(d.notes.iter().any(|n| n.contains("servings")));
        let empty = from_text("   ");
        assert_eq!(empty.recipe.title, "Imported recipe");
        assert!(empty.notes.iter().any(|n| n.contains("No ingredient")));
    }

    #[test]
    fn schema_org_recipe_from_a_page() {
        let html = r#"<html><head><script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"WebPage","name":"x"},
          {"@type":["Recipe"],"name":"Greek Yogurt Parfait","description":"Layered &amp; quick","recipeYield":["2","2 servings"],
           "prepTime":"PT10M","cookTime":"PT0M",
           "recipeIngredient":["1 &frac12; cups Greek yogurt","200 g berries","2 tbsp honey"],
           "recipeInstructions":[{"@type":"HowToSection","name":"Assemble","itemListElement":[{"@type":"HowToStep","text":"Layer the yogurt."},{"@type":"HowToStep","text":"Top with &quot;berries&quot;."}]},"Drizzle honey."],
           "nutrition":{"@type":"NutritionInformation","calories":"310 kcal","proteinContent":"20 g","carbohydrateContent":"45 g","fatContent":"5 g"},
           "recipeCategory":"Breakfast, Snack"}]}</script></head><body>...</body></html>"#;
        let d = from_html(html, "https://example.com/parfait");
        let r = &d.recipe;
        assert_eq!(r.title, "Greek Yogurt Parfait");
        assert_eq!(r.description, "Layered & quick");
        assert_eq!(r.servings, 2.0);
        assert_eq!((r.prep_min, r.cook_min), (Some(10), Some(0)));
        assert_eq!(r.ingredients[0].text, "1.5 cups Greek yogurt");
        assert_eq!(r.steps, vec!["Layer the yogurt.", "Top with \"berries\".", "Drizzle honey."]);
        assert_eq!(r.per_serving.map(|m| (m.kcal, m.protein, m.carbs, m.fat)), Some((310.0, 20.0, 45.0, 5.0)));
        assert_eq!(r.tags, vec!["breakfast", "snack"]);
        assert_eq!(r.source_url.as_deref(), Some("https://example.com/parfait"));
    }

    #[test]
    fn a_page_without_recipe_data_is_read_as_text() {
        let html = "<html><head><style>p{}</style><script>var x = 1;</script></head><body><nav>Home</nav><h1>Simple Salad</h1><p>Serves 2</p><h2>Ingredients</h2><ul><li>1 cucumber</li><li>100 g feta</li></ul><h2>Method</h2><p>Chop and toss everything together in a bowl.</p></body></html>";
        let d = from_html(html, "https://example.com/salad");
        assert_eq!(d.recipe.title, "Simple Salad");
        assert_eq!(d.recipe.servings, 2.0);
        assert_eq!(d.recipe.ingredients.len(), 2);
        assert_eq!(d.recipe.steps.len(), 1);
        assert!(d.notes[0].contains("no recipe data"));
    }

    #[test]
    fn durations_and_entities() {
        assert_eq!(iso_minutes("PT1H30M"), Some(90));
        assert_eq!(iso_minutes("P1DT2H"), Some(1560));
        assert_eq!(iso_minutes("PT45S"), Some(1));
        assert_eq!(iso_minutes("90 minutes"), None);
        assert_eq!(decode_entities("Tom &amp; Jerry&#39;s &#x2013; &bogus; &"), "Tom & Jerry's – &bogus; &");
    }

    #[test]
    fn page_text_is_linear_and_keeps_offsets() {
        // Thousands of unclosed blocks used to restart the search each time.
        let many = "<style>".repeat(50_000) + "<p>Serves 2</p>";
        let t = std::time::Instant::now();
        let _ = page_text(&many);
        assert!(t.elapsed().as_secs() < 2, "{:?}", t.elapsed());
        // A character whose lowercase is longer doesn't shift what's cut.
        assert_eq!(page_text("<p>İ</p><script>x</script><p>Oats</p>"), "İ\nOats");
    }
}
