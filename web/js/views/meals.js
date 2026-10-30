// Food journal. Clients note meals on Today: one line per item, amounts typed
// inline ("chicken 12oz") and parsed to grams server-side. Writes ride the
// offline outbox like habit days, so the card works with no network. The coach
// sees recent meals, the week's averages and their targets on the client's
// Overview — and matches unlogged items against saved staples and food search
// (USDA, proxied by the API), PUTting the meal back online when estimating.
// Clients can also import a Cronometer or MyFitnessPal CSV export (BZ-4).

import { api, enqueue, get, myOutbox, post, put } from '../api.js';
import { addDays, busy, field, fmtDate, h, mount, num, select, sheet, toast, uid } from '../ui.js';
import { mealPlanSheet, todaysPlan } from './recipes.js';

const mealPath = (cid, id) => `/clients/${cid}/meals/${id}`;
const MACROS = [['protein', 'Protein'], ['carbs', 'Carbs'], ['fat', 'Fat']];

/** An item's kcal and macros the way the server counts them (nutrition.rs
 * `Item::macros`): a number set by hand wins, else the food scaled to grams. */
export function itemMacros(it) {
  const f = it.food;
  const scaled = (per) => (f && it.grams && per != null ? per * it.grams / (f.perGrams || 100) : 0);
  return {
    kcal: it.kcal ?? scaled(f?.kcalPer),
    protein: it.protein ?? scaled(f?.proteinPer),
    carbs: it.carbs ?? scaled(f?.carbsPer),
    fat: it.fat ?? scaled(f?.fatPer),
  };
}
const estimated = (it) => it.kcal != null || it.food != null;

/** Totals for a list of meals. */
function sum(meals) {
  const t = { kcal: 0, protein: 0, carbs: 0, fat: 0 };
  for (const m of meals) for (const it of m.items ?? []) {
    const x = itemMacros(it);
    for (const k in t) t[k] += x[k];
  }
  return t;
}

/** A day's total from the server; older cached responses held a bare number. */
const dayKcal = (t) => (typeof t === 'number' ? t : t?.kcal);

/** Server meals with this user's queued writes applied, so Today works offline. */
async function load(cid) {
  // The outbox first: a write it no longer holds has reached the server
  // before the GET below, so a meal saved a moment ago is never in neither.
  const queued = await myOutbox();
  const data = await get(`/clients/${cid}/meals?days=14`, { cache: true });
  const byId = new Map((data.meals ?? []).map((m) => [m.id, m]));
  for (const i of queued) {
    const mt = i.path.match(/^\/clients\/([^/]+)\/meals\/([^/]+)$/);
    if (!mt || mt[1] !== cid) continue;
    if (i.method === 'PUT') byId.set(mt[2], { ...byId.get(mt[2]) ?? {}, ...i.body, id: mt[2] });
    if (i.method === 'DELETE') byId.delete(mt[2]);
  }
  return { ...data, meals: [...byId.values()].sort((a, b) => (a.date || '').localeCompare(b.date || '')) };
}

/** kcal for one meal, or null while none of its items has an estimate. */
function kcalOf(m) {
  const items = (m.items ?? []).filter(estimated);
  return items.length ? Math.round(sum([{ items }]).kcal) : null;
}

const mealLine = (m) => {
  const k = kcalOf(m);
  return `${m.label || 'Meal'} · ${fmtDate(m.date)}${k != null ? ` ≈ ${k} kcal` : ' · not estimated yet'}`;
};

/** "1,450 / 2,000 kcal" with a bar, or just the number with no target. */
function meter(label, value, target, unit) {
  const v = Math.round(value);
  if (!target) return h('div', { class: 'small' }, `${label}: ${v.toLocaleString()} ${unit}`);
  const pct = Math.min(100, (value / target) * 100);
  return h('div', { class: 'meter-row' },
    h('div', { class: 'spread small' }, h('span', {}, label), h('span', { class: 'muted' }, `${v.toLocaleString()} / ${Math.round(target).toLocaleString()} ${unit}`)),
    h('div', { class: `meter ${value > target * 1.1 ? 'over' : ''}`, role: 'meter', 'aria-valuenow': v, 'aria-valuemax': Math.round(target), 'aria-label': label },
      h('span', { style: `width:${pct}%` })));
}

