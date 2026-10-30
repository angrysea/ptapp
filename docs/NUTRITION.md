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
Meal      { id, date, label?, items: [Item], updated_at }
Item      { text, grams?, food?: FoodSnap, kcal?, protein?, carbs?, fat?, basis }
FoodSnap  { fdc_id, description?, per_grams, kcal_per, protein_per?, carbs_per?, fat_per? }
Target    { kcal?, protein?, carbs?, fat?, note?, updated_at? }   // CLIENT#/NUTRITION_TARGET
```

* `basis: freetext | searched | manual | imported` per item, so the journal
  says what an estimate's basis is and the coach can see which items are
  still guesses.
* `Item::macros()` works figure by figure: a number set by hand (or imported)
  wins, else the snapshot's figure scaled to the grams, else 0. `totals`
  sums kcal, protein, carbs and fat per day; `summary` averages the last 7
  days over the days that were logged and counts days within 10% of the
  kcal target.
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
POST     /clients/{cid}/meals/import     # {csv}: Cronometer / MyFitnessPal export
GET|PUT  /clients/{cid}/nutrition/target # anyone in scope reads; the coach writes
GET      /foods/search?q=                # coach-only; never the raw FDC call
GET|POST /foods                          DELETE /foods/{fid}   # coach's saved foods
```

`GET …/meals` returns `{today, meals, totals: {date: {kcal, protein, carbs,
fat}}, target, week: {days, logged, average, onTarget?}}`.

## Targets

The coach sets any of kcal (500-10 000), protein, carbs and fat (0-1 000 g)
and a short note from the Food journal card; all blank clears them. The
client sees today's totals as meters against them on Today; the coach sees
the 7-day averages against them, with how many logged days were within 10%
of the kcal target. Only the client's coach writes targets.

## Import (`crates/core/src/foodimport.rs`)

Neither Cronometer nor MyFitnessPal has an open API, so the client exports a
CSV on the web and uploads it from the Meals card (up to 1.5 MB, 5 000 rows).
Columns are found by header name, not position:

| | date | meal | food | amount | numbers |
|---|---|---|---|---|---|
| Cronometer, *Food & Recipe Entries* | `Day` | `Group` | `Food Name` | `Amount` ("150.00 g") | `Energy (kcal)`, `Protein (g)`, `Carbs (g)`, `Fat (g)` |
| MyFitnessPal, *Nutrition* | `Date` | `Meal` | — (meal totals) | — | `Calories`, `Protein (g)`, `Carbohydrates (g)`, `Fat (g)` |

Rows group into meals by day and meal name (more than 12 foods continue in
"Lunch (2)"); items are `basis: imported` with the numbers as given and
grams where the amount is a weight. Only the last 90 days are taken. Meal
ids come from day, meal and part, so importing an overlapping export again
replaces those meals instead of doubling them; meals typed in the app are
never touched. **The column names above have not yet been checked against a
real export file**; matching by header name (with `Date`/`Day`,
`Calories`/`Energy` and similar alternatives) is meant to absorb small
differences. Check one real file from each app before relying on it.

* `/foods/search` is the only place USDA is touched: server-side fetch with
  the key from the Lambda environment (`FDC_API_KEY`, a template parameter
  defaulting to empty — empty turns search off: the answer is `{candidates: [], off: "…"}` with a 200, and the estimate sheet shows the message once). The
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
* **Client (Today):** today's kcal and macros as meters against the coach's
  targets; tapping a meal shows its lines with numbers and lets the client
  delete it (through the outbox); "Import from Cronometer or MyFitnessPal".
* **Coach (Overview):** targets, the 7-day averages against them, and in the
  estimate sheet the coach's saved staples (★) before search results, a kcal
  field to set a number by hand, and "Save this match as a staple".

## Deliberate non-decisions

* **No per-food brand matching client-side.** Brands in FDC are inconsistent;
  the coach's eye is better, and the review loop already exists.
