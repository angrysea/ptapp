# Nutrition: the food journal

Calorie tracking as a journal: a client logs what they ate, and the coach — who
already reviews everything else — supplies or corrects the estimate. Lookup and
estimation come from USDA FoodData Central, called only through the API.

## Granularity: meal → item lines with amounts

A logged meal is **not** one free-text blob and **not** a full recipe
decomposition; it is a handful of "food + amount" lines:

```
Lunch · 2026-10-29 12:30
  chicken        12 oz
  rice            4 oz
  broccoli        2 oz
```

* **Amounts are where the calories are.** 12 oz vs 8 oz of chicken is ~350 kcal
  apart for the same words. A journal that can't attach weight per item throws
  away most of the accuracy that is actually achievable.
* **Full ingredient decomposition is wasted precision.** No source estimates
  "chicken and broccoli with rice" meaningfully better than the sum of its
  three simple items. Where cooking method matters (breaded vs grilled, butter
  on the rice) it is a free-text note on the line, not structured fields.
* **Item level is where coach review works.** The client writes loose lines;
  the coach confirms one estimate per item — fast, and specific enough to fix.

The client experience stays one field: they type "2 chicken breasts, rice,
broccoli" and the client parses quantities out into item lines
(`qty`/`unit` optional). The coach reviews at item level.

## Data model (`crates/core/src/nutrition.rs`, pure like the rest of `ptcore`)

```rust
Meal      { id, date, time?, label?, items: [Item], updated_at }
Item      { text, qty?, unit?, food?: FdcFood, kcal?, protein?, source }
FdcFood   { fdc_id, description, per: grams, kcal_per, source? }   // snapshot
```

* `source: freeText | searched | manual` per item, so the journal says what an
  estimate's basis is and the coach can see which items are still guesses.
* The `FdcFood` snapshot (kcal per N grams at resolve time) makes the meal
  immutable against later API changes and makes "their usual lunch" free: the
  client re-logs the same item and the stored `fdc_id` is reused without a new
  lookup.
* Pure functions, unit tested like `habits.rs`: `parse_item` (leading/inline
  quantity + unit), `to_grams` (oz, g, lb, cup→grams per food later),
  `estimate` (kcal = `kcal_per × qty_g / per`), `validate` (trim, bounds:
  0 < kcal ≤ 20_000/day-line, ≤ 12 items/meal), `editable` reusing the
  habits window rule (`EDITABLE_DAYS = 7`).

## Storage (single-table layout, same shapes as everything else)

| pk | sk | item |
|---|---|---|
| `CLIENT#{client}` | `MEAL#{date}#{id}` | whole meal document; the date is in the key so a week is one range query, and a date change replaces the old-dated copy; `updatedAt` is stamped by the server on every write |
| `COACH#{coach}` | `FOOD#{id}` | coach's saved food ("their usual lunch"), like `EXERCISE#` custom exercises |
| `FOODCACHE#{lowercase query}` | `CACHE` | FDC search results (top `MAX_CANDIDATES`), `ttl` 30 days |

Week queries are one `Query` on `CLIENT#{id}` between `MEAL#{from}` and
`MEAL#{to}#~` — the same pattern as schedule/logs reads.

## API surface

```
GET|POST /clients/{cid}/meals            PUT|DELETE /clients/{cid}/meals/{mid}
GET      /foods/search?q=                # coach-only; never the raw FDC call
GET|POST /foods                          DELETE /foods/{fid}   # coach's saved foods
```

* `/foods/search` is the only place USDA is touched: server-side fetch with
  the key from the Lambda environment (`FDC_API_KEY`, a template parameter
  defaulting to empty — empty turns search off with a clear message). The
  browser never sees it — FoodData Central deactivates any key found in
  published code.
* Coach flow mirrors what they already do with programs: read the client's
  meal, `GET /foods/search?q=chicken breast`, pick a candidate (the sheet
  starts from the top hit and says so), `PUT` the meal back with the food
  snapshot on the item. A free-tier concern, not a correctness one: the
  coach's pick is cached under `FOODCACHE#` so the next identical query
  costs no FDC call (~1 000 req/hr/IP is ample at this scale; expect one
  call per new unique item line).
* Authorization as usual: `auth::client_scope` for the journal,
  `require_coach()` for edits/estimates; reading a client's meals extends the
  existing `AUDIT#` coach-access log like intake and metrics do.

## Offline: unchanged

Meals ride the existing write outbox like habit days: the browser generates
the meal id (`uid()`), and the queued `PUT /clients/{me}/meals/{id}` is an
upsert; the server stamps `updatedAt` on every write, so a replay can never
outrun a newer copy by lying about time. The coach's estimate PUT runs online
(like the rest of the review loop). One rule from habits is reused: meals can
be written for today and the past week. `web/js/views/meals.js` fills a card
on Today (client) and on the Overview (coach); `todayCard` also opens the
estimate sheet from the coach's card.

## UI

* **Client (Today):** a "meals" card beside habits — breakfast/lunch/dinner/
  snack quick-add, one text field, the parser splitting quantities into item
  lines; unestimated items show a quiet "not logged yet" marker.
* **Coach (client page):** same view plus estimates per item, a daily total,
  and a review prompt for guess items — the intake-questions loop, reused.
* Dashboard weekly totals come later; `stats.rs` style aggregation once there
  are enough logs to make it worth tuning.

## Deliberate non-decisions

* **No per-food brand matching client-side.** Brands in FDC are inconsistent;
  the coach's eye is better, and the review loop already exists.
* **No paid text-parsing APIs yet** (Edamam Nutrition Analysis, NutritionX):
  the free-text → parsed-lines step is ours for now, and their pricing/docs
  couldn't be verified in writing this doc. If clients prove bad at entering
  item lines, `/foods/search` is not the seam to swap — a parsing pass in
  `nutrition.rs` is.
* **Macros beyond kcal** (protein at least): stored from the start in
  `FdcFood`/`Item` because FDC returns them free, but nothing displays them
  until the BRD asks.

## Tests

* `nutrition.rs` unit tests: parser (oz/lb/g, "2 chicken breasts 6oz each"),
  gram math both directions, estimate scaling, `validate` bounds.
* `crates/api/tests/e2e.rs`: client posts a meal ("chicken 12oz" parses to
  ~340 g) → coach PUTs a food snapshot + a manual kcal back → the per-day
  total reads ~761 kcal; empty meals and meals outside the window are 400;
  a second coach gets 403; search is coach-only and says so when the key is
  off; saved foods validate their name; only the client deletes a meal.
