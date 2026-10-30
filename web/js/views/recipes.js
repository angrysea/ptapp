// Recipes, recipe books and meal plans (routes/recipes.rs). Coaches keep
// recipes in books shared with all or chosen clients and lay them out in
// meal plans they assign; clients browse what's shared and keep their own.
// Recipes come in typed, pasted, from a PDF (read here with pdf.js, so the
// file never leaves the device) or from a web page (read by the API).

import { api, enqueue, get, post, put } from '../api.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { $app, busy, confirmSheet, field, fmtDate, h, mount, num, select, sheet, toast, todayIso, uid } from '../ui.js';
import { itemMacros } from './meals.js';

const DRAFT = 'pt.recipeDraft';
const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const LABELS = ['Breakfast', 'Lunch', 'Dinner', 'Snack'];
const PDFJS = '../../vendor/pdfjs-6.4.299/pdf.min.mjs';
const PDF_WORKER = new URL('../../vendor/pdfjs-6.4.299/pdf.worker.min.mjs', import.meta.url).href;

/** One serving's kcal and macros (routes mirror ptcore::recipes::Recipe::serving). */
export function serving(r) {
  if (r.perServing) return r.perServing;
  const t = { kcal: 0, protein: 0, carbs: 0, fat: 0 };
  for (const it of r.ingredients ?? []) {
    const m = itemMacros(it);
    for (const k in t) t[k] += m[k] ?? 0;
  }
  const n = r.servings > 0 ? r.servings : 1;
  return { kcal: t.kcal / n, protein: t.protein / n, carbs: t.carbs / n, fat: t.fat / n };
}
const estimated = (it) => it.kcal != null || it.food != null;
export const macroLine = (m) => `${Math.round(m.kcal)} kcal · P ${Math.round(m.protein)} · C ${Math.round(m.carbs)} · F ${Math.round(m.fat)} g`;

/** Programs, Recipes and Meal plans, side by side for the coach. */
export function planningTabs(active) {
  const tabs = [['/programs', 'Programs'], ['/recipes', 'Recipes'], ['/mealplans', 'Meal plans']];
  return h('div', { class: 'subtabs' }, tabs.map(([p, l]) => h('button', { class: p === active ? 'active' : '', onclick: () => go(p) }, l)));
}

// ---------------------------------------------------------------- importing

async function pdfText(file) {
  const pdfjs = await import(PDFJS);
  pdfjs.GlobalWorkerOptions.workerSrc = PDF_WORKER;
  const doc = await pdfjs.getDocument({ data: await file.arrayBuffer() }).promise;
  const pages = [];
  for (let i = 1; i <= Math.min(doc.numPages, 20); i++) {
    const content = await (await doc.getPage(i)).getTextContent();
    // Items carry their own line breaks (hasEOL); keep the page's lines.
    pages.push(content.items.map((it) => it.str + (it.hasEOL ? '\n' : '')).join(''));
  }
  return pages.join('\n');
}