* **No paid text-parsing APIs yet** (Edamam Nutrition Analysis, NutritionX):
  the free-text → parsed-lines step is ours for now, and their pricing/docs
  couldn't be verified in writing this doc. If clients prove bad at entering
  item lines, `/foods/search` is not the seam to swap — a parsing pass in
  `nutrition.rs` is.
* **Cup and tablespoon weights.** "1 cup rice" stays unmeasured until the
  coach adds grams; per-food household weights from FDC portions are a later
  pass.
* **Nutrition in the progress report** (BZ-6) is not added yet.

## Tests

* `nutrition.rs` unit tests: parser (oz/lb/g, "2 chicken breasts 6oz each"),
  gram math both directions, estimate scaling, `validate` bounds.
* `crates/api/tests/e2e.rs`: client posts a meal ("chicken 12oz" parses to
  ~340 g) → coach PUTs a food snapshot + a manual kcal back → the per-day
  total reads ~761 kcal; empty meals and meals outside the window are 400;
  a second coach gets 403; search is coach-only and says so when the key is
  off; saved foods validate their name; only the client deletes a meal.
* `nutrition.rs` also covers macros per figure, the 7-day summary and target
  bounds; `foodimport.rs` covers quoted CSV fields, a Cronometer file, a
  MyFitnessPal file, US dates, long meals and files it can't read.
* e2e (`nutrition_targets_and_import`): only the client's coach sets targets,
  the journal returns them with the week's summary, an empty body clears
  them; an import groups rows into meals, skips old rows, and importing again
  replaces rather than doubles.
* Browser journey: the client logs a meal and imports a CSV; the coach sets
  targets and estimates by hand; Today's meters read 760 / 2,200 kcal.

## Recipes, recipe books and meal plans

`crates/core/src/recipes.rs`, `recipeimport.rs`; `crates/api/src/routes/recipes.rs`; `web/js/views/recipes.js`.

* **Recipes** are ingredient lines (the journal's `Item`, so each can carry a USDA snapshot, grams, or numbers set by hand), steps, servings, times, tags and a source link. Per-serving calories and macros add up from the ingredients, unless set by hand (or stated by an imported page). Coaches match ingredients to USDA foods in the editor; clients, who can't search, type calories or per-serving numbers.
* **Books** (`COACH#/BOOK#{id}`) file the coach's recipes (`COACH#/RECIPE#{id}`) and are shared with nobody, all clients, or chosen clients. Deleting a book leaves its recipes unfiled. Clients keep their own recipes (`CLIENT#/RECIPE#{id}`), which their coach can see.
* **Import, by rules (no AI):** `POST /recipes/import` with `{text}` or `{url}` returns a draft to check; nothing is saved until it is. Text is split by its headings (Ingredients, Method, Notes…), or without headings by the shape of each line; fractions become decimals; servings and prep/cook times are read from their lines. A web page's schema.org Recipe (JSON-LD, which most recipe sites publish) is read field by field, including its stated nutrition; a page without it is reduced to text. **PDFs are read on the device** with pdf.js (`web/vendor/pdfjs-6.4.299/`, loaded only then) and only the text is sent.
* **Link safety:** the API fetches a link only over http(s) on ports 80/443, only from hosts that resolve to public addresses (checked, then pinned, for each of up to four redirects, so neither DNS nor a redirect can point it at the Lambda's own network or the metadata service), within 10 seconds and 3 MB, HTML only.
* **Meal plans** (`COACH#/MEALPLAN#{id}`) lay the coach's recipes out by week (1-12) and day, each meal a recipe and a number of servings, with each day's totals in the editor. Assigning one (`PUT /clients/{cid}/mealplan`) copies it to the client (`CLIENT#/MEALPLAN`) with the recipes it uses, from a start date; it repeats after its last week. The client's Meals card shows today's planned meals; "Log" writes a serving to the journal (through the outbox, so offline too), and each opens the recipe.
* **Not yet:** cup and spoon weights for ingredients (add grams to count them), a shopping list, and photos on recipes.