/** Today's kcal and macros against the coach's targets. */
function todayMeters(meals, target) {
  const t = sum(meals);
  return h('div', { class: 'meters' },
    meter('Calories', t.kcal, target?.kcal, 'kcal'),
    MACROS.filter(([k]) => target?.[k] || t[k] > 0).map(([k, label]) => meter(label, t[k], target?.[k], 'g')));
}

/** For the client's Today screen: today's totals, the last meals and "+ Log a meal". Fills `slot`. */
export async function todayCard(slot, me) {
  const draw = async () => {
    let data;
    try { data = await load(me.userId); } catch { return; } // offline with nothing cached
    const meals = data.meals ?? [];
    const today = meals.filter((m) => m.date === data.today);
    const recent = meals.filter((m) => (m.date ?? '') >= addDays(data.today, -2)).slice(-5).reverse();
    const plan = await todaysPlan(me, draw);
    mount(slot,
      h('div', { class: 'card' },
        h('div', { class: 'spread' }, h('h3', {}, 'Meals'),
          h('div', { class: 'row' }, h('a', { class: 'btn small ghost', href: '#/recipes' }, 'Recipes'), h('button', { class: 'small ghost', onclick: () => logSheet(me, data.today, draw) }, '+ Log a meal'))),
        plan,
        (today.some((m) => m.items?.some(estimated)) || data.target) && todayMeters(today, data.target),
        data.target?.note && h('p', { class: 'small muted' }, `Coach: ${data.target.note}`),
        recent.length
          ? recent.map((m) => h('button', { class: 'linklike small meal-line', onclick: () => mealSheet(me.userId, m, true, draw) }, mealLine(m)))
          : h('p', { class: 'muted small' }, 'Nothing logged in the last days.'),
        h('button', { class: 'small ghost', onclick: () => importSheet(me.userId, draw) }, 'Import from Cronometer or MyFitnessPal')));
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

const fmtMacros = (x) => `${Math.round(x.kcal)} kcal · P ${Math.round(x.protein)} · C ${Math.round(x.carbs)} · F ${Math.round(x.fat)} g`;

/** One meal's lines with their numbers; the client can delete their own. */
function mealSheet(cid, m, mine, onDone) {
  let changed = false;
  const sh = sheet(
    h('h2', {}, `${m.label || 'Meal'} · ${fmtDate(m.date)}`),
    (m.items ?? []).map((it) => h('div', { class: 'small', style: 'margin-bottom:6px' },
      h('strong', {}, it.text), it.grams != null && ` · ${Math.round(it.grams)} g`,
      h('div', { class: 'muted' }, estimated(it) || it.basis === 'imported' ? fmtMacros(itemMacros(it)) : 'not estimated yet',
        it.basis === 'imported' && ' · imported'))),
    h('p', { class: 'small' }, h('strong', {}, 'Total: '), fmtMacros(sum([m]))),
    mine && h('button', {
      class: 'danger block',
      onclick: (e) => busy(e.currentTarget, async () => {
        // Same outbox key as the meal's PUT, so an unsynced meal is just dropped.
        await enqueue('DELETE', mealPath(cid, m.id), undefined, `meal:${m.id}`);
        changed = true;
        toast('Meal deleted');
        sh.close();
      }),
    }, 'Delete meal'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'),
  );
  sh.onClose = () => { if (changed) onDone?.(); };
}

/** Upload a CSV export; the server reads it (ptcore::foodimport). */
function importSheet(cid, onDone) {
  const file = h('input', { type: 'file', accept: '.csv,text/csv', 'aria-label': 'CSV export' });
  const result = h('div');
  let imported = false;
  const sh = sheet(
    h('h2', {}, 'Import a food diary'),
    h('p', { class: 'small' }, 'Bring meals from another app as a CSV file. The last 90 days are imported; importing the same days again replaces them.'),
    h('ul', { class: 'small muted' },
      h('li', {}, 'Cronometer (web): Settings › Account › Export Data › Food & Recipe Entries.'),
      h('li', {}, 'MyFitnessPal (web): Settings › Export Data. Use the Nutrition file; it has meal totals, not single foods.')),
    file, result,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const f = file.files?.[0];
        if (!f) { toast('Choose a CSV file', 'error'); return; }
        if (!navigator.onLine) { toast('Importing needs a connection', 'error'); return; }
        if (f.size > 1_500_000) { toast('That file is over 1.5 MB; export a shorter date range', 'error'); return; }
        const r = await post(`/clients/${cid}/meals/import`, { csv: await f.text() });
        imported = true;
        mount(result, h('p', { class: 'small' }, `Imported ${r.meals} meal${r.meals === 1 ? '' : 's'} over ${r.days} day${r.days === 1 ? '' : 's'}${r.skipped ? ` (${r.skipped} rows skipped: no date, no numbers, or older than 90 days)` : ''}.`));
        toast('Import done');
      }),
    }, 'Import'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'),
  );
  sh.onClose = () => { if (imported) onDone?.(); };
}