/** Paste, PDF or link → a draft recipe, opened in the editor to check. */
export function importSheet() {
  const mode = select([['text', 'Paste text'], ['pdf', 'A PDF file'], ['url', 'A web page']], 'text', { 'aria-label': 'Import from' });
  const text = h('textarea', { placeholder: 'Paste the recipe: title, servings, ingredients and steps', style: 'min-height:160px', 'aria-label': 'Recipe text' });
  const file = h('input', { type: 'file', accept: 'application/pdf,.pdf', 'aria-label': 'PDF file' });
  const url = h('input', { type: 'url', placeholder: 'https://…', 'aria-label': 'Recipe link' });
  const box = h('div');
  const show = () => mount(box, mode.value === 'text' ? text : mode.value === 'pdf' ? [file, h('p', { class: 'muted small' }, 'The PDF is read on this device; only its text is sent.')] : [url, h('p', { class: 'muted small' }, 'Most recipe sites publish the recipe in a form that reads cleanly; other pages are read as text.')]);
  mode.addEventListener('change', show);
  show();
  const sh = sheet(
    h('h2', {}, 'Import a recipe'),
    field('From', mode), box,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        let body;
        if (mode.value === 'url') {
          if (!url.value.trim()) throw new Error('Paste the recipe’s web address');
          body = { url: url.value.trim() };
        } else if (mode.value === 'pdf') {
          const f = file.files?.[0];
          if (!f) throw new Error('Choose a PDF');
          if (f.size > 20 * 1024 * 1024) throw new Error('That PDF is over 20 MB');
          const t = await pdfText(f);
          if (!t.trim()) throw new Error('That PDF has no text to read (a scanned page is a picture); type or paste the recipe instead');
          body = { text: t };
        } else {
          if (!text.value.trim()) throw new Error('Paste the recipe first');
          body = { text: text.value };
        }
        const draft = await post('/recipes/import', body);
        try { sessionStorage.setItem(DRAFT, JSON.stringify(draft)); } catch { /* too big to keep: opens empty */ }
        sh.close();
        go('/recipes/new');
      }),
    }, 'Read recipe'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

// ---------------------------------------------------------------- the editor (coach's or a client's own)

const blankRecipe = () => ({ title: '', description: '', servings: 1, ingredients: [], steps: [], tags: [], bookId: null, perServing: null });

