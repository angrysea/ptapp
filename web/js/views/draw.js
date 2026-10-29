// Drawing over the review video (VR-4): freehand, line, arrow, circle, a
// three-point angle with live degrees, and a timer that counts video time
// (so tempo reads true in slow motion). Marks are painted onto the review
// canvas every frame, so a recorded reply shows them as the coach draws.

import { h } from '../ui.js';

const COLORS = [['#ffd400', 'Yellow'], ['#ff3b30', 'Red'], ['#00e5ff', 'Cyan']];
const TOOLS = [['none', '✋', 'No tool (scroll and tap)'], ['pen', '✏️', 'Freehand'], ['line', '╱', 'Line'], ['arrow', '➚', 'Arrow'], ['circle', '◯', 'Circle'], ['angle', '∠', 'Angle (tap three points)']];

/** The angle at `b` between `a` and `c`, in degrees (0-180). */
export function angleAt(a, b, c) {
  const [ax, ay, cx, cy] = [a.x - b.x, a.y - b.y, c.x - b.x, c.y - b.y];
  const den = Math.hypot(ax, ay) * Math.hypot(cx, cy);
  if (!den) return 0;
  return (Math.acos(Math.max(-1, Math.min(1, (ax * cx + ay * cy) / den))) * 180) / Math.PI;
}

/**
 * Drawing tools for `canvas`; `now()` gives the video's current time for the
 * timer. Returns { toolbar, paint(ctx), clear() }: paint after each frame.
 */
