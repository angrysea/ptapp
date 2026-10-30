//! Recipes, recipe books and meal plans (ptcore::recipes). A coach's
//! recipes and books live in their partition; books are shared with all or
//! chosen clients. A client's own recipes live in theirs. Importing reads
//! pasted text or a web page (ptcore::recipeimport); the page is fetched
//! here, only from public addresses (`fetch_public`). A meal plan assigned
//! to a client is copied to them with the recipes it uses.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{NaiveDate, Utc};
use ptcore::recipeimport::{from_html, from_text, Draft};
use ptcore::recipes::{AssignedPlan, Book, MealPlan, Recipe};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::new_id;
use crate::AppState;

const MAX_RECIPES: usize = 1000;
const MAX_BOOKS: usize = 100;
const MAX_PLANS: usize = 100;
/// Largest page read when importing from a link.
const MAX_PAGE_BYTES: usize = 3 * 1024 * 1024;

fn coach_pk(coach: &str) -> String {
    format!("COACH#{coach}")
}

fn recipe_sk(id: &str) -> String {
    format!("RECIPE#{id}")
}

fn short_id() -> String {
    new_id()[..12].to_string()
}

fn draft_json(d: Draft) -> Value {
    json!({ "recipe": d.recipe, "notes": d.notes })
}

// ---------------------------------------------------------------- books (coach)

/// `GET /recipes/books`: the coach's books.
pub async fn list_books(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Book>>> {
    let coach = ctx.require_coach()?;
    let mut books: Vec<Book> = st.store.query_prefix(&coach_pk(coach), "BOOK#").await?;
    books.sort_by_key(|b| b.name.to_lowercase());
    Ok(Json(books))
}

/// `POST /recipes/books`.
pub async fn create_book(State(st): State<AppState>, ctx: Ctx, Json(mut b): Json<Book>) -> ApiResult<Json<Book>> {
    let coach = ctx.require_coach()?;
    let existing: Vec<Book> = st.store.query_prefix(&coach_pk(coach), "BOOK#").await?;
    if existing.len() >= MAX_BOOKS {
        return Err(bad_request(format!("up to {MAX_BOOKS} recipe books")));
    }
    b.id = short_id();
    b.validate().map_err(bad_request)?;
    b.updated_at = Some(Utc::now());
    st.store.put(&coach_pk(coach), &format!("BOOK#{}", b.id), &b).await?;
    Ok(Json(b))
}

/// `PUT /recipes/books/{bid}`: rename, describe, or change who it's shared with.
pub async fn update_book(State(st): State<AppState>, ctx: Ctx, Path(bid): Path<String>, Json(mut b): Json<Book>) -> ApiResult<Json<Book>> {
    let coach = ctx.require_coach()?;
    st.store.get::<Book>(&coach_pk(coach), &format!("BOOK#{bid}")).await?.ok_or(ApiError::NotFound("recipe book"))?;
    b.id = bid;
    b.validate().map_err(bad_request)?;
    b.updated_at = Some(Utc::now());
    st.store.put(&coach_pk(coach), &format!("BOOK#{}", b.id), &b).await?;
    Ok(Json(b))
}

/// `DELETE /recipes/books/{bid}`: the book goes; its recipes stay, unfiled.
pub async fn delete_book(State(st): State<AppState>, ctx: Ctx, Path(bid): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let pk = coach_pk(coach);
    let recipes: Vec<Recipe> = st.store.query_prefix(&pk, "RECIPE#").await?;
    for mut r in recipes.into_iter().filter(|r| r.book_id.as_deref() == Some(bid.as_str())) {
        r.book_id = None;
        st.store.put(&pk, &recipe_sk(&r.id), &r).await?;
    }
    st.store.delete(&pk, &format!("BOOK#{bid}")).await?;
    Ok(Json(json!({ "deleted": bid })))
}

// ---------------------------------------------------------------- recipes (coach)

/// `GET /recipes`: all the coach's recipes, by title.
pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Recipe>>> {
    let coach = ctx.require_coach()?;
    let mut rs: Vec<Recipe> = st.store.query_prefix(&coach_pk(coach), "RECIPE#").await?;
    rs.sort_by_key(|r| r.title.to_lowercase());
    Ok(Json(rs))
}

/// `GET /recipes/{rid}`.
pub async fn get_one(State(st): State<AppState>, ctx: Ctx, Path(rid): Path<String>) -> ApiResult<Json<Recipe>> {
    let coach = ctx.require_coach()?;
    st.store.get(&coach_pk(coach), &recipe_sk(&rid)).await?.map(Json).ok_or(ApiError::NotFound("recipe"))
}

async fn check_book(st: &AppState, coach: &str, r: &Recipe) -> ApiResult<()> {
    if let Some(b) = &r.book_id
        && st.store.get::<Book>(&coach_pk(coach), &format!("BOOK#{b}")).await?.is_none()
    {
        return Err(bad_request("that recipe book doesn't exist"));
    }
    Ok(())
}

/// `POST /recipes`.
pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(mut r): Json<Recipe>) -> ApiResult<Json<Recipe>> {
    let coach = ctx.require_coach()?;
    let existing: Vec<Recipe> = st.store.query_prefix(&coach_pk(coach), "RECIPE#").await?;
    if existing.len() >= MAX_RECIPES {
        return Err(bad_request(format!("up to {MAX_RECIPES} recipes")));
    }
    r.id = short_id();
    r.validate().map_err(bad_request)?;
    check_book(&st, coach, &r).await?;
    r.updated_at = Some(Utc::now());
    st.store.put(&coach_pk(coach), &recipe_sk(&r.id), &r).await?;
    Ok(Json(r))
}