/** For the coach's Overview tab: targets, the week's averages, per-day dots and the last meals. */
export async function overviewCard(client) {
  const card = h('div', { class: 'card' });
  const draw = async () => {
    const data = await load(client.userId);
    const days = Array.from({ length: 14 }, (_, i) => addDays(data.today, i - 13));
    const w = data.week;
    const t = data.target;
    mount(card,
      h('div', { class: 'spread' }, h('h3', {}, 'Food journal'),
        h('div', { class: 'row' },
          h('button', { class: 'small ghost', onclick: () => mealPlanSheet(client, draw) }, 'Meal plan'),
          h('button', { class: 'small ghost', onclick: () => targetSheet(client.userId, t, draw) }, t ? 'Targets' : 'Set targets'),
          h('button', { class: 'small', onclick: () => estimate(client.userId, draw) }, 'Estimate'))),
      t && h('p', { class: 'small muted' }, 'Targets: ', [t.kcal && `${t.kcal} kcal`, ...MACROS.filter(([k]) => t[k] != null).map(([k, l]) => `${l} ${t[k]} g`)].filter(Boolean).join(' · '),
        t.note && ` — ${t.note}`),
      (data.meals ?? []).length === 0 ? h('p', { class: 'muted small' }, 'Nothing logged yet. The client notes what they eat; you add the calories.')
        : [
          w?.logged > 0 && h('div', { class: 'meters' },
            h('div', { class: 'small' }, h('strong', {}, '7-day average'), ` (${w.logged} of 7 days logged${w.onTarget != null ? `, ${w.onTarget} within 10% of target` : ''})`),
            meter('Calories', w.average.kcal, t?.kcal, 'kcal'),
            MACROS.filter(([k]) => t?.[k] || w.average[k] > 0).map(([k, l]) => meter(l, w.average[k], t?.[k], 'g'))),
          h('div', { class: 'dots', 'aria-label': 'meals logged per day, last 14 days' }, days.map((d) => {
            const k = dayKcal(data.totals?.[d]);
            return h('span', { class: `dot ${k != null ? 'on' : ''}`, title: `${fmtDate(d)}: ${k != null ? `${Math.round(k)} kcal` : 'nothing logged'}` });
          })),
          (data.meals ?? []).slice(-4).reverse().map((m) => h('button', { class: 'linklike small meal-line', onclick: () => mealSheet(client.userId, m, false) }, mealLine(m))),
        ]);
  };
  draw().catch((e) => mount(card, h('p', { class: 'muted small' }, e.message)));
  return card;
}

