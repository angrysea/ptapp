// Body metrics (BM-1..BM-4, BM-8), progress photos (BM-7) and 1RMs (WB-5,
// views/maxes.js). Used by the client's Progress tab and the coach's client page.

import { del, get, post } from '../api.js';
import { session } from '../session.js';
import { maxes } from './maxes.js';
import {
  addDays, busy, confirmSheet, field, fmtDate, h, isoDate, lineChart, mount, num, round, select, shrinkImage, subtabs, toast, todayIso, units,
} from '../ui.js';

const TAPE = [['neck', 'Neck'], ['chest', 'Chest'], ['waist', 'Waist'], ['hips', 'Hips'], ['armL', 'Arm (L)'], ['armR', 'Arm (R)'],
  ['thighL', 'Thigh (L)'], ['thighR', 'Thigh (R)'], ['calfL', 'Calf (L)'], ['calfR', 'Calf (R)']];

const METHODS = [['jacksonPollock3', 'Jackson-Pollock 3-site'], ['jacksonPollock7', 'Jackson-Pollock 7-site'], ['durninWomersley4', 'Durnin-Womersley 4-site']];

const SITES = {
  chest: ['Chest', 'Diagonal fold halfway between the front of the armpit and the nipple (men); one-third of that distance for women.', [40, 50]],
  abdomen: ['Abdomen', 'Vertical fold 2 cm to the right of the navel.', [55, 84]],
  thigh: ['Thigh', 'Vertical fold on the front of the thigh, midway between the hip crease and the top of the kneecap.', [41, 138]],
  triceps: ['Triceps', 'Vertical fold on the back of the upper arm, midway between the shoulder tip and the elbow; arm relaxed.', [21, 66]],
  suprailiac: ['Suprailiac', 'Diagonal fold just above the hip bone, in line with the front of the armpit. (Durnin-Womersley: at the hip bone, mid-armpit line.)', [64, 92]],
  midaxillary: ['Midaxillary', 'Vertical fold on the side of the torso, in line with the middle of the armpit, at the level of the bottom of the breastbone.', [31, 74]],
  subscapular: ['Subscapular', 'Diagonal fold (45°) 1-2 cm below the lower tip of the shoulder blade (on the back).', [62, 56]],
  biceps: ['Biceps', 'Vertical fold on the front of the upper arm, over the middle of the biceps.', [79, 64]],
};

function requiredSites(method, sex) {
  if (method === 'jacksonPollock3') return sex === 'female' ? ['triceps', 'suprailiac', 'thigh'] : ['chest', 'abdomen', 'thigh'];
  if (method === 'jacksonPollock7') return ['chest', 'midaxillary', 'triceps', 'subscapular', 'abdomen', 'suprailiac', 'thigh'];
  return ['biceps', 'triceps', 'subscapular', 'suprailiac'];
}

function bodyFigure(active, focus) {
  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', '0 0 100 200');
  svg.setAttribute('class', 'body-fig');
  svg.innerHTML = `<circle class="outline" cx="50" cy="16" r="11"/>
    <path class="outline" d="M36 32 Q50 28 64 32 L82 40 L88 100 L80 102 L74 50 L68 52 L70 104 L66 190 L54 190 L50 120 L46 190 L34 190 L30 104 L32 52 L26 50 L20 102 L12 100 L18 40 Z"/>`;
  for (const [k, [, , [x, y]]] of Object.entries(SITES)) {
    if (!active.includes(k)) continue;
    const c = document.createElementNS(NS, 'circle');
    c.setAttribute('cx', x); c.setAttribute('cy', y); c.setAttribute('r', k === focus ? 6 : 4);
    c.setAttribute('class', `site ${k === focus ? 'on' : ''}`);
    svg.append(c);
  }
  return svg;
}

/** Mount the metrics UI for a client into `el`. `client` is their Profile. */
export function renderMetrics(el, client, initial = 'weight') {
  const show = (tab) => {
    const body = h('div');
    mount(el, subtabs([['weight', 'Weight'], ['tape', 'Tape'], ['skinfold', 'Body fat'], ['photos', 'Photos'], ['maxes', 'Maxes']], tab, show), body);
    ({ weight, tape, skinfold, photos, maxes })[tab](body, client).catch((e) => mount(body, h('p', { class: 'muted' }, e.message)));
  };
  show(initial);
}

// Mirrors the API: an entry can be deleted by whoever recorded it; photos by
// the client, or by the coach if the coach took them.
const myRole = () => (session.isCoach ? 'coach' : 'client');
const canDeleteMetric = (m) => m.takenBy === myRole();
const canDeletePhoto = (p) => !session.isCoach || p.takenBy === 'coach';