/// `PUT /recipes/{rid}`.
pub async fn update(State(st): State<AppState>, ctx: Ctx, Path(rid): Path<String>, Json(mut r): Json<Recipe>) -> ApiResult<Json<Recipe>> {
    let coach = ctx.require_coach()?;
    st.store.get::<Recipe>(&coach_pk(coach), &recipe_sk(&rid)).await?.ok_or(ApiError::NotFound("recipe"))?;
    r.id = rid;
    r.validate().map_err(bad_request)?;
    check_book(&st, coach, &r).await?;
    r.updated_at = Some(Utc::now());
    st.store.put(&coach_pk(coach), &recipe_sk(&r.id), &r).await?;
    Ok(Json(r))
}

/// `DELETE /recipes/{rid}`: plans already assigned keep their copy.
pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(rid): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    st.store.delete(&coach_pk(coach), &recipe_sk(&rid)).await?;
    Ok(Json(json!({ "deleted": rid })))
}

// ---------------------------------------------------------------- a client's recipes

/// `GET /clients/{cid}/recipes`: the coach's books shared with the client
/// (with their recipes) and the client's own recipes. The coach sees the
/// same, as their client does.
pub async fn for_client(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let cpk = coach_pk(&scope.client.coach_id);
    let books: Vec<Book> = st.store.query_prefix(&cpk, "BOOK#").await?;
    let shared: Vec<Book> = books.into_iter().filter(|b| b.shared.includes(&scope.client.user_id)).collect();
    let coach_recipes: Vec<Recipe> = if shared.is_empty() { vec![] } else { st.store.query_prefix(&cpk, "RECIPE#").await? };
    let books: Vec<Value> = shared
        .iter()
        .map(|b| {
            let mut rs: Vec<&Recipe> = coach_recipes.iter().filter(|r| r.book_id.as_deref() == Some(b.id.as_str())).collect();
            rs.sort_by_key(|r| r.title.to_lowercase());
            json!({ "book": b, "recipes": rs })
        })
        .collect();
    let mut mine: Vec<Recipe> = st.store.query_prefix(&scope.pk(), "RECIPE#").await?;
    mine.sort_by_key(|r| r.title.to_lowercase());
    Ok(Json(json!({ "books": books, "mine": mine })))
}

/// `POST /clients/{cid}/recipes`: a recipe of the client's own (the client,
/// or their coach for them).
pub async fn create_own(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(mut r): Json<Recipe>) -> ApiResult<Json<Recipe>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let existing: Vec<Recipe> = st.store.query_prefix(&scope.pk(), "RECIPE#").await?;
    if existing.len() >= MAX_RECIPES {
        return Err(bad_request(format!("up to {MAX_RECIPES} recipes")));
    }
    r.id = short_id();
    r.book_id = None;
    r.validate().map_err(bad_request)?;
    r.updated_at = Some(Utc::now());
    st.store.put(&scope.pk(), &recipe_sk(&r.id), &r).await?;
    Ok(Json(r))
}

