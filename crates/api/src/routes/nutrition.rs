//! Food journal (docs/NUTRITION.md). The client writes whole meals with their
//! own id through the offline outbox — like a habit day, the later write wins.
//! The coach reads meals on the client's Overview (audited, like intake),
//! searches foods through /foods/search (USDA FoodData Central, proxied so the
//! key stays in the Lambda, cached in FOODCACHE#) and PUTs estimates back.

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use ptcore::habits::{editable, EDITABLE_DAYS};
use ptcore::foodimport;
use ptcore::nutrition::{Basis, Candidate, FoodSnap, Item, Meal, Saved, Target, MAX_CANDIDATES, MAX_SAVED, normalize_fdc, summary, totals};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::keyed;
use crate::util::{audit, new_id};
use crate::AppState;

const HIGH: &str = "~";

/// Sort key for a meal: the date is part of the key so a week is one range
/// query; one id, one live copy (a date change replaces the old-dated item).
fn meal_sk(date: NaiveDate, id: &str) -> String {
    let d: String = date.to_string();
    format!("MEAL#{d}#{id}")
}

/// The meals dated from `since` on. A meal is found by id this way: ids
/// aren't in the sort key on their own, and scanning every meal ever logged
/// grows with the journal.
async fn meals_since(st: &AppState, pk: &str, since: NaiveDate) -> ApiResult<Vec<Meal>> {
    st.store.query_between(pk, &format!("MEAL#{since}"), &format!("MEAL#{HIGH}"), true).await
}

fn food_sk(id: &str) -> String {
    format!("FOOD#{id}")
}

#[derive(Deserialize)]
pub struct DaysQuery {
    days: Option<i64>,
}

#[derive(Deserialize)]
pub struct MealIn {
    date: NaiveDate,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    items: Vec<ItemIn>,
}

#[derive(Deserialize)]
pub struct ItemIn {
    text: String,
    #[serde(default)]
    grams: Option<f64>,
    #[serde(default)]
    food: Option<FoodSnap>,
    #[serde(default)]
    kcal: Option<f64>,
    #[serde(default)]
    protein: Option<f64>,
    #[serde(default)]
    carbs: Option<f64>,
    #[serde(default)]
    fat: Option<f64>,
    #[serde(default)]
    basis: Option<Basis>,
}

/// A body becomes a Meal; `updated_at` is always the server's clock, so a
/// replayed offline write cannot outrun a newer one by lying about time.
fn to_meal(body: MealIn, id: String) -> Meal {
    Meal {
        id,
        date: body.date,
        label: body.label,
        items: body.items.into_iter().map(|b| Item {
            text: b.text,
            grams: b.grams,
            food: b.food,
            kcal: b.kcal,
            protein: b.protein,
            carbs: b.carbs,
            fat: b.fat,
            basis: b.basis.unwrap_or(Basis::FreeText),
        }).collect(),
        updated_at: Utc::now(),
    }
}

const TARGET_SK: &str = "NUTRITION_TARGET";

/// `GET /clients/{cid}/meals?days=14`: whole meals, per-day kcal and macros,
/// the coach's targets and the last 7 days' averages against them.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Query(q): Query<DaysQuery>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "read the food journal").await;
    let pk = scope.pk();
    let today = scope.today();
    let days = q.days.unwrap_or(14).clamp(1, 90);
    let since = today - Duration::days(days - 1);
    // The week's averages need 7 days even when fewer are listed.
    let mut meals = meals_since(&st, &pk, today - Duration::days(days.max(7) - 1)).await?;
    let target: Option<Target> = st.store.get(&pk, TARGET_SK).await?;
    let week = summary(&totals(&meals), today, 7, target.as_ref());
    meals.retain(|m| m.date >= since);
    let day_totals = totals(&meals);
    let totals_map: serde_json::Map<String, Value> = day_totals.iter().map(|(d, m)| (d.to_string(), json!(m))).collect();
    Ok(Json(json!({ "today": today, "meals": meals, "totals": totals_map, "target": target, "week": week })))
}

/// `GET /clients/{cid}/nutrition/target`.
pub async fn get_target(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let target: Option<Target> = st.store.get(&scope.pk(), TARGET_SK).await?;
    Ok(Json(json!({ "target": target })))
}