/** `#/recipes/{id|new}`: a coach's recipe, or a client's own. */
export async function renderRecipeEditor(id) {
  const coach = session.isCoach;
  const base = coach ? '/recipes' : `/clients/${session.userId}/recipes`;
  let notes = [];
  let r;
  if (id === 'new') {
    let draft = null;
    try { draft = JSON.parse(sessionStorage.getItem(DRAFT)); sessionStorage.removeItem(DRAFT); } catch { /* none */ }
    r = { ...blankRecipe(), ...(draft?.recipe ?? {}) };
    notes = draft?.notes ?? [];
  } else if (coach) {
    r = await get(`/recipes/${id}`);
  } else {
    r = (await get(`/clients/${session.userId}/recipes`)).mine.find((x) => x.id === id);
    if (!r) return mount($app(), h('p', {}, 'Recipe not found.'));
  }
  const books = coach ? await get('/recipes/books').catch(() => []) : [];
  const title = h('input', { value: r.title, maxlength: 120, placeholder: 'e.g. Overnight oats' });
  const desc = h('textarea', { value: r.description ?? '', maxlength: 2000 });
  const servings = h('input', { type: 'number', inputmode: 'decimal', min: 0.5, step: 'any', value: r.servings ?? 1, style: 'width:90px' });
  const book = coach && select([['', 'No book'], ...books.map((b) => [b.id, b.name])], r.bookId ?? '', { 'aria-label': 'Book' });
  const prep = h('input', { type: 'number', inputmode: 'numeric', min: 0, value: r.prepMin ?? '', style: 'width:90px' });
  const cook = h('input', { type: 'number', inputmode: 'numeric', min: 0, value: r.cookMin ?? '', style: 'width:90px' });
  const steps = h('textarea', { value: (r.steps ?? []).join('\n'), placeholder: 'One step per line', style: 'min-height:120px' });
  const tags = h('input', { value: (r.tags ?? []).join(', '), placeholder: 'breakfast, high protein' });
  const source = h('input', { type: 'url', value: r.sourceUrl ?? '', placeholder: 'https://…' });
  const own = { on: h('input', { type: 'checkbox', checked: !!r.perServing, 'aria-label': 'Set per-serving numbers by hand' }) };
  const ownFields = ['kcal', 'protein', 'carbs', 'fat'].map((k) => [k, h('input', { type: 'number', inputmode: 'decimal', min: 0, step: 'any', value: r.perServing?.[k] ?? '', 'aria-label': `${k} per serving`, style: 'width:80px' })]);
  const ings = (r.ingredients ?? []).map((it) => ({ ...it }));
  const ingBox = h('div');
  const summary = h('p', { class: 'small' });

  const current = () => ({
    ...r,
    title: title.value, description: desc.value, servings: num(servings.value) ?? 1, ingredients: ings.filter((i) => i.text.trim()),
    steps: steps.value.split('\n').map((s) => s.trim()).filter(Boolean),
    tags: tags.value.split(',').map((t) => t.trim()).filter(Boolean),
    bookId: coach ? book.value || null : null,
    prepMin: num(prep.value), cookMin: num(cook.value), sourceUrl: source.value.trim() || null,
    perServing: own.on.checked ? Object.fromEntries(ownFields.map(([k, i]) => [k, num(i.value) ?? 0])) : null,
  });
  const refresh = () => {
    const c = current();
    const missing = c.ingredients.filter((i) => !estimated(i)).length;
    mount(summary, h('strong', {}, 'Per serving: '), macroLine(serving(c)),
      !c.perServing && missing > 0 && h('span', { class: 'muted' }, ` · ${missing} ingredient${missing === 1 ? '' : 's'} not counted yet`));
  };
  const drawIngs = () => {
    mount(ingBox, ings.map((it, i) => {
      const text = h('input', { value: it.text, 'aria-label': `Ingredient ${i + 1}`, oninput: (e) => { it.text = e.target.value; refresh(); } });
      const grams = h('input', { type: 'number', inputmode: 'decimal', min: 0, step: 'any', value: it.grams != null ? Math.round(it.grams) : '', placeholder: 'g', 'aria-label': `Grams of ingredient ${i + 1}`, style: 'width:70px',
        oninput: (e) => { it.grams = num(e.target.value); refresh(); drawLine(); } });
      const kcal = h('input', { type: 'number', inputmode: 'decimal', min: 0, step: 'any', value: it.kcal ?? '', placeholder: 'kcal', 'aria-label': `Calories of ingredient ${i + 1}`, style: 'width:70px',
        oninput: (e) => { it.kcal = num(e.target.value); it.basis = it.kcal != null ? 'manual' : it.basis; refresh(); drawLine(); } });
      const status = h('div', { class: 'muted small' });
      const drawLine = () => mount(status, it.food ? `${it.food.description ?? 'USDA food'} · ${Math.round(itemMacros(it).kcal)} kcal${it.grams ? '' : ' (add grams)'}` : it.kcal != null ? 'Calories set by hand' : 'Not counted yet');
      drawLine();
      return h('div', { class: 'item-card' },
        h('div', { class: 'row' }, h('div', { class: 'grow' }, text), grams, kcal,
          coach && h('button', { class: 'small', onclick: () => matchFood(it, () => { drawIngs(); refresh(); }) }, 'Match'),
          h('button', { class: 'small ghost', 'aria-label': `Remove ingredient ${i + 1}`, onclick: () => { ings.splice(i, 1); drawIngs(); refresh(); } }, '✕')),
        status);
    }), h('button', { class: 'small', onclick: () => { ings.push({ text: '', basis: 'freetext' }); drawIngs(); } }, '+ Ingredient'));
  };
  drawIngs();
  for (const el of [servings, ...ownFields.map(([, i]) => i)]) el.addEventListener('input', refresh);
  own.on.addEventListener('change', refresh);
  refresh();

  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/recipes') }, '‹ Recipes'),
    h('h1', {}, id === 'new' ? 'New recipe' : 'Edit recipe'),
    notes.length > 0 && h('div', { class: 'banner small' }, h('strong', {}, 'Check the import: '), notes.join(' ')),
    h('div', { class: 'card' },
      field('Title', title), field('Description', desc),
      h('div', { class: 'row' }, field('Servings', servings), field('Prep (min)', prep), field('Cook (min)', cook)),
      coach && field('Book', book)),
    h('div', { class: 'card' }, h('h3', {}, 'Ingredients'),
      h('p', { class: 'muted small' }, coach ? 'Add grams and Match each to a USDA food, or type its calories, and the macros per serving add up.' : 'Add grams and calories where you know them, or set the per-serving numbers below.'),
      ingBox, summary,
      h('label', { class: 'check' }, own.on, 'Set per-serving numbers by hand'),
      h('div', { class: 'row small' }, ownFields.map(([k, i]) => h('label', {}, `${k} `, i)))),
    h('div', { class: 'card' }, field('Steps', steps), field('Tags', tags), field('Source link', source)),
    h('div', { class: 'row' },
      id !== 'new' && h('button', {
        class: 'danger', onclick: async () => {
          if (!(await confirmSheet(`Delete “${r.title}”?${coach ? ' Meal plans already assigned keep their copy.' : ''}`, 'Delete'))) return;
          await api('DELETE', `${base}/${id}`);
          go('/recipes');
        },
      }, 'Delete'),
      h('button', {
        class: 'primary grow',
        onclick: (e) => busy(e.currentTarget, async () => {
          const body = current();
          if (!body.title.trim()) throw new Error('Give the recipe a title');
          if (!body.ingredients.length) throw new Error('Add at least one ingredient');
          const saved = id === 'new' ? await post(base, body) : await put(`${base}/${id}`, body);
          toast('Recipe saved');
          if (id === 'new') go(`/recipes/${saved.id}`);
        }),
      }, 'Save recipe')));
}

