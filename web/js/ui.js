// Small DOM + formatting helpers. All user content goes in as text nodes.

export function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  let value;
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === 'value') value = v;
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (k === 'class') el.className = v;
    else if (k === 'checked' || k === 'disabled' || k === 'selected' || k === 'hidden') el[k] = !!v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid instanceof Node ? kid : String(kid));
  }
  if (value !== undefined) el.value = value;
  return el;
}

export function mount(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return el;
}

export const $app = () => document.getElementById('app');

export function toast(msg, kind = '') {
  const box = document.getElementById('toast');
  const d = h('div', { class: kind }, msg);
  box.append(d);
  setTimeout(() => d.remove(), kind === 'error' ? 5000 : 2500);
}

export function errorToast(e) {
  console.error(e);
  toast(e?.message || String(e), 'error');
}

let fieldSeq = 0;
/** Labelled form field; the label is associated with the control for screen
 * readers. `info` (an `infoButton`) sits beside the label, outside it. */
export function field(label, input, hint, info) {
  const control = input.matches?.('input, select, textarea') ? input : null;
  if (control && !control.id) control.id = `f${++fieldSeq}`;
  const lbl = h('label', { for: control?.id }, label);
  return h('div', { class: 'field' }, info ? h('div', { class: 'label-row' }, lbl, info) : lbl, input, hint && h('div', { class: 'muted small' }, hint));
}

/** A small ⓘ button that explains `title` in a sheet. */
export function infoButton(title, ...content) {
  return h('button', {
    type: 'button',
    class: 'info',
    'aria-label': `What is ${title}?`,
    onclick: (e) => {
      e.preventDefault();
      e.stopPropagation();
      const s = sheet(h('h2', {}, title), ...content, h('button', { class: 'primary block', onclick: () => s.close() }, 'Got it'));
    },
  }, 'i');
}

const EYE = '<svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7S1 12 1 12z"/><circle cx="12" cy="12" r="3"/></svg>';
const EYE_OFF = '<svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.9 17.9A10.4 10.4 0 0 1 12 19c-7 0-11-7-11-7a19 19 0 0 1 5.1-5.9M9.9 4.2A9.7 9.7 0 0 1 12 4c7 0 11 7 11 7a19 19 0 0 1-2.2 3.2"/><path d="M14.1 14.1a3 3 0 1 1-4.2-4.2"/><path d="M1 1l22 22"/></svg>';

/** A labelled password box with an eye button inside it that shows or hides
 * what's typed. Returns `{ el, input }`. */
export function passwordField(label, attrs = {}) {
  const input = h('input', { type: 'password', ...attrs });
  input.id = `f${++fieldSeq}`;
  const eye = h('button', { type: 'button', class: 'eye', 'aria-label': `Show ${label.toLowerCase()}`, 'aria-pressed': 'false', 'aria-controls': input.id });
  eye.innerHTML = EYE; // static markup
  eye.addEventListener('click', () => {
    const show = input.type === 'password';
    input.type = show ? 'text' : 'password';
    eye.setAttribute('aria-pressed', String(show));
    eye.setAttribute('aria-label', `${show ? 'Hide' : 'Show'} ${label.toLowerCase()}`);
    eye.innerHTML = show ? EYE_OFF : EYE;
  });
  const el = h('div', { class: 'field' }, h('label', { for: input.id }, label), h('div', { class: 'pw' }, input, eye));
  return { el, input };
}

export function select(options, value, attrs = {}) {
  return h('select', { ...attrs, value },
    options.map((o) => (Array.isArray(o) ? h('option', { value: o[0] }, o[1]) : h('option', { value: o }, o))));
}

export function sheet(...content) {
  const back = h('div', { class: 'sheet-backdrop' });
  const box = h('div', { class: 'sheet' }, ...content);
  back.append(box);
  back.addEventListener('click', (e) => { if (e.target === back) close(); });
  document.body.append(back);
  // `onClose` runs however the sheet closes (a button or a tap outside it),
  // e.g. to stop a video that would otherwise keep playing.
  const api = { close, box, onClose: null };
  function close() { back.remove(); api.onClose?.(); }
  return api;
}