/// `PUT /clients/{cid}/nutrition/target`: the coach sets daily kcal and macro
/// targets (any of them; all empty clears the targets).
pub async fn put_target(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(mut body): Json<Target>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if !scope.by_coach {
        return Err(ApiError::Forbidden("the coach sets nutrition targets"));
    }
    body.validate().map_err(bad_request)?;
    if body == Target::default() {
        st.store.delete(&scope.pk(), TARGET_SK).await?;
        return Ok(Json(json!({ "target": null })));
    }
    body.updated_at = Some(Utc::now());
    st.store.put(&scope.pk(), TARGET_SK, &body).await?;
    Ok(Json(json!({ "target": body })))
}

#[derive(Deserialize)]
pub struct ImportIn {
    csv: String,
}

/// CSV text accepted in one import.
const MAX_IMPORT_BYTES: usize = 1_500_000;

/// `POST /clients/{cid}/meals/import` `{csv}`: a Cronometer or MyFitnessPal
/// export (ptcore::foodimport) becomes meals for the last 90 days. Imported
/// meals have stable ids, so importing an overlapping export again replaces
/// them; meals typed in the app are left alone.
pub async fn import(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<ImportIn>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if body.csv.len() > MAX_IMPORT_BYTES {
        return Err(bad_request("that file is over 1.5 MB; export a shorter date range"));
    }
    let out = foodimport::import(&body.csv, scope.today()).map_err(bad_request)?;
    if out.meals.is_empty() {
        return Err(bad_request(format!("nothing to import from the last {} days ({} rows skipped)", foodimport::MAX_DAYS_BACK, out.skipped)));
    }
    let pk = scope.pk();
    let items = out.meals.iter().map(|m| keyed(&pk, &meal_sk(m.date, m.id.as_str()), m)).collect::<ApiResult<Vec<_>>>()?;
    st.store.batch_put(items).await?;
    let days: std::collections::BTreeSet<NaiveDate> = out.meals.iter().map(|m| m.date).collect();
    Ok(Json(json!({ "source": out.source, "meals": out.meals.len(), "days": days.len(), "skipped": out.skipped })))
}

/// `POST /clients/{cid}/meals`: a fresh meal, server-assigned id. Clients
/// normally use the PUT below through the outbox; this is the coach's path
/// and the e2e shortcut.
pub async fn create(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<MealIn>) -> ApiResult<Json<Meal>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if !editable(body.date, scope.today()) {
        return Err(bad_request("meals can be logged for today and the past week"));
    }
    let mut meal = to_meal(body, new_id()[..12].to_string());
    meal.validate().map_err(bad_request)?;
    st.store.put(&scope.pk(), &meal_sk(meal.date, meal.id.as_str()), &meal).await?;
    Ok(Json(meal))
}

/// `PUT /clients/{cid}/meals/{mid}`: create-or-replace one meal, idempotent
/// like a habit day. If the new copy names a different date, the old-dated
/// copy of this id is removed too, so one id never has two live copies.
pub async fn put_meal(State(st): State<AppState>, ctx: Ctx, Path((cid, mid)): Path<(String, String)>, Json(body): Json<MealIn>) -> ApiResult<Json<Meal>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let pk = scope.pk();
    if !editable(body.date, scope.today()) {
        return Err(bad_request("meals can be logged for today and the past week"));
    }
    // Only meals still in the editable week can be moved to another date.
    let all = meals_since(&st, &pk, scope.today() - Duration::days(EDITABLE_DAYS)).await?;
    if let Some(old) = all.iter().find(|m| m.id == mid && m.date != body.date) {
        st.store.delete(&pk, &meal_sk(old.date, mid.as_str())).await?;
    }
    let meal_date: NaiveDate = body.date;
    let mut meal = to_meal(body, mid.clone());
    meal.validate().map_err(bad_request)?;
    st.store.put(&pk, &meal_sk(meal_date, mid.as_str()), &meal).await?;
    Ok(Json(meal))
}

