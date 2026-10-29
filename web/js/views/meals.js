// Food journal. Clients note meals on Today: one line per item, amounts typed
// inline ("chicken 12oz") and parsed to grams server-side. Writes ride the
// offline outbox like habit days, so the card works with no network. The coach
// sees recent meals with kcal on the client's Overview — and matches unlogged
// items against food search (USDA, proxied by the API), PUTting the meal back
// online when estimating.

import { api, enqueue, get, myOutbox } from '../api.js';
import { addDays, busy, field, fmtDate, h, mount, select, sheet, toast, uid } from '../ui.js';

const mealPath = (cid, id) => `/clients/${cid}/meals/${id}`;

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

/** kcal for a whole meal from the server's per-day totals. */
function kcalOf(m, data) {
  const t = data.totals?.[m.date];
  return t ? Math.round(t) : null;
}

/** For the client's Today screen: the last meals and "+ Log a meal". Fills `slot`. */
export async function todayCard(slot, me) {
  let data;
  try { data = await load(me.userId); } catch { return; } // offline with nothing cached
  const recent = (data.meals ?? []).filter((m) => (m.date ?? '') >= addDays(data.today, -2)).slice(-5).reverse();
  const row = (m) => {
    const k = kcalOf(m, data);
    return h('div', { class: 'small muted' }, m.label || 'Meal', ' · ', fmtDate(m.date), k ? ` ≈ ${k} kcal` : ' · not logged yet');
  };
  const draw = () => mount(slot,
    h('div', { class: 'card' },
      h('div', { class: 'spread' }, h('h3', {}, 'Meals'), h('button', { class: 'small ghost', onclick: () => logSheet(me, data.today, draw) }, '+ Log a meal')),
      recent.length ? recent.map(row) : h('p', { class: 'muted small' }, 'Nothing logged in the last days.')));
  draw();
}

/** One meal, typed as item lines; queued through the outbox like a habit day. */
async function logSheet(me, today, onDone) {
  const label = select([['Breakfast', 'Breakfast'], ['Lunch', 'Lunch'], ['Dinner', 'Dinner'], ['Snack', 'Snack']], 'Lunch');
  const lines = h('textarea', { maxlength: 800, placeholder: 'chicken 12oz\nrice\nbroccoli 200g', 'aria-label': 'One item per line', style: 'min-height:70px' });
  const sh = sheet(
    h('h2', {}, 'Log a meal'),
    label, lines,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const items = lines.value.split('\n').map((s) => s.trim()).filter(Boolean).map((text) => ({ text }));
        if (!items.length) { toast('Write at least one item line', 'error'); return; }
        const id = uid();
        enqueue('PUT', mealPath(me.userId, id), { date: today, label: label.value, items }, `meal:${id}`);
        toast('Meal saved — it syncs when you are back online');
        sh.close();
      }),
    }, 'Save meal'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Done'),
  );
  sh.onClose = () => onDone?.();
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
          h('div', { class: 'small muted' }, (data.meals ?? []).slice(-4).reverse().map((m) => {
            const k = kcalOf(m, data);
            return `${m.label || 'Meal'} · ${fmtDate(m.date)} · ${k ? `≈ ${k} kcal` : 'not logged yet'}\n`;
          })),
        ]);
  };
  draw().catch((e) => mount(card, h('p', { class: 'muted small' }, e.message)));
  return card;
}

/** The coach's estimate pass: match each unestimated item to the top food-
    * search hit and save; the coach can re-run to overwrite or set numbers. */
async function estimate(cid, onDone) {
  const data = await load(cid);
  const meals = (data.meals ?? []).filter((m) => (m.date ?? '') >= addDays(data.today, -13));
  if (!meals.length) { toast('Nothing logged to estimate'); return; }
  const pick = select(meals.map((m) => [m.id, `${m.label || 'Meal'} · ${fmtDate(m.date)}`]), meals[meals.length - 1].id);
  const rows = h('div');
  const show = (items) => mount(rows, items.map((it) => {
    const k = foodKcal(it);
    const from = it.kcal != null ? `${Math.round(it.kcal)} kcal (coach set)` : it.food ? `≈ ${Math.round(k ?? 0)} kcal (${it.food.description ?? 'food'})` : 'not logged yet';
    return h('div', { class: 'small' }, it.text, it.grams ? ` · ${Math.round(it.grams)} g` : '', ' · ', from);
  }));
  const sh = sheet(
    h('h2', {}, 'Estimate a meal'),
    pick,
    show((meals.find((m) => m.id === pick.value) ?? { items: [] }).items),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const m = meals.find((x) => x.id === pick.value);
        const items = [];
        const notes = [];
        for (const it of m.items) {
          if (it.food || it.kcal != null) { items.push(it); continue; }
          const found = await api('GET', `/foods/search?q=${encodeURIComponent(it.text)}`);
          const top = found.candidates?.[0];
          if (!top) { notes.push(`no match for ${it.text}`); items.push(it); continue; }
          items.push({ ...it, food: { fdcId: top.fdcId, description: top.name, perGrams: top.grams ?? 100, kcalPer: top.kcal ?? 0 }, basis: 'searched' });
          notes.push(`${it.text}: ${top.name}`);
        }
        await api('PUT', mealPath(cid, m.id), { date: m.date, label: m.label, items });
        toast(notes.length ? `Matched: ${notes.join(', ')} — check one looks right` : 'Meal saved');
        onDone?.();
      }),
    }, 'Match foods and save'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Done'),
  );
  sh.onClose = () => onDone?.();
}
