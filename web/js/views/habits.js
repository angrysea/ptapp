// Daily habits (BZ-3). Clients tap tiles on Today: a check toggles, a count
// steps up and down or takes a typed number. Writes go through the offline
// outbox (one queued value per habit and day), and values still queued are
// shown over the cached list, so tiles work offline. Coaches see the last
// two weeks on the client's Overview; either side adds and archives habits.

import { api, enqueue, get, myOutbox } from '../api.js';
import { session } from '../session.js';
import { addDays, busy, field, fmtDate, h, mount, select, sheet, toast } from '../ui.js';

const dayPath = (cid, hid, date) => `/clients/${cid}/habits/${hid}/days/${date}`;
const met = (hb, v) => v >= hb.target - 1e-9;

/** Server data with this user's still-queued day values applied. */
async function load(cid) {
  // The outbox first, so a value synced while the GET runs is in one or the other.
  const queued = await myOutbox();
  const data = await get(`/clients/${cid}/habits?days=14`, { cache: true });
  const value = {};
  for (const l of data.logs) value[`${l.habitId}|${l.date}`] = l.value;
  for (const i of queued) {
    const m = i.method === 'PUT' && i.path.match(/^\/clients\/[^/]+\/habits\/([^/]+)\/days\/([^/]+)$/);
    if (m) value[`${m[1]}|${m[2]}`] = i.body.value;
  }
  return { ...data, value };
}

/** For the client's Today screen: a tile per active habit. Fills `slot`. */
export async function habitTiles(slot, me) {
  let data;
  try { data = await load(me.userId); } catch { return; } // offline with nothing cached
  const active = data.habits.filter((x) => x.active);
  const today = data.today;
  const draw = () => mount(slot,
    active.length > 0 && h('h2', {}, 'Habits'),
    active.length > 0 && h('div', { class: 'habit-grid' }, active.map(tile)),
    h('button', { class: 'small ghost', onclick: () => manage(me.userId, () => habitTiles(slot, me)) }, active.length ? 'Manage habits' : '+ Add a daily habit'));

  function tile(hb) {
    const key = `${hb.id}|${today}`;
    const v = data.value[key] || 0;
    // The server's streak ends yesterday until today is met.
    const before = data.streaks[hb.id] ?? 0;
    const serverMetToday = data.logs.some((l) => l.habitId === hb.id && l.date === today && met(hb, l.value));
    const streak = (serverMetToday ? before - 1 : before) + (met(hb, v) ? 1 : 0);
    const set = (next) => {
      data.value[key] = Math.max(0, next);
      enqueue('PUT', dayPath(me.userId, hb.id, today), { value: data.value[key] }, `habit:${hb.id}:${today}`);
      draw();
    };
    const label = h('div', {}, h('strong', {}, hb.name), streak > 0 && h('span', { class: 'streak' }, ` 🔥 ${streak}`));
    if (hb.kind === 'check') {
      return h('button', { class: `habit ${v ? 'done' : ''}`, 'aria-pressed': String(!!v), onclick: () => set(v ? 0 : 1) },
        label, h('div', { class: 'small' }, v ? '✓ Done today' : 'Tap when done'));
    }
    const input = h('input', { type: 'number', inputmode: 'decimal', min: 0, value: v || '', 'aria-label': `${hb.name} today`, onchange: (e) => set(Number(e.target.value) || 0) });
    return h('div', { class: `habit ${met(hb, v) ? 'done' : ''}` },
      label,
      h('div', { class: 'small' }, `${v || 0} / ${hb.target}${hb.unit ? ` ${hb.unit}` : ''}`),
      h('div', { class: 'row habit-count' },
        h('button', { class: 'small', 'aria-label': `One less ${hb.name}`, onclick: () => set(v - 1) }, '−'),
        input,
        h('button', { class: 'small', 'aria-label': `One more ${hb.name}`, onclick: () => set(v + 1) }, '+')));
  }
  draw();
}

/** For the coach's Overview tab: each habit's last 14 days and streak. */
export async function habitsCard(client) {
  const card = h('div', { class: 'card' });
  const draw = async () => {
    const data = await load(client.userId);
    const days = Array.from({ length: 14 }, (_, i) => addDays(data.today, i - 13));
    const active = data.habits.filter((x) => x.active);
    mount(card,
      h('div', { class: 'spread' }, h('h3', {}, 'Habits'), h('button', { class: 'small', onclick: () => manage(client.userId, draw) }, 'Manage')),
      active.length === 0 ? h('p', { class: 'muted small' }, 'No habits yet. Add daily habits for this client to tick off on their Today screen.')
        : active.map((hb) => h('div', { class: 'habit-row' },
          h('div', { class: 'spread' }, h('span', {}, hb.name, hb.kind === 'count' ? h('span', { class: 'muted small' }, ` · ${hb.target}${hb.unit ? ` ${hb.unit}` : ''}`) : ''),
            h('span', { class: 'small' }, `🔥 ${data.streaks[hb.id] ?? 0}`)),
          h('div', { class: 'dots', 'aria-label': `${hb.name}, last 14 days` }, days.map((d) => {
            const v = data.value[`${hb.id}|${d}`] || 0;
            return h('span', { class: `dot ${met(hb, v) ? 'on' : v ? 'part' : ''}`, title: `${fmtDate(d)}: ${v}` });
          })))));
  };
  draw().catch((e) => mount(card, h('p', { class: 'muted small' }, e.message)));
  return card;
}

/** Add, edit and archive habits (coach or client). */
async function manage(cid, onDone) {
  const { habits } = await get(`/clients/${cid}/habits?days=1`);
  const list = h('div');
  const name = h('input', { maxlength: 60, placeholder: 'e.g. Drink water' });
  const kind = select([['check', 'Done / not done'], ['count', 'A number with a target']], 'check');
  const target = h('input', { type: 'number', inputmode: 'decimal', min: 0, placeholder: '8' });
  const unit = h('input', { maxlength: 20, placeholder: 'glasses' });
  const countFields = h('div', { class: 'grid2', hidden: true }, field('Daily target', target), field('Unit', unit));
  kind.addEventListener('change', () => { countFields.hidden = kind.value !== 'count'; });
  const body = (hb, active) => ({ name: hb.name, kind: hb.kind, target: hb.target, unit: hb.unit || null, active });
  const drawList = (hs) => mount(list, hs.map((hb) => h('div', { class: 'spread habit-row' },
    h('span', { class: hb.active ? '' : 'muted' }, hb.name, hb.active ? '' : ' (archived)'),
    h('button', {
      class: 'small ghost',
      onclick: (e) => busy(e.currentTarget, async () => {
        const saved = await api('PUT', `/clients/${cid}/habits/${hb.id}`, body(hb, !hb.active));
        drawList(hs.map((x) => (x.id === hb.id ? saved : x)));
      }),
    }, hb.active ? 'Archive' : 'Restore'))));
  drawList(habits);
  const sh = sheet(
    h('h2', {}, 'Habits'),
    list,
    h('h3', {}, 'Add a habit'),
    field('Habit', name), field('Kind', kind), countFields,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await api('POST', `/clients/${cid}/habits`, { name: name.value, kind: kind.value, target: Number(target.value) || 0, unit: unit.value || null });
        toast('Habit added');
        sh.close();
      }),
    }, 'Add habit'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Done'),
  );
  sh.onClose = () => onDone?.();
}