const metricsPath = (client, kind) => `/clients/${client.userId}/metrics?kind=${kind}`;

async function deleteMetric(client, m, rerender) {
  if (!(await confirmSheet('Delete this entry?', 'Delete'))) return;
  await del(`/clients/${client.userId}/metrics/${m.id}`);
  rerender();
}

// ------------------------------------------------------------ weight (BM-1)

async function weight(el, client) {
  const u = units(session.profile);
  const list = await get(metricsPath(client, 'weight'), { cache: true });
  const pts = list.map((m) => ({ x: m.date, y: u.toW(m.weightKg) }));
  const sorted = [...pts].sort((a, b) => a.x.localeCompare(b.x));
  // 7-day trailing average as the trend line
  const trend = sorted.map((p) => {
    const win = sorted.filter((q) => q.x <= p.x && q.x > addDays(p.x, -7));
    return { x: p.x, y: round(win.reduce((s, q) => s + q.y, 0) / win.length, 1) };
  });
  const weeks = {};
  for (const p of sorted) {
    const d = new Date(p.x + 'T12:00:00');
    d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
    (weeks[isoDate(d)] ||= []).push(p.y);
  }
  const weekly = Object.entries(weeks).sort().reverse().slice(0, 8);

  const date = h('input', { type: 'date', value: todayIso() });
  const val = h('input', { type: 'number', inputmode: 'decimal', step: '0.1', placeholder: u.w });
  mount(el,
    h('div', { class: 'card' },
      h('div', { class: 'row' }, h('div', { class: 'grow' }, field('Date', date)), h('div', { class: 'grow' }, field(`Weight (${u.w})`, val))),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!num(val.value)) throw new Error('Enter a weight');
          await post(`/clients/${client.userId}/metrics`, { date: date.value, kind: 'weight', weightKg: u.fromW(val.value) });
          toast('Saved');
          weight(el, client);
        }),
      }, 'Log weight')),
    h('div', { class: 'card' }, h('h3', {}, 'Trend'),
      lineChart([{ label: `Weight (${u.w})`, points: pts }, { label: '7-day average', points: trend, dashed: true, color: '#d97706' }])),
    weekly.length > 0 && h('div', { class: 'card' }, h('h3', {}, 'Weekly average'),
      h('table', {}, h('tr', {}, h('th', {}, 'Week of'), h('th', {}, u.w), h('th', {}, 'Change')),
        weekly.map(([wk, ys], i) => {
          const avg = ys.reduce((a, b) => a + b, 0) / ys.length;
          const prev = weekly[i + 1] && weekly[i + 1][1].reduce((a, b) => a + b, 0) / weekly[i + 1][1].length;
          return h('tr', {}, h('td', {}, fmtDate(wk)), h('td', {}, round(avg, 1)),
            h('td', {}, prev != null ? `${avg - prev >= 0 ? '+' : ''}${round(avg - prev, 1)}` : ''));
        }))),
    list.length > 0 && h('div', { class: 'card' }, h('h3', {}, 'Entries'),
      h('table', {}, [...list].reverse().slice(0, 30).map((m) => h('tr', {},
        h('td', {}, fmtDate(m.date)), h('td', {}, `${u.toW(m.weightKg)} ${u.w}`), h('td', { class: 'muted small' }, m.takenBy),
        h('td', {}, canDeleteMetric(m) && h('button', { class: 'small ghost', onclick: () => deleteMetric(client, m, () => weight(el, client)) }, '✕')))))),
  );
}

// ------------------------------------------------------------ tape (BM-2)

async function tape(el, client) {
  const u = units(session.profile);
  const list = await get(metricsPath(client, 'tape'), { cache: true });
  const latest = list[list.length - 1];
  const first = list[0];
  const date = h('input', { type: 'date', value: todayIso() });
  const inputs = Object.fromEntries(TAPE.map(([k]) => [k, h('input', { type: 'number', inputmode: 'decimal', step: '0.1' })]));
  mount(el,
    h('div', { class: 'card' },
      field('Date', date),
      h('div', { class: 'grid2' }, TAPE.map(([k, label]) => field(`${label} (${u.l})`, inputs[k]))),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          const tapeCm = {};
          for (const [k] of TAPE) if (num(inputs[k].value)) tapeCm[k] = u.fromL(inputs[k].value);
          await post(`/clients/${client.userId}/metrics`, { date: date.value, kind: 'tape', tapeCm });
          toast('Saved');
          tape(el, client);
        }),
      }, 'Save measurements')),
    latest && h('div', { class: 'card' }, h('h3', {}, `Latest (${fmtDate(latest.date)})`),
      h('table', {}, h('tr', {}, h('th', {}, 'Site'), h('th', {}, u.l), h('th', {}, `Since ${fmtDate(first.date)}`)),
        TAPE.filter(([k]) => latest.tapeCm[k] != null).map(([k, label]) => {
          const base = list.find((m) => m.tapeCm[k] != null).tapeCm[k];
          const d = u.toL(latest.tapeCm[k]) - u.toL(base);
          return h('tr', {}, h('td', {}, label), h('td', {}, u.toL(latest.tapeCm[k])), h('td', {}, `${d >= 0 ? '+' : ''}${round(d, 1)}`));
        }))),
    list.length > 1 && h('div', { class: 'card' }, h('h3', {}, 'Waist & hips'),
      lineChart([
        { label: 'Waist', points: list.filter((m) => m.tapeCm.waist).map((m) => ({ x: m.date, y: u.toL(m.tapeCm.waist) })) },
        { label: 'Hips', points: list.filter((m) => m.tapeCm.hips).map((m) => ({ x: m.date, y: u.toL(m.tapeCm.hips) })) },
      ].filter((sr) => sr.points.length))),
  );
}

