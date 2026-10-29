// The progress report (BZ-6): a printable summary of a client's last few
// weeks (adherence, strength, records, bodyweight, check-ins, habits) with a
// note from the coach. "Print / Save as PDF" uses the browser's own PDF
// export (the share sheet on a phone), so nothing is generated on the server.
// Coaches open it from a client's Overview; clients from Progress.

import { get } from '../api.js';
import { session } from '../session.js';
import { $app, addDays, barChart, fmtDate, h, lineChart, mount, round, select, todayIso, units } from '../ui.js';

const WEEKS = [[4, 'Last 4 weeks'], [8, 'Last 8 weeks'], [12, 'Last 12 weeks'], [26, 'Last 6 months']];
const avg = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

export async function renderReport(cid, weeks = 8) {
  const u = units(session.profile);
  const isCoach = session.isCoach;
  const days = weeks * 7;
  const from = addDays(todayIso(), -(days - 1));
  const [detail, dash, weights, records, checkins, habits] = await Promise.all([
    isCoach ? get(`/clients/${cid}`) : Promise.resolve({ profile: session.profile }),
    get(`/clients/${cid}/dashboard?weeks=${weeks}`),
    get(`/clients/${cid}/metrics?kind=weight`).catch(() => []),
    get(`/clients/${cid}/records`).catch(() => []),
    get(`/clients/${cid}/checkins`).catch(() => ({ checkins: [] })),
    get(`/clients/${cid}/habits?days=${Math.min(days, 90)}`).catch(() => ({ habits: [], logs: [] })),
  ]);
  const client = detail.profile;
  const a = dash.adherence;

  // bodyweight over the period: first and last entries, and the change
  const w = weights.filter((m) => m.date >= from && m.weightKg != null).sort((x, y) => x.date.localeCompare(y.date));
  const wChange = w.length > 1 ? u.toW(w.at(-1).weightKg) - u.toW(w[0].weightKg) : null;
  // new bests set in the period (not first-time baselines)
  const prs = records.filter((r) => r.date >= from && !r.first).sort((x, y) => y.date.localeCompare(x.date));
  // check-ins in the period: the average of each 1-10 question
  const cis = checkins.checkins.filter((c) => c.weekStart >= addDays(from, -6));
  const scales = new Map();
  for (const c of cis) for (const q of c.questions) if (q.kind === 'scale' && typeof c.answers[q.id] === 'number') {
    if (!scales.has(q.id)) scales.set(q.id, { label: q.label, values: [] });
    scales.get(q.id).values.push(c.answers[q.id]);
  }
  // habits: share of days each was met, counted from when it was added (or
  // the period's start; logs come back for 90 days at most)
  const habitFrom = addDays(todayIso(), -(Math.min(days, 90) - 1));
  const habitRows = habits.habits.filter((x) => x.active).map((hb) => {
    const since = [habitFrom, hb.createdAt.slice(0, 10)].sort().at(-1);
    const span = Math.round((Date.parse(todayIso()) - Date.parse(since)) / 864e5) + 1;
    const met = habits.logs.filter((l) => l.habitId === hb.id && l.date >= since && l.value >= hb.target - 1e-9).length;
    return { name: hb.name, met, span, pct: Math.round((met / span) * 100) };
  });
  // strength: each lift's estimated 1RM, first and best in the period
  const lifts = dash.strength.slice(0, 6).map((s) => ({
    name: s.exerciseName,
    first: u.toW(s.points[0]?.e1rmKg),
    best: u.toW(s.bestE1rmKg),
    points: s.points,
  }));

  const pick = select(WEEKS.map(([k, l]) => [String(k), l]), String(weeks), { 'aria-label': 'Report period', onchange: (e) => renderReport(cid, Number(e.target.value)) });
  const stat = (value, label) => h('div', { class: 'report-stat' }, h('div', { class: 'stat' }, value), h('div', { class: 'stat-label' }, label));

  mount($app(),
    h('div', { class: 'no-print row', style: 'margin-bottom:8px' },
      h('a', { class: 'btn ghost small', href: isCoach ? `#/c/${cid}/overview` : '#/progress' }, '‹ Back'),
      h('div', { class: 'grow' }, pick),
      h('button', { class: 'primary', onclick: () => window.print() }, 'Print / Save as PDF')),
    h('div', { class: 'report' },
      h('h1', {}, 'Progress report'),
      h('p', { class: 'muted' }, `${client.name} · ${fmtDate(from)} – ${fmtDate(todayIso())}`),
      h('div', { class: 'card report-stats' },
        stat(a.pct != null ? `${Math.round(a.pct)}%` : '—', 'Workouts done as planned'),
        stat(`${a.completed}/${a.due}`, 'Workouts completed'),
        stat(String(prs.length), 'Personal records'),
        stat(wChange == null ? '—' : `${wChange > 0 ? '+' : ''}${round(wChange, 1)} ${u.w}`, 'Bodyweight change')),

      lifts.length > 0 && h('div', { class: 'card' },
        h('h2', {}, `Strength: estimated 1RM (${u.w})`),
        lineChart(lifts.slice(0, 4).map((l) => ({ label: l.name, points: l.points.map((p) => ({ x: p.date, y: u.toW(p.e1rmKg) })) }))),
        h('table', {}, h('tr', {}, h('th', {}, 'Lift'), h('th', {}, 'Start'), h('th', {}, 'Best'), h('th', {}, 'Change')),
          lifts.map((l) => h('tr', {}, h('td', {}, l.name), h('td', {}, l.first ?? '—'), h('td', {}, l.best),
            h('td', {}, l.first != null ? `${l.best - l.first >= 0 ? '+' : ''}${round(l.best - l.first, 1)}` : '—'))))),

      prs.length > 0 && h('div', { class: 'card' },
        h('h2', {}, 'Personal records'),
        h('ul', {}, prs.slice(0, 15).map((r) => h('li', {},
          `${fmtDate(r.date)} · ${r.exerciseName}: ${r.reps == null ? 'estimated 1RM' : `${r.reps}-rep max`} ${u.toW(r.kg)} ${u.w}`,
          r.previous && h('span', { class: 'muted' }, ` (was ${u.toW(r.previous.kg)})`))))),

      dash.weeklyVolume.length > 0 && h('div', { class: 'card' },
        h('h2', {}, `Weekly training volume (${u.w} × reps)`),
        barChart(dash.weeklyVolume.map((v) => ({ x: fmtDate(v.weekStart, { month: 'short', day: 'numeric' }), y: u.toW(v.volumeKg) })))),

      w.length > 0 && h('div', { class: 'card' },
        h('h2', {}, `Bodyweight (${u.w})`),
        lineChart([{ label: 'Bodyweight', points: w.map((m) => ({ x: m.date, y: u.toW(m.weightKg) })) }])),

      scales.size > 0 && h('div', { class: 'card' },
        h('h2', {}, `Weekly check-ins (${cis.length})`),
        h('table', {}, h('tr', {}, h('th', {}, 'Question'), h('th', {}, 'Average (1-10)')),
          [...scales.values()].map((s) => h('tr', {}, h('td', {}, s.label), h('td', {}, round(avg(s.values), 1)))))),

      habitRows.length > 0 && h('div', { class: 'card' },
        h('h2', {}, 'Habits'),
        h('table', {}, h('tr', {}, h('th', {}, 'Habit'), h('th', {}, 'Days met')),
          habitRows.map((r) => h('tr', {}, h('td', {}, r.name), h('td', {}, `${r.met} of ${r.span} (${r.pct}%)`))))),

      h('div', { class: 'card' },
        h('h2', {}, isCoach ? 'Coach’s notes' : 'Notes'),
        // Typed here and printed with the report; not saved.
        h('div', { class: 'report-notes', contenteditable: 'true', role: 'textbox', 'aria-multiline': 'true', 'aria-label': 'Notes for the report',
          'data-placeholder': isCoach ? 'What went well, what’s next… (printed with the report)' : 'Anything to add (printed with the report)' })),
      h('p', { class: 'small muted' }, `Prepared ${fmtDate(todayIso())} with Personal Training Coach.`)),
  );
}