export function confirmSheet(message, okLabel = 'OK') {
  return new Promise((resolve) => {
    const s = sheet(
      h('p', {}, message),
      h('div', { class: 'row' },
        h('button', { class: 'grow', onclick: () => { s.close(); resolve(false); } }, 'Cancel'),
        h('button', { class: 'primary grow', onclick: () => { s.close(); resolve(true); } }, okLabel)),
    );
  });
}

/** Subtab bar: tabs = [[key,label]], returns element; onPick(key). */
export function subtabs(tabs, active, onPick) {
  const bar = h('div', { class: 'subtabs' });
  for (const [k, label] of tabs) {
    bar.append(h('button', { class: k === active ? 'active' : '', onclick: () => onPick(k) }, label));
  }
  return bar;
}

export async function busy(button, fn) {
  button.disabled = true;
  try { return await fn(); } catch (e) { errorToast(e); } finally { button.disabled = false; }
}

// ------------------------------------------------------------ dates

export const isoDate = (d) => {
  const z = new Date(d.getTime() - d.getTimezoneOffset() * 60000);
  return z.toISOString().slice(0, 10);
};
export const todayIso = () => isoDate(new Date());
export const addDays = (iso, n) => { const d = new Date(iso + 'T12:00:00'); d.setDate(d.getDate() + n); return isoDate(d); };
export const fmtDate = (iso, opts = { weekday: 'short', month: 'short', day: 'numeric' }) =>
  new Date(iso + 'T12:00:00').toLocaleDateString(undefined, opts);
export const fmtDateTime = (ts) => new Date(ts).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
/** Downscales an image to at most `max` px JPEG before upload, to save storage and egress. */
export async function shrinkImage(file, max = 1600) {
  const bmp = await createImageBitmap(file);
  const scale = Math.min(1, max / Math.max(bmp.width, bmp.height));
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(bmp.width * scale);
  canvas.height = Math.round(bmp.height * scale);
  canvas.getContext('2d').drawImage(bmp, 0, 0, canvas.width, canvas.height);
  return new Promise((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.85));
}
export const uid = () => crypto.randomUUID().replace(/-/g, '');

// ------------------------------------------------------------ units (BM-8)

const KG_PER_LB = 0.45359237;
export function units(profile) {
  const imperial = profile?.units === 'imperial';
  return {
    imperial,
    w: imperial ? 'lb' : 'kg',
    l: imperial ? 'in' : 'cm',
    toW: (kg) => (kg == null ? null : round(imperial ? kg / KG_PER_LB : kg, 1)),
    fromW: (v) => (v === '' || v == null || isNaN(v) ? null : round(imperial ? Number(v) * KG_PER_LB : Number(v), 3)),
    toL: (cm) => (cm == null ? null : round(imperial ? cm / 2.54 : cm, 1)),
    fromL: (v) => (v === '' || v == null || isNaN(v) ? null : round(imperial ? Number(v) * 2.54 : Number(v), 2)),
  };
}
export const round = (v, p = 1) => Math.round(v * 10 ** p) / 10 ** p;
export const num = (v) => (v === '' || v == null || isNaN(Number(v)) ? null : Number(v));

// ------------------------------------------------------------ charts

const SVG = 'http://www.w3.org/2000/svg';
const s = (tag, attrs) => {
  const el = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  return el;
};
export const COLORS = ['#0f766e', '#d97706', '#7c3aed', '#db2777', '#2563eb'];

/**
 * Minimal line chart. series: [{ label, color, dashed, points: [{x: 'YYYY-MM-DD', y}] }]
 */