// ------------------------------------------------------------ skinfold (BM-3, BM-4, BM-5)

async function skinfold(el, client) {
  const u = units(session.profile);
  const [list, weights] = await Promise.all([
    get(metricsPath(client, 'skinfold'), { cache: true }),
    get(metricsPath(client, 'weight'), { cache: true }),
  ]);
  if (!client.sex || !client.birthDate) {
    return mount(el, h('div', { class: 'banner' }, 'Body-fat equations need sex and birth date. ',
      session.isCoach ? 'Ask the client to complete their intake.' : h('a', { href: '#/intake' }, 'Complete your intake.')));
  }
  let method = 'jacksonPollock3';
  let focus = null;
  const methodSel = select(METHODS, method);
  const eqSel = select([['siri', 'Siri'], ['brozek', 'Brozek']], 'siri');
  const date = h('input', { type: 'date', value: todayIso() });
  const lastW = weights[weights.length - 1];
  const bw = h('input', { type: 'number', inputmode: 'decimal', step: '0.1', value: lastW ? u.toW(lastW.weightKg) : '' });
  const sitesBox = h('div');
  const figBox = h('div');
  let inputs = {};

  function drawSites() {
    const sites = requiredSites(method, client.sex);
    inputs = {};
    mount(figBox, bodyFigure(sites, focus));
    mount(sitesBox, sites.map((k) => {
      const [label, howto] = SITES[k];
      const r = [0, 1, 2].map((i) => h('input', {
        type: 'number', inputmode: 'decimal', step: '0.5', placeholder: i === 0 ? 'mm' : 'opt.',
        onfocus: () => { focus = k; mount(figBox, bodyFigure(sites, focus)); },
      }));
      inputs[k] = r;
      return h('div', { class: 'field' },
        h('label', {}, `${label} (mm)`),
        h('div', { class: 'small muted', style: 'margin-bottom:4px' }, howto),
        h('div', { class: 'row' }, r.map((i) => h('div', { class: 'grow' }, i))));
    }));
  }
  methodSel.addEventListener('change', () => { method = methodSel.value; focus = null; drawSites(); });
  drawSites();

  const result = h('div');
  const bfPts = list.map((m) => ({ x: m.date, y: m.skinfold.result.bodyFatPct }));
  mount(el,
    h('div', { class: 'card' },
      h('div', { class: 'grid2' }, field('Method', methodSel), field('Equation', eqSel), field('Date', date), field(`Body weight (${u.w})`, bw)),
      h('p', { class: 'muted small' }, 'Take each site 2-3 times on the right side of the body; readings are averaged.'),
      figBox, sitesBox,
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          const sitesMm = {};
          for (const [k, r] of Object.entries(inputs)) {
            const vals = r.map((i) => num(i.value)).filter((v) => v != null);
            if (!vals.length) throw new Error(`Enter ${SITES[k][0]}`);
            sitesMm[k] = vals;
          }
          const m = await post(`/clients/${client.userId}/metrics`, {
            date: date.value, kind: 'skinfold',
            skinfold: { method, equation: eqSel.value, sitesMm, bodyWeightKg: u.fromW(bw.value) },
          });
          const r = m.skinfold.result;
          mount(result, h('div', { class: 'card today' },
            h('div', { class: 'grid2' },
              stat(`${r.bodyFatPct}%`, 'Body fat'),
              stat(r.leanMassKg != null ? `${u.toW(r.leanMassKg)} ${u.w}` : '—', 'Lean mass'),
              stat(r.fatMassKg != null ? `${u.toW(r.fatMassKg)} ${u.w}` : '—', 'Fat mass'),
              stat(r.bodyDensity.toFixed(4), 'Density g/cc'))));
          toast('Saved');
        }),
      }, 'Calculate & save')),
    result,
    h('div', { class: 'card' }, h('h3', {}, 'Body fat %'), lineChart([{ label: 'Body fat %', points: bfPts }])),
    list.length > 0 && h('div', { class: 'card' }, h('h3', {}, 'History'),
      h('table', {}, h('tr', {}, h('th', {}, 'Date'), h('th', {}, 'BF%'), h('th', {}, 'Lean'), h('th', {}, 'Method'), h('th', {})),
        [...list].reverse().map((m) => h('tr', {},
          h('td', {}, fmtDate(m.date)), h('td', {}, m.skinfold.result.bodyFatPct),
          h('td', {}, m.skinfold.result.leanMassKg != null ? u.toW(m.skinfold.result.leanMassKg) : '—'),
          h('td', { class: 'small muted' }, `${METHODS.find(([k]) => k === m.skinfold.method)[1]} / ${m.skinfold.equation} · by ${m.takenBy}`),
          h('td', {}, canDeleteMetric(m) && h('button', { class: 'small ghost', onclick: () => deleteMetric(client, m, () => skinfold(el, client)) }, '✕')))))),
  );
}