/** USDA search for one ingredient (coach only, like the food journal's). */
async function matchFood(it, onPick) {
  const words = it.text.replace(/[\d.,/]+/g, ' ').replace(/\b(cups?|tbsp|tsp|g|grams?|oz|lbs?|kg|ml)\b/gi, ' ').trim();
  const q = h('input', { value: words, 'aria-label': 'Search foods' });
  const list = h('div');
  const search = async () => {
    mount(list, h('p', { class: 'muted small' }, 'Searching…'));
    const r = await api('GET', `/foods/search?q=${encodeURIComponent(q.value)}`).catch((e) => ({ candidates: [], off: e.message }));
    mount(list, r.off && h('p', { class: 'small muted' }, r.off),
      (r.candidates ?? []).map((c) => h('button', {
        class: 'block small', style: 'text-align:left;margin:4px 0',
        onclick: () => {
          it.food = { fdcId: c.fdcId, description: c.name, perGrams: c.grams ?? 100, kcalPer: c.kcal ?? 0, proteinPer: c.protein ?? null, carbsPer: c.carbs ?? null, fatPer: c.fat ?? null };
          it.basis = 'searched';
          it.kcal = null;
          sh.close();
          onPick();
        },
      }, `${c.name}${c.kcal != null ? ` · ${Math.round(c.kcal)} kcal/100 g` : ''}`)));
  };
  const sh = sheet(h('h2', {}, `Match “${it.text}”`), h('div', { class: 'row' }, h('div', { class: 'grow' }, q), h('button', { class: 'small', onclick: search }, 'Search')), list,
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
  if (words) search();
}

// ---------------------------------------------------------------- coach: books and recipes

/** `#/recipes` for the coach: books with their recipes, and the unfiled ones. */
async function renderCoachRecipes() {
  const [books, recipes] = await Promise.all([get('/recipes/books'), get('/recipes')]);
  const card = (r) => h('div', { class: 'spread recipe-row', onclick: () => go(`/recipes/${r.id}`), style: 'cursor:pointer' },
    h('div', {}, h('strong', {}, r.title), h('div', { class: 'muted small' }, `${r.servings} serving${r.servings === 1 ? '' : 's'} · ${macroLine(serving(r))}`)),
    h('span', { class: 'muted' }, '›'));
  const shareText = (b) => (b.shared?.with === 'all' ? 'Shared with all clients' : b.shared?.with === 'clients' ? `Shared with ${b.shared.ids.length} client${b.shared.ids.length === 1 ? '' : 's'}` : 'Not shared');
  const unfiled = recipes.filter((r) => !r.bookId || !books.some((b) => b.id === r.bookId));
  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Recipes'),
      h('div', { class: 'row' }, h('button', { onclick: importSheet }, 'Import'), h('button', { class: 'primary', onclick: () => go('/recipes/new') }, '+ New'))),
    planningTabs('/recipes'),
    books.map((b) => h('div', { class: 'card' },
      h('div', { class: 'spread' }, h('div', {}, h('h3', {}, b.name), h('div', { class: 'muted small' }, shareText(b))),
        h('button', { class: 'small ghost', onclick: () => bookSheet(b) }, 'Edit book')),
      b.description && h('p', { class: 'small' }, b.description),
      recipes.filter((r) => r.bookId === b.id).map(card),
      !recipes.some((r) => r.bookId === b.id) && h('p', { class: 'muted small' }, 'No recipes in this book yet.'))),
    unfiled.length > 0 && h('div', { class: 'card' }, h('h3', {}, books.length ? 'Not in a book' : 'Your recipes'), unfiled.map(card)),
    recipes.length === 0 && h('p', { class: 'muted' }, 'No recipes yet. Add one, or import one from text, a PDF or a web page.'),
    h('button', { class: 'block', onclick: () => bookSheet(null) }, '+ New recipe book'));
}