/// `PUT /clients/{cid}/recipes/{rid}`.
pub async fn update_own(State(st): State<AppState>, ctx: Ctx, Path((cid, rid)): Path<(String, String)>, Json(mut r): Json<Recipe>) -> ApiResult<Json<Recipe>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    st.store.get::<Recipe>(&scope.pk(), &recipe_sk(&rid)).await?.ok_or(ApiError::NotFound("recipe"))?;
    r.id = rid;
    r.book_id = None;
    r.validate().map_err(bad_request)?;
    r.updated_at = Some(Utc::now());
    st.store.put(&scope.pk(), &recipe_sk(&r.id), &r).await?;
    Ok(Json(r))
}

/// `DELETE /clients/{cid}/recipes/{rid}`.
pub async fn remove_own(State(st): State<AppState>, ctx: Ctx, Path((cid, rid)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    st.store.delete(&scope.pk(), &recipe_sk(&rid)).await?;
    Ok(Json(json!({ "deleted": rid })))
}

// ---------------------------------------------------------------- import

#[derive(Deserialize)]
pub struct ImportIn {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// `POST /recipes/import` `{text}` or `{url}`: a draft recipe to check and
/// save (nothing is stored). PDFs are read to text on the device first.
pub async fn import(_ctx: Ctx, Json(body): Json<ImportIn>) -> ApiResult<Json<Value>> {
    match (body.text, body.url) {
        (Some(t), None) => {
            if t.trim().is_empty() || t.len() > 200_000 {
                return Err(bad_request("paste the recipe's text (up to 200,000 characters)"));
            }
            Ok(Json(draft_json(from_text(&t))))
        }
        (None, Some(u)) => {
            let (final_url, html) = fetch_public(u.trim()).await?;
            Ok(Json(draft_json(from_html(&html, &final_url))))
        }
        _ => Err(bad_request("send the recipe's text or a link to it")),
    }
}

/// Whether an address is on the public internet: not loopback, private,
/// link-local (cloud metadata lives there), shared, multicast or reserved.
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, ..] = v.octets();
            !(v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || v.is_multicast() || v.is_documentation()
                || a == 0 || (a == 100 && (64..128).contains(&b)) || (a == 192 && b == 0) || (a == 198 && (18..20).contains(&b)) || a >= 240)
        }
        // IPv6: only global unicast (2000::/3), and none of the ranges that
        // carry an IPv4 address inside (6to4 2002::/16, Teredo 2001::/32),
        // which could smuggle a private one past the IPv4 check; NAT64,
        // IPv4-compatible and IPv4-mapped addresses fall outside 2000::/3.
        IpAddr::V6(v) => {
            let [a, b, ..] = v.segments();
            (a & 0xe000) == 0x2000 && a != 0x2002 && !(a == 0x2001 && (b == 0 || b == 0x0db8))
        }
    }
}

