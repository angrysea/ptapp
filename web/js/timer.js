// LG-2: rest timer and interval timers (EMOM, AMRAP, work/rest intervals,
// the lines of a flow or the stages of a ramp-up) and a stopwatch for "for
// time" blocks.
// Runs entirely on the device, so it works offline. Keeps the screen awake
// while running where the Wake Lock API is available.

import { h, sheet } from './ui.js';

let audio;
function beep(freq = 880, ms = 180) {
  try {
    audio ||= new (window.AudioContext || window.webkitAudioContext)();
    const o = audio.createOscillator();
    const g = audio.createGain();
    o.frequency.value = freq;
    g.gain.value = 0.15;
    o.connect(g).connect(audio.destination);
    o.start();
    o.stop(audio.currentTime + ms / 1000);
  } catch { /* audio unavailable */ }
  navigator.vibrate?.(ms);
}

const mmss = (s) => `${Math.floor(Math.max(s, 0) / 60)}:${String(Math.max(Math.ceil(s), 0) % 60).padStart(2, '0')}`;

async function wakeLock() {
  try { return await navigator.wakeLock?.request('screen'); } catch { return null; }
}

/** Wake lock that releases even if the request resolves after the timer was
 * closed, and re-acquires when the page becomes visible again (Chrome drops
 * the lock when the screen turns off). */
function keepAwake() {
  const c = { lock: null, closed: false };
  const onVis = (e) => { if (!e.hidden && !c.closed) wakeLock().then((l) => { c.lock = l; }); };
  document.addEventListener('visibilitychange', onVis);
  wakeLock().then((l) => { c.lock = l; if (c.closed) l?.release?.(); });
  c.stop = () => { c.closed = true; document.removeEventListener('visibilitychange', onVis); c.lock?.release?.(); };
  return c;
}

/** Countdown timer shown as a bottom sheet: rest between sets, or a timed
 * piece of cardio (`kind` 'work', labelled with the exercise). */
export function restTimer(seconds, label = 'Rest', kind = 'rest') {
  let end = Date.now() + seconds * 1000;
  const big = h('div', { class: 'timer-big' }, mmss(seconds));
  const lock = keepAwake();
  const tick = setInterval(() => {
    const left = (end - Date.now()) / 1000;
    big.textContent = mmss(left);
    if (left <= 3 && left > 0 && Math.abs(left - Math.round(left)) < 0.13) beep(660, 80);
    if (left <= 0) { beep(990, 400); close(); }
  }, 250);
  const s = sheet(
    h('div', { class: `timer-phase ${kind}` }, label),
    big,
    h('div', { class: 'row', style: 'justify-content:center' },
      h('button', { onclick: () => { end -= 15000; } }, '−15s'),
      h('button', { onclick: () => { end += 15000; } }, '+15s'),
      h('button', { class: 'primary', onclick: () => close() }, kind === 'rest' ? 'Skip' : 'Stop')),
  );
  function close() { clearInterval(tick); lock.stop(); s.close(); }
}

/** Seconds a line takes: its cardio time, or reps written as a time
 * ("30s", "45 sec", "1 min", "1:30"); null when it's counted in reps. */
export function lineSeconds(it) {
  if (it.cardio?.durationSec) return it.cardio.durationSec;
  const r = String(it.reps || '').trim().toLowerCase();
  let m = r.match(/^(\d+):(\d{2})$/);
  if (m) return Number(m[1]) * 60 + Number(m[2]);
  m = r.match(/^(\d+(?:\.\d+)?)\s*(s|sec|secs|seconds?|m|min|mins|minutes?)$/);
  if (!m) return null;
  return Math.round(Number(m[1]) * (m[2].startsWith('m') ? 60 : 1));
}

/** The timed steps of a flow (each line, for each round) or a ramp-up
 * (each stage); empty when none of its lines has a time. */
export function lineSteps(block) {
  const lines = block.items.map((it, i) => ({ it, i, sec: lineSeconds(it) })).filter((x) => x.sec);
  if (!lines.length) return [];
  const rounds = block.kind === 'flow' ? block.rounds || 1 : 1;
  const steps = [];
  for (let r = 1; r <= rounds; r++) {
    for (const { it, i, sec } of lines) {
      const label = block.kind === 'ramp' ? `Stage ${i + 1} · ${it.exerciseName}` : it.exerciseName;
      steps.push({ label: rounds > 1 ? `${label} · round ${r} of ${rounds}` : label, kind: 'work', sec });
    }
  }
  return steps;
}

/** Whether `blockTimer` has something to time for this block. */
export const hasBlockTimer = (b) => ['emom', 'amrap', 'interval', 'circuit', 'superset'].includes(b.kind) || (['flow', 'ramp'].includes(b.kind) && lineSteps(b).length > 0);

/**
 * Interval timer for a workout block. Builds a list of phases from the block
 * kind and steps through them with beeps.
 */