/// `DELETE /clients/{cid}/meals/{mid}`: the client's own to remove.
pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path((cid, mid)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if scope.by_coach {
        return Err(ApiError::Forbidden("only the client deletes their meals"));
    }
    // As far back as the journal is listed.
    let all = meals_since(&st, &scope.pk(), scope.today() - Duration::days(90)).await?;
    let Some(old) = all.iter().find(|m| m.id == mid) else { return Err(ApiError::NotFound("meal")); };
    st.store.delete(&scope.pk(), &meal_sk(old.date, mid.as_str())).await?;
    Ok(Json(json!({ "deleted": mid })))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SearchCache {
    query: String,
    candidates: Vec<Candidate>,
    fetched_at: DateTime<Utc>,
    ttl: i64,
}

/// `GET /foods/search?q=`: coach picks a food to attach. The USDA call runs
/// here — the browser must never hold the key — and repeats of the same words
/// are answered from FOODCACHE# (the free tier allows ~1 000 lookups an hour).
pub async fn search(State(st): State<AppState>, ctx: Ctx, Query(q): Query<SearchQuery>) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    let raw = q.q.unwrap_or_default();
    let words: Vec<String> = raw.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.is_empty() || raw.trim().chars().count() > 80 {
        return Err(bad_request("search expects a food name (up to 80 characters)"));
    }
    let query: String = words.join(" ");
    let hit: Option<SearchCache> = st.store.get(&format!("FOODCACHE#{query}"), "CACHE").await?;
    if let Some(hit) = hit {
        return Ok(Json(json!({ "query": query, "candidates": hit.candidates })));
    }
    let Some(key) = st.fdc_key.clone() else { return Err(bad_request("food search is off: no FoodData Central key is set up (docs/DEPLOY.md)")); };
    // The key goes in a header and the words are encoded as parameters, so
    // "mac & cheese" can't add parameters of its own.
    let page_size = MAX_CANDIDATES.to_string();
    let url = reqwest::Url::parse_with_params("https://api.nal.usda.gov/fdc/v1/foods/search", [("query", query.as_str()), ("pageSize", page_size.as_str())])
        .map_err(|_| bad_request("search expects a food name"))?;
    let Ok(res) = st.http.get(url).header("X-Api-Key", key).send().await else {
        tracing::warn!("food search unreachable: network");
        return Err(bad_request("FoodData Central is not answering now; try again later"));
    };
    match res.status().as_u16() {
        200..=299 => {}
        s => {
            tracing::warn!("food search refused: {s}");
            return Err(bad_request("FoodData Central refused the search; try other words"));
        }
    }
    let Ok(data) = res.json().await else { return Err(bad_request("FoodData Central returned something unexpected")); };
    let candidates: Vec<Candidate> = normalize_fdc(data);
    let now = Utc::now();
    st.store.put(&format!("FOODCACHE#{query}"), "CACHE", &SearchCache { query: query.clone(), candidates: candidates.clone(), fetched_at: now, ttl: now.timestamp() + 30 * 24 * 3600 }).await?;
    Ok(Json(json!({ "query": query, "candidates": candidates })))
}

#[derive(Deserialize)]
pub struct FoodIn {
    name: String,
    #[serde(default)]
    snap: Option<FoodSnap>,
}

/// `GET /foods`: the coach's saved staples.
pub async fn list_foods(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Saved>>> {
    let coach = ctx.require_coach()?;
    let mut foods: Vec<Saved> = st.store.query_prefix(&format!("COACH#{coach}"), "FOOD#").await?;
    foods.sort_by_key(|f| f.created_at);
    Ok(Json(foods))
}

/// `POST /foods`: save a staple, usually straight from a search candidate.
pub async fn create_food(State(st): State<AppState>, ctx: Ctx, Json(body): Json<FoodIn>) -> ApiResult<Json<Saved>> {
    let coach = ctx.require_coach()?;
    let pk = format!("COACH#{coach}");
    let existing: Vec<Saved> = st.store.query_prefix(&pk, "FOOD#").await?;
    if existing.len() >= MAX_SAVED {
        return Err(bad_request(format!("up to {MAX_SAVED} saved foods; delete one you no longer use")));
    }
    let mut f = Saved { id: new_id()[..12].to_string(), name: body.name, snap: body.snap, created_at: Utc::now() };
    f.validate().map_err(bad_request)?;
    st.store.put(&pk, &food_sk(f.id.as_str()), &f).await?;
    Ok(Json(f))
}

/// `DELETE /foods/{fid}`.
pub async fn remove_food(State(st): State<AppState>, ctx: Ctx, Path(fid): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let pk = format!("COACH#{coach}");
    let f: Option<Saved> = st.store.get(&pk, &food_sk(fid.as_str())).await?;
    let Some(x) = f else { return Err(ApiError::NotFound("food")); };
    st.store.delete(&pk, &food_sk(fid.as_str())).await?;
    Ok(Json(json!({ "deleted": x.id })))
}