/// Fetches a web page for import. Only http(s) on the standard ports, only
/// hosts that resolve to public addresses (checked, then pinned, for each of
/// up to four redirects, so a redirect or a DNS answer can't point the
/// server at itself or its network), 10 seconds, 3 MB, HTML only.
async fn fetch_public(raw: &str) -> ApiResult<(String, String)> {
    let mut url = reqwest::Url::parse(raw).map_err(|_| bad_request("that isn't a web address"))?;
    for _ in 0..5 {
        if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
            return Err(bad_request("only http and https links can be imported"));
        }
        let Some(host) = url.host_str().map(str::to_string) else { return Err(bad_request("that link has no host")) };
        let port = url.port_or_known_default().unwrap_or(443);
        if !matches!(port, 80 | 443) {
            return Err(bad_request("only links on the standard web ports can be imported"));
        }
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|_| bad_request("couldn't find that site"))?.collect();
        if addrs.is_empty() || !addrs.iter().all(|a| public_ip(a.ip())) {
            return Err(bad_request("that link doesn't point to a public website"));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            // A proxy would resolve the host itself, past the pinned address.
            .no_proxy()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (compatible; PTCoachRecipeImport/1.0)")
            .resolve(&host, addrs[0])
            .build()
            .map_err(|e| ApiError::Internal(format!("http client: {e}")))?;
        let mut res = client.get(url.clone()).header("Accept", "text/html,application/xhtml+xml").send().await.map_err(|_| bad_request("that site didn't answer; try again, or paste the recipe's text"))?;
        if res.status().is_redirection() {
            let loc = res.headers().get("location").and_then(|v| v.to_str().ok()).ok_or_else(|| bad_request("that site redirected nowhere"))?;
            url = url.join(loc).map_err(|_| bad_request("that site redirected to a bad address"))?;
            continue;
        }
        if !res.status().is_success() {
            return Err(bad_request(format!("that site answered {}; paste the recipe's text instead", res.status().as_u16())));
        }
        let html_type = res.headers().get("content-type").and_then(|v| v.to_str().ok()).is_none_or(|t| t.contains("html") || t.contains("text/plain"));
        if !html_type {
            return Err(bad_request("that link isn't a web page; for a PDF, use the PDF import"));
        }
        let mut body: Vec<u8> = vec![];
        while let Some(chunk) = res.chunk().await.map_err(|_| bad_request("the page stopped loading; try again"))? {
            if body.len() + chunk.len() > MAX_PAGE_BYTES {
                return Err(bad_request("that page is too large to import"));
            }
            body.extend_from_slice(&chunk);
        }
        return Ok((url.to_string(), String::from_utf8_lossy(&body).into_owned()));
    }
    Err(bad_request("that site redirected too many times"))
}

// ---------------------------------------------------------------- meal plans (coach)

fn plan_sk(id: &str) -> String {
    format!("MEALPLAN#{id}")
}

/// The coach's recipes a plan uses, by id; refuses one that doesn't exist.
async fn plan_recipes(st: &AppState, coach: &str, p: &MealPlan) -> ApiResult<HashMap<String, Recipe>> {
    let all: Vec<Recipe> = st.store.query_prefix(&coach_pk(coach), "RECIPE#").await?;
    let mut out = HashMap::new();
    for id in p.recipe_ids() {
        let r = all.iter().find(|r| r.id == id).ok_or_else(|| bad_request("a meal in the plan uses a recipe that no longer exists"))?;
        out.insert(id, r.clone());
    }
    Ok(out)
}

fn plan_json(p: &MealPlan, recipes: &HashMap<String, Recipe>) -> Value {
    let totals: Vec<Value> = p.day_totals(recipes).into_iter().map(|(week, day, m)| json!({ "week": week, "day": day, "total": m })).collect();
    json!({ "plan": p, "dayTotals": totals })
}

