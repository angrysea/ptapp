// Food journal. Clients note meals on Today: one line per item, amounts typed
// inline ("chicken 12oz") and parsed to grams server-side. Writes ride the
// offline outbox like habit days, so the card works with no network. The coach
// sees recent meals with kcal on the client's Overview — and matches unlogged
// items against food search (USDA, proxied by the API), PUTting the meal back
// online when estimating.

import { api, enqueue, get, myOutbox } from '../api.js';
import { addDays, busy, fmtDate, h, mount, num, select, sheet, toast, uid } from '../ui.js';

const mealPath = (cid, id) => `/clients/${cid}/meals/${id}`;

/** An item's kcal the way the server counts it (nutrition.rs `Item::energy`). */
const foodKcal = (it) => it.kcal ?? (it.food && it.grams ? (it.food.kcalPer ?? 0) * it.grams / (it.food.perGrams || 100) : null);

/** Server meals with this user's queued PUTs applied, so Today works offline. */
async function load(cid) {
  const data = await get(`/clients/${cid}/meals?days=14`, { cache: true });
  const byId = new Map((data.meals ?? []).map((m) => [m.id, m]));
  for (const i of await myOutbox()) {
    const mt = i.method === 'PUT' && i.path.match(/^\/clients\/([^/]+)\/meals\/([^/]+)$/);
    if (mt && mt[1] === cid) byId.set(mt[2], { ...byId.get(mt[2]) ?? {}, ...i.body, id: mt[2] });
  }
  return { ...data, meals: [...byId.values()].sort((a, b) => (a.date || '').localeCompare(b.date || '')) };
}

/** kcal for one meal, or null while none of its items has an estimate. */
function kcalOf(m) {
  const known = (m.items ?? []).map(foodKcal).filter((k) => k != null);
  return known.length ? Math.round(known.reduce((a, b) => a + b, 0)) : null;
}

const mealLine = (m) => {
  const k = kcalOf(m);
  return `${m.label || 'Meal'} · ${fmtDate(m.date)}${k != null ? ` ≈ ${k} kcal` : ' · not estimated yet'}`;
};

/** For the client's Today screen: the last meals and "+ Log a meal". Fills `slot`. */
export async function todayCard(slot, me) {
  const draw = async () => {
    let data;
    try { data = await load(me.userId); } catch { return; } // offline with nothing cached
    const recent = (data.meals ?? []).filter((m) => (m.date ?? '') >= addDays(data.today, -2)).slice(-5).reverse();
    mount(slot,
      h('div', { class: 'card' },
        h('div', { class: 'spread' }, h('h3', {}, 'Meals'), h('button', { class: 'small ghost', onclick: () => logSheet(me, data.today, draw) }, '+ Log a meal')),
        recent.length ? recent.map((m) => h('div', { class: 'small muted' }, mealLine(m))) : h('p', { class: 'muted small' }, 'Nothing logged in the last days.')));
  };
  await draw();
}

/** One meal, typed as item lines; queued through the outbox like a habit day. */
async function logSheet(me, today, onDone) {
  const label = select([['Breakfast', 'Breakfast'], ['Lunch', 'Lunch'], ['Dinner', 'Dinner'], ['Snack', 'Snack']], 'Lunch');
  const lines = h('textarea', { maxlength: 800, placeholder: 'chicken 12oz\nrice\nbroccoli 200g', 'aria-label': 'One item per line', style: 'min-height:70px' });
  let saved = false;
  const sh = sheet(
    h('h2', {}, 'Log a meal'),
    label, lines,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const items = lines.value.split('\n').map((s) => s.trim()).filter(Boolean).map((text) => ({ text }));
        if (!items.length) { toast('Write at least one item line', 'error'); return; }
        const id = uid();
        await enqueue('PUT', mealPath(me.userId, id), { date: today, label: label.value, items }, `meal:${id}`);
        saved = true;
        toast(navigator.onLine ? 'Meal saved' : 'Meal saved. It syncs when you’re back online.');
        sh.close();
      }),
    }, 'Save meal'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
  // Redraw with the new meal (load() reads it from the outbox).
  sh.onClose = () => { if (saved) onDone?.(); };
}