export function drawingTools(canvas, now) {
  let tool = 'none';
  let color = COLORS[0][0];
  const shapes = []; // finished marks, oldest first
  let draft = null; // the mark being drawn
  let hover = null; // pointer position, for the angle preview
  let timer = null; // { start, stop } in video seconds

  const pos = (e) => {
    const r = canvas.getBoundingClientRect();
    return { x: ((e.clientX - r.left) / r.width) * canvas.width, y: ((e.clientY - r.top) / r.height) * canvas.height };
  };

  canvas.addEventListener('pointerdown', (e) => {
    if (tool === 'none') return;
    e.preventDefault();
    const p = pos(e);
    if (tool === 'angle') {
      draft = draft?.type === 'angle' ? draft : { type: 'angle', color, pts: [] };
      draft.pts.push(p);
      if (draft.pts.length === 3) { shapes.push(draft); draft = null; }
      return;
    }
    canvas.setPointerCapture(e.pointerId);
    draft = tool === 'pen' ? { type: 'pen', color, pts: [p] } : { type: tool, color, a: p, b: p };
  });
  canvas.addEventListener('pointermove', (e) => {
    if (tool === 'none') return;
    const p = pos(e);
    hover = p;
    if (!draft || draft.type === 'angle') return;
    if (draft.type === 'pen') draft.pts.push(p);
    else draft.b = p;
  });
  const finish = () => {
    if (!draft || draft.type === 'angle') return;
    const tiny = draft.type === 'pen' ? draft.pts.length < 2 : Math.hypot(draft.b.x - draft.a.x, draft.b.y - draft.a.y) < 4;
    if (!tiny) shapes.push(draft);
    draft = null;
  };
  canvas.addEventListener('pointerup', finish);
  canvas.addEventListener('pointercancel', () => { if (draft?.type !== 'angle') draft = null; });
  canvas.addEventListener('pointerleave', () => { hover = null; });

  // ---- painting
  function paint(ctx) {
    const lw = Math.max(2, canvas.width / 220);
    ctx.save();
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.lineWidth = lw;
    for (const s of shapes) paintShape(ctx, s, lw, null);
    if (draft) paintShape(ctx, draft, lw, hover);
    if (timer) paintTimer(ctx, lw);
    ctx.restore();
  }

  function label(ctx, text, x, y, c, lw) {
    ctx.font = `600 ${Math.round(lw * 7)}px system-ui, sans-serif`;
    const w = ctx.measureText(text).width + lw * 4;
    const hgt = lw * 9;
    ctx.fillStyle = 'rgba(0,0,0,0.65)';
    ctx.fillRect(x, y - hgt / 2, w, hgt);
    ctx.fillStyle = c;
    ctx.textBaseline = 'middle';
    ctx.fillText(text, x + lw * 2, y);
  }

  function paintShape(ctx, s, lw, preview) {
    ctx.strokeStyle = s.color;
    ctx.beginPath();
    if (s.type === 'pen') {
      s.pts.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)));
    } else if (s.type === 'line' || s.type === 'arrow') {
      ctx.moveTo(s.a.x, s.a.y);
      ctx.lineTo(s.b.x, s.b.y);
      if (s.type === 'arrow') {
        const ang = Math.atan2(s.b.y - s.a.y, s.b.x - s.a.x);
        const head = lw * 6;
        for (const d of [-0.45, 0.45]) {
          ctx.moveTo(s.b.x, s.b.y);
          ctx.lineTo(s.b.x - head * Math.cos(ang + d), s.b.y - head * Math.sin(ang + d));
        }
      }
    } else if (s.type === 'circle') {
      ctx.arc(s.a.x, s.a.y, Math.hypot(s.b.x - s.a.x, s.b.y - s.a.y), 0, Math.PI * 2);
    } else if (s.type === 'angle') {
      const pts = preview && s.pts.length < 3 ? [...s.pts, preview] : s.pts;
      pts.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)));
      ctx.stroke();
      for (const p of s.pts) { ctx.beginPath(); ctx.arc(p.x, p.y, lw * 1.5, 0, Math.PI * 2); ctx.fillStyle = s.color; ctx.fill(); }
      if (pts.length === 3) {
        const [a, b, c] = pts;
        const deg = angleAt(a, b, c);
        const r = lw * 10;
        ctx.beginPath();
        let a1 = Math.atan2(a.y - b.y, a.x - b.x);
        let a2 = Math.atan2(c.y - b.y, c.x - b.x);
        let diff = a2 - a1;
        while (diff > Math.PI) diff -= 2 * Math.PI;
        while (diff < -Math.PI) diff += 2 * Math.PI;
        ctx.arc(b.x, b.y, r, a1, a1 + diff, diff < 0);
        ctx.stroke();
        label(ctx, `${Math.round(deg)}°`, b.x + r * 1.2, b.y - r * 1.2, s.color, lw);
      }
      return;
    }
    ctx.stroke();
  }

  function paintTimer(ctx, lw) {
    const end = timer.stop ?? now();
    label(ctx, `⏱ ${Math.max(0, end - timer.start).toFixed(2)} s`, lw * 4, lw * 8, timer.stop == null ? '#ffffff' : '#ffd400', lw);
  }

  // ---- the toolbar
  const toolBtns = TOOLS.map(([k, icon, name]) => h('button', {
    type: 'button', class: `small ${k === tool ? 'on' : ''}`, title: name, 'aria-label': name, 'aria-pressed': String(k === tool),
    onclick: () => setTool(k),
  }, icon));
  function setTool(k) {
    tool = k;
    draft = null;
    canvas.classList.toggle('drawing', k !== 'none');
    toolBtns.forEach((b, i) => { const on = TOOLS[i][0] === k; b.classList.toggle('on', on); b.setAttribute('aria-pressed', String(on)); });
  }
  const swatches = COLORS.map(([c, name]) => h('button', {
    type: 'button', class: `small swatch ${c === color ? 'on' : ''}`, style: `background:${c}`, 'aria-label': `${name} ink`, title: `${name} ink`,
    onclick: (e) => { color = c; swatches.forEach((b) => b.classList.toggle('on', b === e.currentTarget)); },
  }));
  const timerBtn = h('button', {
    type: 'button', class: 'small', title: 'Timer: start, stop, then clear', 'aria-label': 'Timer',
    onclick: () => {
      if (!timer) timer = { start: now(), stop: null };
      else if (timer.stop == null) timer.stop = now();
      else timer = null;
      timerBtn.textContent = !timer ? '⏱ Timer' : timer.stop == null ? '⏱ Stop' : '⏱ Clear';
    },
  }, '⏱ Timer');
  const toolbar = h('div', { class: 'draw-bar', role: 'toolbar', 'aria-label': 'Drawing tools' },
    toolBtns, h('span', { class: 'sep' }), swatches, h('span', { class: 'sep' }), timerBtn,
    h('button', { type: 'button', class: 'small', 'aria-label': 'Undo the last mark', onclick: () => (draft ? (draft = null) : shapes.pop()) }, '↶ Undo'),
    h('button', { type: 'button', class: 'small', 'aria-label': 'Clear all marks', onclick: () => clear() }, 'Clear'));

  function clear() { shapes.length = 0; draft = null; }

  return { toolbar, paint, clear, setTool, get count() { return shapes.length; } };
}