/** Create or edit a book: its name and who it's shared with. */
async function bookSheet(b) {
  const roster = await get('/clients', { cache: true }).catch(() => []);
  const name = h('input', { value: b?.name ?? '', maxlength: 80 });
  const desc = h('textarea', { value: b?.description ?? '', maxlength: 500 });
  const how = select([['nobody', 'Only me'], ['all', 'All my clients'], ['clients', 'Chosen clients']], b?.shared?.with ?? 'all', { 'aria-label': 'Shared with' });
  const chosen = new Set(b?.shared?.ids ?? []);
  const picks = h('div', { hidden: how.value !== 'clients' }, roster.map((row) => {
    const c = h('input', { type: 'checkbox', checked: chosen.has(row.client.clientId), onchange: (e) => (e.target.checked ? chosen.add(row.client.clientId) : chosen.delete(row.client.clientId)) });
    return h('label', { class: 'check' }, c, row.client.name);
  }));
  how.addEventListener('change', () => { picks.hidden = how.value !== 'clients'; });
  const sh = sheet(
    h('h2', {}, b ? 'Edit recipe book' : 'New recipe book'),
    field('Name', name), field('Description', desc), field('Shared with', how), picks,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const shared = how.value === 'clients' ? { with: 'clients', ids: [...chosen] } : { with: how.value };
        const body = { name: name.value, description: desc.value, shared };
        if (b) await put(`/recipes/books/${b.id}`, body); else await post('/recipes/books', body);
        toast('Book saved');
        sh.close();
        renderCoachRecipes();
      }),
    }, 'Save book'),
    b && h('button', {
      class: 'danger block',
      onclick: async () => {
        if (!(await confirmSheet(`Delete the “${b.name}” book? Its recipes stay, not in a book.`, 'Delete'))) return;
        await api('DELETE', `/recipes/books/${b.id}`);
        sh.close();
        renderCoachRecipes();
      },
    }, 'Delete book'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

// ---------------------------------------------------------------- a recipe to read (and log)

/** A recipe to read, and for a client to log a serving of. */
export function recipeSheet(r, { servings = 1, label = 'Meal' } = {}) {
  const s = serving(r);
  const n = h('input', { type: 'number', inputmode: 'decimal', min: 0.25, step: 'any', value: servings, style: 'width:80px', 'aria-label': 'Servings eaten' });
  const sh = sheet(
    h('h2', {}, r.title),
    r.description && h('p', { class: 'small' }, r.description),
    h('p', { class: 'small' }, h('strong', {}, 'Per serving: '), macroLine(s), ` · makes ${r.servings}`,
      (r.prepMin || r.cookMin) ? ` · ${[r.prepMin && `prep ${r.prepMin} min`, r.cookMin && `cook ${r.cookMin} min`].filter(Boolean).join(', ')}` : ''),
    h('h3', {}, 'Ingredients'),
    h('ul', { class: 'small' }, (r.ingredients ?? []).map((i) => h('li', {}, i.text, i.grams ? ` (${Math.round(i.grams)} g)` : ''))),
    (r.steps ?? []).length > 0 && [h('h3', {}, 'Steps'), h('ol', { class: 'small' }, r.steps.map((x) => h('li', {}, x)))],
    r.sourceUrl && h('p', { class: 'small' }, h('a', { href: r.sourceUrl, target: '_blank', rel: 'noopener noreferrer' }, 'Original recipe')),
    !session.isCoach && h('div', { class: 'row' }, n, h('span', { class: 'small' }, 'servings'),
      h('button', { class: 'primary grow', onclick: (e) => busy(e.currentTarget, async () => { await logServing(r, num(n.value) ?? 1, label); sh.close(); }) }, 'Log to my food journal')),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'));
}

/** Writes a serving of a recipe to today's food journal (through the outbox). */
export async function logServing(r, servings, label) {
  const s = serving(r);
  const id = uid();
  const round1 = (v) => Math.round(v * servings * 10) / 10;
  await enqueue('PUT', `/clients/${session.userId}/meals/${id}`, {
    date: todayIso(), label,
    items: [{ text: `${r.title} (${servings} serving${servings === 1 ? '' : 's'})`.slice(0, 140), kcal: round1(s.kcal), protein: round1(s.protein), carbs: round1(s.carbs), fat: round1(s.fat), basis: 'manual' }],
  }, `meal:${id}`);
  toast(navigator.onLine ? 'Logged to your food journal' : 'Logged; it syncs when you’re back online');
}

// ---------------------------------------------------------------- client: recipes

async function renderClientRecipes() {
  const data = await get(`/clients/${session.userId}/recipes`, { cache: true });
  const row = (r, mine) => h('div', { class: 'spread recipe-row', style: 'cursor:pointer', onclick: () => (mine ? go(`/recipes/${r.id}`) : recipeSheet(r)) },
    h('div', {}, h('strong', {}, r.title), h('div', { class: 'muted small' }, macroLine(serving(r)), ' per serving')),
    h('span', { class: 'muted' }, mine ? 'Edit ›' : '›'));
  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/') }, '‹ Today'),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Recipes'),
      h('div', { class: 'row' }, h('button', { onclick: importSheet }, 'Import'), h('button', { class: 'primary', onclick: () => go('/recipes/new') }, '+ New'))),
    data.books.map(({ book, recipes }) => h('div', { class: 'card' }, h('h3', {}, book.name), book.description && h('p', { class: 'small' }, book.description),
      recipes.length ? recipes.map((r) => row(r, false)) : h('p', { class: 'muted small' }, 'No recipes yet.'))),
    h('div', { class: 'card' }, h('h3', {}, 'My recipes'),
      data.mine.length ? data.mine.map((r) => row(r, true)) : h('p', { class: 'muted small' }, 'Save your own recipes here, or import them from text, a PDF or a web page.')));
}