/** For the coach's Overview tab: per-day dots and the last meals. */
export async function overviewCard(client) {
  const card = h('div', { class: 'card' });
  const draw = async () => {
    const data = await load(client.userId);
    const days = Array.from({ length: 14 }, (_, i) => addDays(data.today, i - 13));
    mount(card,
      h('div', { class: 'spread' }, h('h3', {}, 'Food journal'),
        h('button', { class: 'small', onclick: () => estimate(client.userId, draw) }, 'Estimate')),
      (data.meals ?? []).length === 0 ? h('p', { class: 'muted small' }, 'Nothing logged yet. The client notes what they eat; you add the calories.')
        : [
          h('div', { class: 'dots', 'aria-label': 'meals logged per day, last 14 days' }, days.map((d) => {
            const k = data.totals?.[d];
            return h('span', { class: `dot ${k ? 'on' : ''}`, title: `${fmtDate(d)}: ${k ? `${Math.round(k)} kcal` : 'nothing logged'}` });
          })),
          (data.meals ?? []).slice(-4).reverse().map((m) => h('div', { class: 'small muted' }, mealLine(m))),
        ]);
  };
  draw().catch((e) => mount(card, h('p', { class: 'muted small' }, e.message)));
  return card;
}

/** The coach's estimate pass: for each item without an estimate, pick one of
 * the food-search matches (or none) and check the amount, then save. */
async function estimate(cid, onDone) {
  const data = await load(cid);
  const meals = (data.meals ?? []).filter((m) => (m.date ?? '') >= addDays(data.today, -13));
  if (!meals.length) { toast('Nothing logged to estimate'); return; }
  const pick = select(meals.map((m) => [m.id, `${m.label || 'Meal'} · ${fmtDate(m.date)}`]), meals[meals.length - 1].id);
  const rows = h('div');
  let pending = []; // per item: () => the item to save
  let saved = false;

  async function show() {
    const m = meals.find((x) => x.id === pick.value);
    mount(rows, h('p', { class: 'muted small' }, 'Looking up foods…'));
    pending = await Promise.all(m.items.map(async (it) => {
      if (it.food || it.kcal != null) {
        const k = foodKcal(it);
        return { row: h('div', { class: 'small' }, it.text, ' · ', it.kcal != null ? `${Math.round(it.kcal)} kcal (set by hand)` : `≈ ${Math.round(k ?? 0)} kcal (${it.food.description ?? 'food'})`), get: () => it };
      }
      const found = await api('GET', `/foods/search?q=${encodeURIComponent(it.text)}`).catch(() => ({ candidates: [] }));
      const cands = (found.candidates ?? []).slice(0, 5);
      const choice = select([['', 'No match'], ...cands.map((c, i) => [String(i), `${c.name}${c.kcal != null ? ` · ${Math.round(c.kcal)} kcal/100 g` : ''}`])], cands.length ? '0' : '', { 'aria-label': `Food for ${it.text}` });
      const grams = h('input', { type: 'number', inputmode: 'decimal', min: 0, step: 'any', value: it.grams != null ? Math.round(it.grams) : '', placeholder: 'grams', 'aria-label': `Grams of ${it.text}`, style: 'width:90px' });
      return {
        row: h('div', { class: 'field' }, h('div', { class: 'small' }, h('strong', {}, it.text), it.grams == null && h('span', { class: 'muted' }, ' · no amount given')),
          h('div', { class: 'row' }, h('div', { class: 'grow' }, choice), grams, h('span', { class: 'small muted' }, 'g'))),
        get: () => {
          const c = cands[Number(choice.value)];
          if (choice.value === '' || !c) return it;
          return { ...it, grams: num(grams.value) ?? it.grams ?? null, basis: 'searched',
            food: { fdcId: c.fdcId, description: c.name, perGrams: c.grams ?? 100, kcalPer: c.kcal ?? 0 } };
        },
      };
    }));
    mount(rows, pending.map((p) => p.row));
  }

  pick.addEventListener('change', show);
  const sh = sheet(
    h('h2', {}, 'Estimate a meal'),
    pick,
    rows,
    h('p', { class: 'muted small' }, 'Without an amount, a match can’t give calories: add the grams.'),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const m = meals.find((x) => x.id === pick.value);
        await api('PUT', mealPath(cid, m.id), { date: m.date, label: m.label, items: pending.map((p) => p.get()) });
        saved = true;
        toast('Meal saved');
        sh.close();
      }),
    }, 'Save'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
  sh.onClose = () => { if (saved) onDone?.(); };
  show();
}
