// Trim (VS-2): pick the part of a clip to keep, at most 60 seconds. Two
// range inputs share one track (start and end handles); moving one shows
// that frame, and Play loops the selection.

import { h } from '../ui.js';

export const MAX_CLIP_SECONDS = 60;

const round1 = (s) => Math.round(s * 10) / 10;
const clock = (s) => {
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(1).padStart(4, '0')}`;
};

/**
 * A trim control for `file`. `duration` (seconds) is used when the browser
 * can't read the clip's length itself, as with some in-app recordings.
 * Returns:
 *  - `el`: the control to mount;
 *  - `ready`: resolves with the clip's length in seconds, or null if this
 *    browser can't read it (the clip then uploads whole);
 *  - `range()`: the current `{ start, end, duration }` in seconds;
 *  - `destroy()`: stops playback and frees the preview.
 */
export function trimmer(file, { duration: known } = {}) {
  const url = URL.createObjectURL(file);
  const video = h('video', { class: 'player', playsinline: true, preload: 'metadata' });
  video.muted = true; // lets iOS load the clip without a tap; Play unmutes
  const startIn = h('input', { type: 'range', 'aria-label': 'Start', min: 0, max: 0, step: 0.1, value: 0 });
  const endIn = h('input', { type: 'range', 'aria-label': 'End', min: 0, max: 0, step: 0.1, value: 0 });
  const sel = h('div', { class: 'trim-sel' });
  const readout = h('span', { class: 'small', 'aria-live': 'polite' }, 'Loading the clip…');
  const play = h('button', { class: 'small', type: 'button', disabled: true }, '▶ Play');
  const range = h('div', { class: 'trim-range' }, h('div', { class: 'trim-track' }, sel), startIn, endIn);
  const el = h('div', { class: 'trim' }, video, range, h('div', { class: 'spread' }, readout, play));

  let duration = 0, start = 0, end = 0, min = 1;

  /** Keeps start < end, at least `min` apart and at most 60 s apart; the handle not being moved gives way. */
  function apply(moved) {
    let s = Number(startIn.value), e = Number(endIn.value);
    if (moved === 'start') {
      s = Math.min(s, duration - min);
      e = Math.min(Math.max(e, s + min), s + MAX_CLIP_SECONDS);
    } else {
      e = Math.max(e, min);
      s = Math.max(Math.min(s, e - min), e - MAX_CLIP_SECONDS);
    }
    start = Math.max(0, round1(s));
    end = Math.min(duration, round1(e));
    startIn.value = String(start);
    endIn.value = String(end);
    sel.style.left = `${(start / duration) * 100}%`;
    sel.style.right = `${100 - (end / duration) * 100}%`;
    readout.textContent = `${clock(start)} – ${clock(end)} · ${(end - start).toFixed(1)} s`
      + (duration > MAX_CLIP_SECONDS ? ` of ${Math.round(duration)} s (60 s max)` : '');
  }

  for (const [input, which] of [[startIn, 'start'], [endIn, 'end']]) {
    input.addEventListener('input', () => {
      apply(which);
      video.pause();
      play.textContent = '▶ Play';
      video.currentTime = which === 'start' ? start : end;
    });
  }
  play.addEventListener('click', () => {
    if (!video.paused) { video.pause(); play.textContent = '▶ Play'; return; }
    video.muted = false;
    if (video.currentTime < start || video.currentTime >= end - 0.05) video.currentTime = start;
    video.play().catch(() => {});
    play.textContent = '❚❚ Pause';
  });
  // Loop within the selection.
  video.addEventListener('timeupdate', () => {
    if (!video.paused && video.currentTime >= end) video.currentTime = start;
  });

  const ready = new Promise((resolve) => {
    const fail = () => { clearTimeout(t); range.hidden = true; play.hidden = true; readout.textContent = 'This clip can’t be previewed here; it will upload whole.'; resolve(null); };
    const t = setTimeout(fail, 10000);
    video.addEventListener('loadedmetadata', () => {
      clearTimeout(t);
      const d = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : known;
      if (!d) return fail();
      video.pause();
      video.currentTime = 0;
      duration = round1(d);
      min = Math.min(1, duration);
      for (const input of [startIn, endIn]) input.max = String(duration);
      startIn.value = '0';
      endIn.value = String(Math.min(MAX_CLIP_SECONDS, duration));
      apply('end');
      play.disabled = false;
      resolve(duration);
    }, { once: true });
    video.addEventListener('error', fail, { once: true });
    video.src = url;
    video.play().catch(() => {}); // iOS loads nothing until playback is requested
  });

  return {
    el,
    ready,
    range: () => ({ start, end, duration }),
    destroy() { video.pause(); video.removeAttribute('src'); video.load(); URL.revokeObjectURL(url); },
  };
}