/** The coach's daily targets for a client; all blank clears them. */
function targetSheet(cid, cur, onDone) {
  const input = (k, ph) => h('input', { type: 'number', inputmode: 'decimal', min: 0, step: 'any', value: cur?.[k] ?? '', placeholder: ph });
  const kcal = input('kcal', 'e.g. 2200');
  const macros = Object.fromEntries(MACROS.map(([k]) => [k, input(k, 'grams')]));
  const note = h('input', { type: 'text', maxlength: 280, value: cur?.note ?? '', placeholder: 'e.g. protein first, carbs around training' });
  let saved = false;
  const sh = sheet(
    h('h2', {}, 'Daily nutrition targets'),
    h('p', { class: 'small muted' }, 'Set only what matters for this client. The client sees these on Today.'),
    field('Calories (kcal)', kcal),
    h('div', { class: 'grid2' }, MACROS.map(([k, l]) => field(`${l} (g)`, macros[k]))),
    field('Note for the client', note),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const body = { kcal: num(kcal.value), note: note.value || null, ...Object.fromEntries(MACROS.map(([k]) => [k, num(macros[k].value)])) };
        await put(`/clients/${cid}/nutrition/target`, body);
        saved = true;
        toast('Targets saved');
        sh.close();
      }),
    }, 'Save'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
  sh.onClose = () => { if (saved) onDone?.(); };
}

/** A search candidate or saved staple as the snapshot stored on an item. */
const snapOf = (c) => ({ fdcId: c.fdcId, description: c.name, perGrams: c.grams ?? 100, kcalPer: c.kcal ?? 0, proteinPer: c.protein ?? null, carbsPer: c.carbs ?? null, fatPer: c.fat ?? null });
const words = (s) => s.toLowerCase().split(/[^a-z0-9]+/).filter((w) => w.length > 2);

/** The coach's estimate pass: for each item without an estimate, pick a saved
 * staple or one of the food-search matches (or set the kcal by hand) and
 * check the amount, then save. A searched match can be kept as a staple. */