/** `#/recipes`, for the coach or a client. */
export const renderRecipes = () => (session.isCoach ? renderCoachRecipes() : renderClientRecipes());

// ---------------------------------------------------------------- coach: meal plans

/** `#/mealplans`: the coach's meal plans. */
export async function renderMealPlans() {
  const plans = await get('/mealplans');
  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Meal plans'), h('button', { class: 'primary', onclick: () => go('/mealplans/new') }, '+ New')),
    planningTabs('/mealplans'),
    plans.length === 0 && h('p', { class: 'muted' }, 'No meal plans yet. A plan lays your recipes out by day; assign it from a client’s Overview.'),
    plans.map((p) => h('div', { class: 'card link', onclick: () => go(`/mealplans/${p.id}`) },
      h('strong', {}, p.name), h('div', { class: 'muted small' }, `${p.weeks} week${p.weeks === 1 ? '' : 's'} · ${p.days.reduce((n, d) => n + d.meals.length, 0)} meals`),
      p.description && h('p', { class: 'small' }, p.description))));
}

/** `#/mealplans/{id|new}`: weeks of days, each with meals of recipes, and each day's totals. */
export async function renderMealPlanEditor(id) {
  const recipes = await get('/recipes');
  const byId = Object.fromEntries(recipes.map((r) => [r.id, r]));
  const p = id === 'new' ? { name: '', description: '', weeks: 1, days: [] } : (await get(`/mealplans/${id}`)).plan;
  const name = h('input', { value: p.name, maxlength: 80, placeholder: 'e.g. 2,000 kcal high protein' });
  const desc = h('textarea', { value: p.description ?? '', maxlength: 1000 });
  const weeks = h('input', { type: 'number', min: 1, max: 12, value: p.weeks, style: 'width:80px' });
  const grid = h('div');
  const dayOf = (w, d) => p.days.find((x) => x.week === w && x.day === d) ?? (p.days.push({ week: w, day: d, meals: [] }), p.days.at(-1));
  const total = (day) => day.meals.reduce((t, m) => {
    const r = byId[m.recipeId];
    if (!r) return t;
    const s = serving(r);
    return { kcal: t.kcal + s.kcal * m.servings, protein: t.protein + s.protein * m.servings, carbs: t.carbs + s.carbs * m.servings, fat: t.fat + s.fat * m.servings };
  }, { kcal: 0, protein: 0, carbs: 0, fat: 0 });
  const draw = () => {
    p.weeks = Math.max(1, Math.min(12, Number(weeks.value) || 1));
    mount(grid, Array.from({ length: p.weeks }, (_, wi) => h('div', { class: 'card' }, h('h3', {}, `Week ${wi + 1}`),
      DAYS.map((dn, di) => {
        const day = dayOf(wi + 1, di + 1);
        return h('div', { class: 'plan-day' },
          h('div', { class: 'spread' }, h('strong', {}, dn), day.meals.length > 0 && h('span', { class: 'muted small' }, macroLine(total(day)))),
          day.meals.map((m, mi) => h('div', { class: 'row small' },
            h('span', { class: 'grow' }, `${m.label}: ${byId[m.recipeId]?.title ?? 'a deleted recipe'} × ${m.servings}`),
            h('button', { class: 'small ghost', 'aria-label': `Remove ${m.label} on ${dn}`, onclick: () => { day.meals.splice(mi, 1); draw(); } }, '✕'))),
          h('button', { class: 'small ghost', onclick: () => addMeal(day, draw) }, `+ Meal on ${dn}`));
      }),
      h('button', { class: 'small', onclick: () => copyWeek(wi + 1) }, `Copy week ${wi + 1} to the next`))));
  };
  const addMeal = (day, done) => {
    if (!recipes.length) { toast('Add some recipes first'); return; }
    const label = select(LABELS.map((l) => [l, l]), LABELS[Math.min(day.meals.length, 3)], { 'aria-label': 'Meal' });
    const recipe = select(recipes.map((r) => [r.id, `${r.title} · ${Math.round(serving(r).kcal)} kcal`]), recipes[0].id, { 'aria-label': 'Recipe' });
    const n = h('input', { type: 'number', inputmode: 'decimal', min: 0.25, step: 'any', value: 1, 'aria-label': 'Servings', style: 'width:80px' });
    const sh = sheet(h('h2', {}, 'Add a meal'), field('Meal', label), field('Recipe', recipe), field('Servings', n),
      h('button', { class: 'primary block', onclick: () => { day.meals.push({ label: label.value, recipeId: recipe.value, servings: num(n.value) ?? 1 }); sh.close(); done(); } }, 'Add'),
      h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
  };
  const copyWeek = (w) => {
    if (w >= 12) return;
    if (w + 1 > p.weeks) weeks.value = w + 1;
    p.days = p.days.filter((d) => d.week !== w + 1).concat(p.days.filter((d) => d.week === w).map((d) => ({ ...structuredClone(d), week: w + 1 })));
    draw();
  };
  weeks.addEventListener('change', draw);
  draw();
  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/mealplans') }, '‹ Meal plans'),
    h('h1', {}, id === 'new' ? 'New meal plan' : 'Edit meal plan'),
    h('div', { class: 'card' }, field('Name', name), field('Description', desc), field('Weeks (repeats after)', weeks)),
    grid,
    h('div', { class: 'row' },
      id !== 'new' && h('button', {
        class: 'danger', onclick: async () => {
          if (!(await confirmSheet(`Delete “${p.name}”? Clients following it keep their copy.`, 'Delete'))) return;
          await api('DELETE', `/mealplans/${id}`);
          go('/mealplans');
        },
      }, 'Delete'),
      h('button', {
        class: 'primary grow',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!name.value.trim()) throw new Error('Give the plan a name');
          const body = { ...p, name: name.value, description: desc.value, days: p.days.filter((d) => d.meals.length && d.week <= p.weeks) };
          const saved = id === 'new' ? await post('/mealplans', body) : await put(`/mealplans/${id}`, body);
          toast('Meal plan saved');
          if (id === 'new') go(`/mealplans/${saved.plan.id}`);
        }),
      }, 'Save meal plan')));
}

