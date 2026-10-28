// Personal records (LG-5). The server finds them when a log is saved; after
// finishing a workout the app waits for the outbox to send it, then asks for
// that log's new records and celebrates them. The Records tab of Progress
// (views/metrics.js) lists them for the client and their coach.

import { api, get, myOutbox } from '../api.js';
import { session } from '../session.js';
import { confirmSheet, fmtDate, h, mount, sheet, toast, units } from '../ui.js';

const label = (r) => (r.reps == null ? 'Estimated 1RM' : `${r.reps}-rep max`);

/** Resolves once the queued save of log `logId` has been sent (or after `ms`). */
async function sent(logId, ms = 15000) {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    if (!(await myOutbox()).some((i) => i.ref === `log:${logId}`)) return true;
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

/** Shows the records a just-finished workout set, if any. */
export async function celebrate(clientId, logId) {
  try {
    if (!(await sent(logId))) return; // still offline: the Records tab will show them
    const recs = await get(`/clients/${clientId}/records?log=${encodeURIComponent(logId)}`);
    if (!recs.length) return;
    const u = units(session.profile);
    const sh = sheet(
      h('div', { class: 'center pr-burst', 'aria-hidden': 'true' }, '🏆'),
      h('h2', { class: 'center' }, recs.length === 1 ? 'New personal record!' : `${recs.length} new personal records!`),
      h('ul', { class: 'pr-list' }, recs.map((r) => h('li', {},
        h('strong', {}, r.exerciseName), ` · ${label(r)}: ${u.toW(r.kg)} ${u.w}`,
        r.previous && h('span', { class: 'muted small' }, ` (was ${u.toW(r.previous.kg)} ${u.w})`)))),
      h('button', { class: 'primary block', onclick: () => sh.close() }, 'Nice'),
    );
  } catch { /* a celebration is a bonus; never surface an error for it */ }
}

/** The Records tab: best per exercise, e1RM first, then rep-maxes. */
export async function records(el, client) {
  const u = units(session.profile);
  const all = await get(`/clients/${client.userId}/records`);
  const byExercise = {};
  for (const r of all) (byExercise[r.exerciseName] ||= []).push(r);
  const names = Object.keys(byExercise).sort((a, b) => a.localeCompare(b));
  mount(el,
    h('p', { class: 'muted small' }, 'Heaviest weight for 1-10 reps and best estimated 1RM, from sets marked done. 🏆 marks a new best; the first time a lift is logged sets its starting point.'),
    names.length === 0 && h('div', { class: 'card muted' }, 'No records yet. They appear once workouts with weights are logged.'),
    names.map((name) => h('div', { class: 'card' },
      h('h3', {}, name),
      byExercise[name]
        .sort((a, b) => (a.reps ?? 0) - (b.reps ?? 0))
        .map((r) => h('div', { class: 'spread max-row' },
          h('div', {},
            h('strong', {}, `${label(r)}: ${u.toW(r.kg)} ${u.w}`), r.first ? '' : ' 🏆',
            h('div', { class: 'muted small' }, [
              fmtDate(r.date),
              r.set && `from ${u.toW(r.set[0])} ${u.w} × ${r.set[1]}`,
              r.previous && `was ${u.toW(r.previous.kg)} ${u.w}`,
            ].filter(Boolean).join(' · '))),
          h('button', {
            class: 'small ghost', 'aria-label': `Remove ${name} ${label(r)}`,
            onclick: async () => {
              if (!(await confirmSheet(`Remove this ${label(r)} record for ${name}? Use this for a mistyped weight.${r.previous ? ' The one before it comes back.' : ''}`, 'Remove'))) return;
              await api('DELETE', `/clients/${client.userId}/records/${encodeURIComponent(r.key)}`);
              toast('Record removed');
              records(el, client);
            },
          }, '✕'))))),
  );
}