async function estimate(cid, onDone) {
  const data = await load(cid);
  // Meals are editable for a week (habits' EDITABLE_DAYS), so older ones can't be estimated.
  const meals = (data.meals ?? []).filter((m) => (m.date ?? '') >= addDays(data.today, -6));
  if (!meals.length) { toast('Nothing from the past week to estimate'); return; }
  const staples = await get('/foods').catch(() => []);
  const pick = select(meals.map((m) => [m.id, `${m.label || 'Meal'} · ${fmtDate(m.date)}`]), meals[meals.length - 1].id);
  const rows = h('div');
  let pending = []; // per item: { get: () => the item to save, keep: () => a staple to save or null }
  let searchOff = null; // the API's message when food search isn't set up
  let saved = false;

  async function show() {
    const m = meals.find((x) => x.id === pick.value);
    mount(rows, h('p', { class: 'muted small' }, 'Looking up foods…'));
    pending = await Promise.all(m.items.map(async (it) => {
      if (estimated(it)) {
        return { row: h('div', { class: 'small' }, it.text, ' · ', fmtMacros(itemMacros(it)), it.food ? ` (${it.food.description ?? 'food'})` : ' (set by hand)'), get: () => it, keep: () => null };
      }
      const ws = words(it.text);
      const mine = staples.filter((s) => s.snap && words(s.name).some((w) => ws.includes(w)))
        .map((s) => ({ staple: true, name: s.name, fdcId: s.snap.fdcId, grams: s.snap.perGrams, kcal: s.snap.kcalPer, protein: s.snap.proteinPer, carbs: s.snap.carbsPer, fat: s.snap.fatPer }));
      const found = searchOff ? { candidates: [] } : await api('GET', `/foods/search?q=${encodeURIComponent(it.text)}`).catch(() => ({ candidates: [] }));
      searchOff ||= found.off ?? null;
      const cands = [...mine, ...(found.candidates ?? []).slice(0, 5)];
      const choice = select([['', 'No match'], ...cands.map((c, i) => [String(i), `${c.staple ? '★ ' : ''}${c.name}${c.kcal != null ? ` · ${Math.round(c.kcal)} kcal/${Math.round(c.grams ?? 100)} g` : ''}`])], cands.length ? '0' : '', { 'aria-label': `Food for ${it.text}` });
      const grams = h('input', { type: 'number', inputmode: 'decimal', min: 0, step: 'any', value: it.grams != null ? Math.round(it.grams) : '', placeholder: 'grams', 'aria-label': `Grams of ${it.text}`, style: 'width:90px' });
      const kcal = h('input', { type: 'number', inputmode: 'decimal', min: 0, step: 'any', placeholder: 'or kcal', 'aria-label': `Calories for ${it.text}`, style: 'width:90px' });
      const keep = h('input', { type: 'checkbox', 'aria-label': `Save the match for ${it.text} as a staple` });
      return {
        row: h('div', { class: 'field' }, h('div', { class: 'small' }, h('strong', {}, it.text), it.grams == null && h('span', { class: 'muted' }, ' · no amount given')),
          h('div', { class: 'row' }, h('div', { class: 'grow' }, choice), grams, h('span', { class: 'small muted' }, 'g'), kcal),
          h('label', { class: 'small muted' }, keep, ' Save this match as a staple')),
        get: () => {
          if (num(kcal.value) != null) return { ...it, kcal: num(kcal.value), basis: 'manual' };
          const c = cands[Number(choice.value)];
          if (choice.value === '' || !c) return it;
          return { ...it, grams: num(grams.value) ?? it.grams ?? null, basis: 'searched', food: snapOf(c) };
        },
        keep: () => {
          const c = cands[Number(choice.value)];
          return keep.checked && choice.value !== '' && c && !c.staple ? { name: it.text.slice(0, 60), snap: snapOf(c) } : null;
        },
      };
    }));
    mount(rows, searchOff && h('p', { class: 'small muted' }, searchOff), pending.map((p) => p.row));
  }

  pick.addEventListener('change', show);
  const sh = sheet(
    h('h2', {}, 'Estimate a meal'),
    pick,
    rows,
    h('p', { class: 'muted small' }, 'Without an amount, a match can’t give calories: add the grams, or type the kcal yourself. ★ marks your saved staples.'),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const m = meals.find((x) => x.id === pick.value);
        await api('PUT', mealPath(cid, m.id), { date: m.date, label: m.label, items: pending.map((p) => p.get()) });
        for (const s of pending.map((p) => p.keep()).filter(Boolean)) {
          await post('/foods', s).catch((err) => toast(err.message, 'error'));
        }
        saved = true;
        toast('Meal saved');
        sh.close();
      }),
    }, 'Save'),
    h('button', { class: 'ghost block', onclick: () => staplesSheet(staples) }, 'Saved staples'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
  sh.onClose = () => { if (saved) onDone?.(); };
  show();
}

/** The coach's saved staples, to remove ones no longer used. */
function staplesSheet(staples) {
  const list = h('div');
  const draw = () => mount(list, staples.length === 0 ? h('p', { class: 'muted small' }, 'None yet. Tick “Save this match as a staple” when estimating.')
    : staples.map((s) => h('div', { class: 'spread small', style: 'margin-bottom:6px' },
      h('span', {}, s.name, s.snap && h('span', { class: 'muted' }, ` · ${Math.round(s.snap.kcalPer)} kcal/${Math.round(s.snap.perGrams)} g`)),
      h('button', {
        class: 'small ghost', 'aria-label': `Delete ${s.name}`,
        onclick: (e) => busy(e.currentTarget, async () => {
          await api('DELETE', `/foods/${s.id}`);
          staples.splice(staples.indexOf(s), 1);
          draw();
        }),
      }, 'Delete'))));
  draw();
  const sh = sheet(h('h2', {}, 'Saved staples'), list, h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'));
}
