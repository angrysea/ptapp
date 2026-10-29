// Warm-ups and post-workout cardio (crates/core/src/cardio.rs): block
// phases, items done for time or distance, and heart-rate zones. Distances
// are stored in metres and shown in km or miles by the user's units.

import { h, infoButton, num, round } from './ui.js';

export const PHASES = [['warmup', 'Warm-up'], ['main', 'Main workout'], ['post', 'Post-workout cardio']];
const ORDER = { warmup: 0, main: 1, post: 2 };
export const phaseOf = (b) => b.phase || 'main';
/** Blocks in the order they're done: warm-ups, main work, post-workout. */
export const byPhase = (blocks) => [...blocks].sort((a, b) => ORDER[phaseOf(a)] - ORDER[phaseOf(b)]);

/** % of max heart rate per zone, zone 1 first (same as cardio::ZONES). */
export const ZONES = [[50, 60], [60, 70], [70, 80], [80, 90], [90, 100]];
const ZONE_FEEL = ['Very easy, recovery', 'Easy: you can hold a conversation', 'Moderate: short sentences only', 'Hard: a few words at a time', 'All-out'];

/** Estimated max heart rate (Tanaka: 208 − 0.7 × age), or null without a birth date. */
export function maxHr(birthDate) {
  if (!birthDate) return null;
  const age = (Date.now() - Date.parse(birthDate)) / (365.2425 * 864e5);
  return Math.round(208 - 0.7 * age);
}

/** "Zone 2 (115-131 bpm)" when the birth date is known, else the percentages. */
export function zoneText(zone, birthDate) {
  const [lo, hi] = ZONES[zone - 1];
  const max = maxHr(birthDate);
  return max ? `Zone ${zone} (${Math.round((max * lo) / 100)}-${Math.round((max * hi) / 100)} bpm)` : `Zone ${zone} (${lo}-${hi}% of max heart rate)`;
}

export const zoneOptions = [['', 'No zone'], ...ZONES.map(([lo, hi], i) => [String(i + 1), `Zone ${i + 1} (${lo}-${hi}%)`])];

export const zoneInfo = (birthDate) => {
  const max = maxHr(birthDate);
  return infoButton('heart-rate zones',
    h('p', {}, 'Zones are bands of heart rate, as a share of the maximum. Lower zones build an aerobic base; higher ones are short, hard efforts.'),
    h('table', { class: 'scale' },
      h('tr', {}, h('th', {}, 'Zone'), h('th', {}, max ? 'Heart rate' : '% of max'), h('th', {}, 'How it feels')),
      ZONES.map(([lo, hi], i) => h('tr', {}, h('td', {}, i + 1),
        h('td', {}, max ? `${Math.round((max * lo) / 100)}-${Math.round((max * hi) / 100)} bpm` : `${lo}-${hi}%`), h('td', {}, ZONE_FEEL[i])))),
    h('p', { class: 'muted small' }, max
      ? `Your max is estimated from your age as ${max} bpm. Without a heart-rate monitor, go by how it feels.`
      : 'Max heart rate is estimated from age (208 − 0.7 × age). Without a heart-rate monitor, go by how it feels.'));
};

/** Distance in the user's units: km, or miles in imperial. */
export function distUnits(u) {
  const per = u.imperial ? 1609.344 : 1000;
  return {
    unit: u.imperial ? 'mi' : 'km',
    to: (m) => (m == null ? null : round(m / per, 2)),
    from: (v) => (num(v) == null ? null : Math.round(num(v) * per)),
  };
}

/** "20 min", or "12:30" when it isn't whole minutes. */
export function fmtDuration(sec) {
  if (sec % 60 === 0) return `${sec / 60} min`;
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`;
}

/** What a cardio item asks for: "2 × 10 min · 2 km · Zone 2 (…) · RPE 6". */
export function cardioText(it, u, birthDate) {
  const c = it.cardio;
  const d = distUnits(u);
  const amount = [c.durationSec && fmtDuration(c.durationSec), c.distanceM && `${d.to(c.distanceM)} ${d.unit}`].filter(Boolean).join(' · ');
  return [
    it.sets > 1 ? `${it.sets} × ${amount}` : amount,
    c.zone && zoneText(c.zone, birthDate),
    it.rpe != null && `RPE ${it.rpe}`,
    it.restSec && `rest ${it.restSec}s`,
  ].filter(Boolean).join(' · ');
}

/** A logged cardio set: "20 min · 4 km · 142 bpm". */
export function cardioLogText(s, u) {
  const d = distUnits(u);
  return [s.durationSec != null && fmtDuration(s.durationSec), s.distanceM != null && `${d.to(s.distanceM)} ${d.unit}`,
    s.avgHr != null && `${s.avgHr} bpm`, s.rpe != null && `RPE ${s.rpe}`].filter(Boolean).join(' · ');
}

export const isCardioSet = (s) => s.durationSec != null || s.distanceM != null;