export const stat = (value, label) => h('div', {}, h('div', { class: 'stat' }, value), h('div', { class: 'stat-label' }, label));

// ------------------------------------------------------------ photos (BM-7)

async function photos(el, client) {
  const list = await get(`/clients/${client.userId}/photos`);
  const byDate = {};
  for (const p of list) (byDate[p.date] ||= {})[p.pose] = p;
  const dates = Object.keys(byDate).sort().reverse();

  const date = h('input', { type: 'date', value: todayIso() });
  const pose = select([['front', 'Front'], ['side', 'Side'], ['back', 'Back']], 'front');
  const file = h('input', { type: 'file', accept: 'image/*', capture: 'environment' });
  const left = select(dates.map((d) => [d, fmtDate(d)]), dates[dates.length - 1]);
  const right = select(dates.map((d) => [d, fmtDate(d)]), dates[0]);
  const compare = h('div');
  const drawCompare = () => mount(compare, ['front', 'side', 'back'].map((p) => h('div', { class: 'row', style: 'flex-wrap:nowrap' },
    [left.value, right.value].map((d) => h('div', { class: 'grow' },
      byDate[d]?.[p] ? h('img', { src: byDate[d][p].url, alt: `${p} ${d}`, style: 'width:100%;border-radius:8px' })
        : h('div', { class: 'muted small center' }, `no ${p}`))))));
  left.addEventListener('change', drawCompare);
  right.addEventListener('change', drawCompare);

  mount(el,
    h('div', { class: 'card' },
      h('p', { class: 'muted small' }, '🔒 Private: only you and your coach can see these photos.'),
      h('div', { class: 'grid2' }, field('Date', date), field('Pose', pose)),
      field('Photo', file),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!file.files[0]) throw new Error('Choose a photo');
          const blob = await shrinkImage(file.files[0]);
          const { photo, uploadUrl, uploadHeaders } = await post(`/clients/${client.userId}/photos`, { date: date.value, pose: pose.value, contentType: 'image/jpeg', size: blob.size });
          const res = await fetch(uploadUrl, { method: 'PUT', headers: uploadHeaders, body: blob });
          if (!res.ok) throw new Error(`Upload failed (${res.status})`);
          await post(`/clients/${client.userId}/photos/${photo.id}/confirm`, {});
          toast('Photo uploaded');
          photos(el, client);
        }),
      }, 'Upload')),
    dates.length > 1 && h('div', { class: 'card' }, h('h3', {}, 'Compare'),
      h('div', { class: 'row' }, h('div', { class: 'grow' }, left), h('div', { class: 'grow' }, right)), compare),
    dates.map((d) => h('div', { class: 'card' }, h('h3', {}, fmtDate(d)),
      h('div', { class: 'photo-grid' }, ['front', 'side', 'back'].map((p) => byDate[d][p]
        ? h('div', {}, h('img', { src: byDate[d][p].url, alt: `${p} ${d}`, loading: 'lazy' }),
          canDeletePhoto(byDate[d][p]) && h('button', {
            class: 'small ghost', onclick: async () => {
              if (!(await confirmSheet('Delete this photo permanently?', 'Delete'))) return;
              await del(`/clients/${client.userId}/photos/${byDate[d][p].id}`);
              photos(el, client);
            },
          }, `✕ ${p}`))
        : h('div', { class: 'muted small center' }, p))))),
  );
  if (dates.length > 1) drawCompare();
}
