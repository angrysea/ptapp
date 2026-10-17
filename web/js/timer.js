// LG-2: rest timer and interval timers (EMOM, AMRAP, work/rest intervals).
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

/** Countdown rest timer shown as a bottom sheet. */
export function restTimer(seconds) {
  let end = Date.now() + seconds * 1000;
  const big = h('div', { class: 'timer-big' }, mmss(seconds));
  let lock;
  wakeLock().then((l) => { lock = l; });
  const tick = setInterval(() => {
    const left = (end - Date.now()) / 1000;
    big.textContent = mmss(left);
    if (left <= 3 && left > 0 && Math.abs(left - Math.round(left)) < 0.13) beep(660, 80);
    if (left <= 0) { beep(990, 400); close(); }
  }, 250);
  const s = sheet(
    h('div', { class: 'timer-phase rest' }, 'Rest'),
    big,
    h('div', { class: 'row', style: 'justify-content:center' },
      h('button', { onclick: () => { end -= 15000; } }, '−15s'),
      h('button', { onclick: () => { end += 15000; } }, '+15s'),
      h('button', { class: 'primary', onclick: () => close() }, 'Skip')),
  );
  function close() { clearInterval(tick); lock?.release?.(); s.close(); }
}

/**
 * Interval timer for a workout block. Builds a list of phases from the block
 * kind and steps through them with beeps.
 */
export function blockTimer(block) {
  const phases = [];
  const kind = block.kind;
  if (kind === 'emom') {
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
  let lock;
  const phaseEl = h('div', { class: 'timer-phase' });
  const big = h('div', { class: 'timer-big' });
  const roundsEl = h('span', {}, '0');
  const pauseBtn = h('button', { onclick: togglePause }, 'Pause');

  function enter(i) {
    idx = i;
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
    h('h3', { class: 'center' }, kind.toUpperCase()),
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
  wakeLock().then((l) => { lock = l; });
  enter(0);
  function close() { clearInterval(tick); lock?.release?.(); s.close(); }
}