export function blockTimer(block) {
  const phases = [];
  const kind = block.kind;
  if (kind === 'flow' || kind === 'ramp') {
    phases.push(...lineSteps(block));
  } else if (kind === 'emom') {
    const n = block.rounds || Math.max(1, Math.round((block.durationSec || 600) / 60));
    for (let i = 1; i <= n; i++) phases.push({ label: `Minute ${i} of ${n}`, kind: 'work', sec: 60 });
  } else if (kind === 'amrap') {
    phases.push({ label: 'AMRAP', kind: 'work', sec: block.durationSec || 600 });
  } else {
    const rounds = block.rounds || 3;
    const work = block.workSec || 40;
    const rest = block.restSec ?? 20;
    for (let i = 1; i <= rounds; i++) {
      phases.push({ label: `Round ${i} of ${rounds} · work`, kind: 'work', sec: work });
      if (rest > 0 && i < rounds) phases.push({ label: `Round ${i} of ${rounds} · rest`, kind: 'rest', sec: rest });
    }
  }
  phases.unshift({ label: 'Get ready', kind: 'rest', sec: 10 });

  let idx = 0;
  let end = 0;
  let pausedLeft = null;
  let rounds = 0;
  const phaseEl = h('div', { class: 'timer-phase' });
  const big = h('div', { class: 'timer-big' });
  const roundsEl = h('span', {}, '0');
  const pauseBtn = h('button', { onclick: togglePause }, 'Pause');

  function enter(i) {
    idx = i;
    pausedLeft = null; // entering a new phase discards a paused countdown
    const p = phases[i];
    end = Date.now() + p.sec * 1000;
    phaseEl.textContent = p.label;
    phaseEl.className = `timer-phase ${p.kind}`;
    beep(p.kind === 'work' ? 990 : 660, 300);
  }
  function togglePause() {
    if (pausedLeft == null) { pausedLeft = end - Date.now(); pauseBtn.textContent = 'Resume'; }
    else { end = Date.now() + pausedLeft; pausedLeft = null; pauseBtn.textContent = 'Pause'; }
  }
  const tick = setInterval(() => {
    if (pausedLeft != null) return;
    const left = (end - Date.now()) / 1000;
    big.textContent = mmss(left);
    if (left <= 3 && left > 0 && Math.abs(left - Math.round(left)) < 0.13) beep(660, 80);
    if (left <= 0) {
      if (idx + 1 < phases.length) enter(idx + 1);
      else { beep(990, 700); phaseEl.textContent = 'Done!'; big.textContent = '0:00'; clearInterval(tick); }
    }
  }, 250);

  const s = sheet(
    h('h3', { class: 'center' }, { flow: 'FLOW', ramp: 'RAMP-UP' }[kind] ?? kind.toUpperCase()),
    phaseEl,
    big,
    kind === 'amrap' && h('div', { class: 'row', style: 'justify-content:center' },
      h('span', { class: 'muted' }, 'Rounds:'), h('span', { class: 'stat' }, roundsEl),
      h('button', { onclick: () => { roundsEl.textContent = String(++rounds); } }, '+1 round')),
    h('div', { class: 'row', style: 'justify-content:center' },
      pauseBtn,
      h('button', { onclick: () => idx + 1 < phases.length && enter(idx + 1) }, 'Next'),
      h('button', { class: 'primary', onclick: close }, 'Close')),
  );
  const lock = keepAwake();
  enter(0);
  function close() { clearInterval(tick); lock.stop(); s.close(); }
}

/** Stopwatch for a "for time" block: counts up, beeps at the cap if there is
 * one, and hands the time to `onStop` (seconds) when stopped. */
export function stopwatch(label, capSec, onStop) {
  let start = Date.now();
  let pausedAt = null;
  const big = h('div', { class: 'timer-big' }, '0:00');
  const capEl = capSec ? h('div', { class: 'muted center' }, `Time cap ${mmss(capSec)}`) : null;
  const lock = keepAwake();
  let capped = false;
  const elapsed = () => ((pausedAt ?? Date.now()) - start) / 1000;
  const tick = setInterval(() => {
    const t = elapsed();
    big.textContent = mmss(Math.floor(t));
    if (capSec && !capped && t >= capSec) { capped = true; beep(990, 700); }
  }, 250);
  const pauseBtn = h('button', {
    onclick: () => {
      if (pausedAt == null) { pausedAt = Date.now(); pauseBtn.textContent = 'Resume'; }
      else { start += Date.now() - pausedAt; pausedAt = null; pauseBtn.textContent = 'Pause'; }
    },
  }, 'Pause');
  beep(990, 300);
  const s = sheet(
    h('div', { class: 'timer-phase work' }, label),
    big,
    capEl,
    h('div', { class: 'row', style: 'justify-content:center' },
      pauseBtn,
      h('button', { onclick: close }, 'Cancel'),
      h('button', { class: 'primary', onclick: () => { const t = Math.max(1, Math.round(elapsed())); close(); onStop?.(capSec ? Math.min(t, capSec) : t); } }, 'Done')),
  );
  s.onClose = () => { clearInterval(tick); lock.stop(); };
  function close() { s.close(); }
}