export function lineChart(series, { height = 180, yFmt = (v) => round(v, 1) } = {}) {
  const pts = series.flatMap((sr) => sr.points);
  if (pts.length === 0) return h('p', { class: 'muted small' }, 'No data yet.');
  const W = 600, H = height, L = 44, R = 10, T = 10, B = 24;
  const xs = pts.map((p) => Date.parse(p.x));
  const ys = pts.map((p) => p.y);
  let [x0, x1] = [Math.min(...xs), Math.max(...xs)];
  let [y0, y1] = [Math.min(...ys), Math.max(...ys)];
  if (x0 === x1) { x0 -= 86400000; x1 += 86400000; }
  const pad = (y1 - y0) * 0.1 || Math.abs(y1) * 0.05 || 1;
  y0 -= pad; y1 += pad;
  const X = (x) => L + ((x - x0) / (x1 - x0)) * (W - L - R);
  const Y = (y) => T + (1 - (y - y0) / (y1 - y0)) * (H - T - B);
  const svg = s('svg', { viewBox: `0 0 ${W} ${H}`, class: 'chart', role: 'img' });
  for (let i = 0; i <= 3; i++) {
    const v = y0 + ((y1 - y0) * i) / 3;
    svg.append(s('line', { x1: L, x2: W - R, y1: Y(v), y2: Y(v), class: 'grid' }));
    const t = s('text', { x: L - 6, y: Y(v) + 4, 'text-anchor': 'end' });
    t.textContent = yFmt(v);
    svg.append(t);
  }
  for (const [x, anchor] of [[x0, 'start'], [x1, 'end']]) {
    const t = s('text', { x: X(x), y: H - 6, 'text-anchor': anchor });
    t.textContent = new Date(x).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    svg.append(t);
  }
  series.forEach((sr, i) => {
    const color = sr.color || COLORS[i % COLORS.length];
    const sorted = [...sr.points].sort((a, b) => a.x.localeCompare(b.x));
    if (sorted.length > 1) {
      svg.append(s('polyline', {
        points: sorted.map((p) => `${X(Date.parse(p.x))},${Y(p.y)}`).join(' '),
        fill: 'none', stroke: color, 'stroke-width': sr.dashed ? 2 : 2.5, 'stroke-dasharray': sr.dashed ? '6 4' : '',
      }));
    }
    if (!sr.dashed) for (const p of sorted) svg.append(s('circle', { cx: X(Date.parse(p.x)), cy: Y(p.y), r: 3.5, fill: color }));
  });
  const legend = series.length > 1 || series[0].label
    ? h('div', { class: 'legend' }, series.map((sr, i) => h('span', { style: `--c:${sr.color || COLORS[i % COLORS.length]}` }, sr.label || '')))
    : null;
  return h('div', {}, svg, legend);
}

/** Bar chart for weekly totals: bars = [{x: label, y}] */
export function barChart(bars, { height = 160, yFmt = (v) => Math.round(v) } = {}) {
  if (!bars.length) return h('p', { class: 'muted small' }, 'No data yet.');
  const W = 600, H = height, L = 44, B = 22, T = 8;
  const max = Math.max(...bars.map((b) => b.y), 1);
  const bw = (W - L) / bars.length;
  const svg = s('svg', { viewBox: `0 0 ${W} ${H}`, class: 'chart', role: 'img' });
  const t = s('text', { x: L - 6, y: T + 10, 'text-anchor': 'end' });
  t.textContent = yFmt(max);
  svg.append(t);
  bars.forEach((b, i) => {
    const bh = ((H - T - B) * b.y) / max;
    svg.append(s('rect', { x: L + i * bw + 3, y: H - B - bh, width: Math.max(bw - 6, 2), height: bh, rx: 3, fill: COLORS[0] }));
    if (bars.length <= 12 || i % 2 === 0) {
      const lbl = s('text', { x: L + i * bw + bw / 2, y: H - 6, 'text-anchor': 'middle' });
      lbl.textContent = b.x;
      svg.append(lbl);
    }
  });
  return svg;
}
