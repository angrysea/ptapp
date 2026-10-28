// Progression (WB-6): the rule editor for a program item, and the coach's
// suggestions card on the Clients page. A met rule only ever suggests; the
// coach accepts (optionally with their own numbers) or dismisses.

import { api, get } from '../api.js';
import { session } from '../session.js';
import { busy, fmtDate, h, mount, num, toast, units } from '../ui.js';

/** "Add 5 lb at RPE ≤ 8, up to 225 lb", for the collapsed rule. */
function describe(rule, u) {
  if (!rule) return 'None';
  const add = [rule.addKg != null && `+${u.toW(rule.addKg)} ${u.w}`, rule.addReps != null && `+${rule.addReps} rep${rule.addReps === 1 ? '' : 's'}`].filter(Boolean).join(' or ');
  return [add, rule.maxRpe != null && `at RPE ≤ ${rule.maxRpe}`, rule.capKg != null && `up to ${u.toW(rule.capKg)} ${u.w}`].filter(Boolean).join(', ');
}

/** The progression rule editor for one program item. */
export function ruleEditor(it, u, touch) {
  const r = it.progression || {};
  const input = (value, attrs) => h('input', { type: 'number', inputmode: 'decimal', step: 'any', min: 0, value: value ?? '', ...attrs });
  const addW = input(r.addKg != null ? u.toW(r.addKg) : '', { 'aria-label': `Add load (${u.w})` });
  const addReps = input(r.addReps, { 'aria-label': 'Add reps', step: 1, min: 1, max: 5 });
  const maxRpe = input(r.maxRpe, { 'aria-label': 'At RPE or lower', step: '0.5', min: 1, max: 10 });
  const cap = input(r.capKg != null ? u.toW(r.capKg) : '', { 'aria-label': `Up to (${u.w})` });
  const summary = h('summary', {}, `Progression: ${describe(it.progression, u)}`);
  const store = () => {
    const rule = { addKg: u.fromW(addW.value), addReps: num(addReps.value), maxRpe: num(maxRpe.value), capKg: u.fromW(cap.value) };
    it.progression = rule.addKg != null || rule.addReps != null ? rule : null;
    summary.textContent = `Progression: ${describe(it.progression, u)}`;
    touch();
  };
  for (const el of [addW, addReps, maxRpe, cap]) el.addEventListener('change', store);
  const lbl = (text, el) => h('label', { class: 'small' }, text, el);
  return h('details', { class: 'rule', open: !!it.progression },
    summary,
    h('p', { class: 'muted small' }, 'When every set hits the reps (and the RPE, if set), suggest a change for the next workout with this exercise. You approve it on the Clients page.'),
    h('div', { class: 'item-grid' },
      lbl(`Add ${u.w}`, addW), lbl('or add reps', addReps), lbl('At RPE ≤', maxRpe), lbl(`Up to ${u.w}`, cap)));
}

/** The coach's pending suggestions, for the top of the Clients page. */
export async function suggestionsCard() {
  const list = await get('/suggestions').catch(() => []);
  if (!list.length) return null;
  const u = units(session.profile);
  const card = h('div', { class: 'card' });
  const draw = (items) => {
    if (!items.length) return card.remove();
    mount(card,
      h('h2', {}, `Progression suggestions (${items.length})`),
      items.map((s) => {
        const load = s.proposed.loadKg != null && h('input', { type: 'number', inputmode: 'decimal', step: 'any', value: u.toW(s.proposed.loadKg), 'aria-label': `New load (${u.w})` });
        const reps = s.proposed.reps != null && h('input', { value: s.proposed.reps, 'aria-label': 'New reps' });
        const done = (msg) => { toast(msg); draw(items.filter((x) => x !== s)); };
        return h('div', { class: 'suggestion' },
          h('div', {}, h('strong', {}, s.clientName), ` · ${s.exerciseName}`),
          h('div', { class: 'muted small' }, `${fmtDate(s.logDate)}: ${s.why}`),
          h('div', { class: 'row', style: 'margin-top:6px;align-items:center' },
            h('span', { class: 'small' }, `${fmtDate(s.targetDate)}:`),
            load && [h('span', { class: 'small muted' }, `${u.toW(s.current.loadKg) ?? '—'} →`), load, h('span', { class: 'small' }, u.w)],
            reps && [h('span', { class: 'small muted' }, `${s.current.reps} reps →`), reps]),
          h('div', { class: 'row', style: 'margin-top:6px' },
            h('button', {
              class: 'small primary',
              onclick: (e) => busy(e.currentTarget, async () => {
                try {
                  await api('POST', `/suggestions/${encodeURIComponent(s.id)}/accept`, { loadKg: load ? u.fromW(load.value) : null, reps: reps ? reps.value : null });
                  done(`${s.clientName}’s ${fmtDate(s.targetDate)} workout updated`);
                } catch (err) {
                  if (err.status !== 409) throw err;
                  done(err.body?.current?.message || 'That workout has changed; suggestion closed');
                }
              }),
            }, 'Accept'),
            h('button', {
              class: 'small ghost',
              onclick: (e) => busy(e.currentTarget, async () => {
                await api('POST', `/suggestions/${encodeURIComponent(s.id)}/dismiss`);
                done('Dismissed');
              }),
            }, 'Dismiss')));
      }));
  };
  draw(list);
  return card;
}