// ---------------------------------------------------------------- a client's plan

/** For the coach's client Overview: the client's meal plan, assign or remove. */
export async function mealPlanSheet(client, onDone) {
  const [plans, now] = await Promise.all([get('/mealplans').catch(() => []), get(`/clients/${client.userId}/mealplan`).catch(() => ({ assigned: null }))]);
  const pick = select(plans.map((p) => [p.id, p.name]), now.assigned?.planId ?? plans[0]?.id ?? '', { 'aria-label': 'Meal plan' });
  const start = h('input', { type: 'date', value: todayIso() });
  const sh = sheet(
    h('h2', {}, `${client.name}’s meal plan`),
    now.assigned ? h('p', { class: 'small' }, `Following “${now.assigned.name}” since ${fmtDate(now.assigned.startDate)}.`) : h('p', { class: 'muted small' }, 'No meal plan yet.'),
    plans.length === 0 ? h('p', { class: 'muted small' }, 'Make a meal plan first, under Programs › Meal plans.')
      : [field('Plan', pick), field('Starting', start, 'Repeats after its last week. Assigning replaces the current plan.'),
        h('button', {
          class: 'primary block',
          onclick: (e) => busy(e.currentTarget, async () => {
            await put(`/clients/${client.userId}/mealplan`, { planId: pick.value, startDate: start.value });
            toast('Meal plan assigned');
            sh.close();
            onDone?.();
          }),
        }, 'Assign')],
    now.assigned && h('button', {
      class: 'danger block',
      onclick: (e) => busy(e.currentTarget, async () => { await api('DELETE', `/clients/${client.userId}/mealplan`); toast('Meal plan removed'); sh.close(); onDone?.(); }),
    }, 'Remove meal plan'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'));
}

/** For the client's Meals card on Today: today's planned meals, to open or
 * log; `onLogged` redraws the card with the new entry. */
export async function todaysPlan(me, onLogged) {
  let d;
  try { d = await get(`/clients/${me.userId}/mealplan`, { cache: true }); } catch { return null; }
  if (!d.assigned || !d.meals.length) return null;
  return h('div', { class: 'plan-today' },
    h('div', { class: 'small' }, h('strong', {}, 'Today’s plan'), h('span', { class: 'muted' }, ` · ${d.assigned.name}`)),
    d.meals.map((m) => h('div', { class: 'spread small', style: 'margin:4px 0' },
      h('button', { class: 'linklike', onclick: () => m.recipe && recipeSheet(m.recipe, { servings: m.servings, label: m.label }) }, `${m.label}: ${m.recipe?.title ?? 'recipe'} × ${m.servings}`),
      m.recipe && h('button', { class: 'small', onclick: (e) => busy(e.currentTarget, async () => { await logServing(m.recipe, m.servings, m.label); onLogged?.(); }) }, 'Log'))));
}