/// `GET /mealplans`.
pub async fn list_plans(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<MealPlan>>> {
    let coach = ctx.require_coach()?;
    let mut ps: Vec<MealPlan> = st.store.query_prefix(&coach_pk(coach), "MEALPLAN#").await?;
    ps.sort_by_key(|p| p.name.to_lowercase());
    Ok(Json(ps))
}

/// `GET /mealplans/{id}`: the plan and each day's totals.
pub async fn get_plan(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let p: MealPlan = st.store.get(&coach_pk(coach), &plan_sk(&id)).await?.ok_or(ApiError::NotFound("meal plan"))?;
    let all: Vec<Recipe> = st.store.query_prefix(&coach_pk(coach), "RECIPE#").await?;
    let recipes: HashMap<String, Recipe> = all.into_iter().map(|r| (r.id.clone(), r)).collect();
    Ok(Json(plan_json(&p, &recipes)))
}

/// `POST /mealplans`.
pub async fn create_plan(State(st): State<AppState>, ctx: Ctx, Json(mut p): Json<MealPlan>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let existing: Vec<MealPlan> = st.store.query_prefix(&coach_pk(coach), "MEALPLAN#").await?;
    if existing.len() >= MAX_PLANS {
        return Err(bad_request(format!("up to {MAX_PLANS} meal plans")));
    }
    p.id = short_id();
    p.validate().map_err(bad_request)?;
    let recipes = plan_recipes(&st, coach, &p).await?;
    p.updated_at = Some(Utc::now());
    st.store.put(&coach_pk(coach), &plan_sk(&p.id), &p).await?;
    Ok(Json(plan_json(&p, &recipes)))
}

/// `PUT /mealplans/{id}`: clients already following it keep their copy until it's assigned again.
pub async fn update_plan(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(mut p): Json<MealPlan>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    st.store.get::<MealPlan>(&coach_pk(coach), &plan_sk(&id)).await?.ok_or(ApiError::NotFound("meal plan"))?;
    p.id = id;
    p.validate().map_err(bad_request)?;
    let recipes = plan_recipes(&st, coach, &p).await?;
    p.updated_at = Some(Utc::now());
    st.store.put(&coach_pk(coach), &plan_sk(&p.id), &p).await?;
    Ok(Json(plan_json(&p, &recipes)))
}

/// `DELETE /mealplans/{id}`.
pub async fn delete_plan(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    st.store.delete(&coach_pk(coach), &plan_sk(&id)).await?;
    Ok(Json(json!({ "deleted": id })))
}

// ---------------------------------------------------------------- a client's meal plan

const ASSIGNED_SK: &str = "MEALPLAN";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignPlan {
    plan_id: String,
    start_date: NaiveDate,
}

/// `PUT /clients/{cid}/mealplan`: the coach gives the client a plan from
/// `startDate` (replacing any they had), with its recipes copied in.
pub async fn assign(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<AssignPlan>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?.to_string();
    let scope = client_scope(&st, &ctx, &cid).await?;
    let plan: MealPlan = st.store.get(&coach_pk(&coach), &plan_sk(&body.plan_id)).await?.ok_or(ApiError::NotFound("meal plan"))?;
    let recipes = plan_recipes(&st, &coach, &plan).await?;
    let a = AssignedPlan { plan, recipes, start_date: body.start_date, assigned_at: Utc::now() };
    st.store.put(&scope.pk(), ASSIGNED_SK, &a).await?;
    Ok(Json(json!({ "assigned": a.plan.name, "startDate": a.start_date })))
}

/// `DELETE /clients/{cid}/mealplan`.
pub async fn unassign(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    let scope = client_scope(&st, &ctx, &cid).await?;
    st.store.delete(&scope.pk(), ASSIGNED_SK).await?;
    Ok(Json(json!({ "removed": true })))
}

#[derive(Deserialize)]
pub struct DayQuery {
    date: Option<NaiveDate>,
}

/// `GET /clients/{cid}/mealplan?date=`: the client's plan, and the meals
/// planned for the day (today by default) with their recipes and totals.
pub async fn for_day(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Query(q): Query<DayQuery>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let date = q.date.unwrap_or_else(|| scope.today());
    let Some(a) = st.store.get::<AssignedPlan>(&scope.pk(), ASSIGNED_SK).await? else {
        return Ok(Json(json!({ "assigned": null, "date": date, "meals": [] })));
    };
    let meals: Vec<Value> = a
        .meals_on(date)
        .into_iter()
        .map(|m| {
            let r = a.recipes.get(&m.recipe_id);
            json!({ "label": m.label, "servings": m.servings, "recipe": r, "perServing": r.map(|r| r.serving()) })
        })
        .collect();
    let slot = a.slot(date);
    Ok(Json(json!({
        "assigned": { "name": a.plan.name, "description": a.plan.description, "weeks": a.plan.weeks, "startDate": a.start_date, "planId": a.plan.id },
        "date": date,
        "slot": slot.map(|(w, d)| json!({ "week": w, "day": d })),
        "meals": meals,
    })))
}

#[cfg(test)]
mod tests {
    use super::public_ip;

    #[test]
    fn only_public_addresses_are_fetched() {
        for blocked in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1",
            "::ffff:127.0.0.1", "::ffff:169.254.169.254", "64:ff9b::a9fe:a9fe", "64:ff9b:1::a00:1", "2002:a9fe:a9fe::1", "2001:0:4136:e378::1", "::a9fe:a9fe", "2001:db8::1",
        ] {
            assert!(!public_ip(blocked.parse().unwrap()), "{blocked} must be refused");
        }
        for ok in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(ok.parse().unwrap()), "{ok} is public");
        }
    }
}
