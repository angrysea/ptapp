// Set videos (VS-1..VS-3): helpers for uploading a clip in parts straight to
// S3 (the upload itself is uploads.js), and playing one back. The server side
// is crates/api/src/routes/videos.rs.

import { h, sheet } from './ui.js';

const TYPES = { mov: 'video/quicktime', mp4: 'video/mp4', m4v: 'video/mp4', webm: 'video/webm' };

/** The file's MIME type; some pickers leave `type` empty, so fall back to the extension. */
export function videoType(file) {
  return file.type || TYPES[((file.name || '').split('.').pop() || '').toLowerCase()] || 'video/mp4';
}

/** Clip length in seconds, or null if this browser can't tell. iOS loads
 * nothing until playback is requested, so a muted play() nudges it. */
export function clipDuration(file) {
  return new Promise((resolve) => {
    const v = document.createElement('video');
    v.muted = true; v.playsInline = true; v.preload = 'metadata';
    const url = URL.createObjectURL(file);
    const done = (d) => { clearTimeout(t); v.pause(); v.removeAttribute('src'); v.load(); URL.revokeObjectURL(url); resolve(d); };
    const t = setTimeout(() => done(null), 8000);
    v.addEventListener('loadedmetadata', () => done(Number.isFinite(v.duration) ? v.duration : null), { once: true });
    v.addEventListener('error', () => done(null), { once: true });
    v.src = url;
    v.play().catch(() => {});
  });
}

/** PUTs one part and returns S3's ETag for it. XHR rather than fetch, for
 * upload progress. Failures carry `status` (HTTP) or `network: true`. */
export function putPart(url, headers, blob, onBytes) {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('PUT', url);
    for (const [k, v] of Object.entries(headers || {})) xhr.setRequestHeader(k, v);
    xhr.upload.onprogress = (e) => onBytes(e.loaded);
    xhr.onload = () => {
      if (xhr.status < 200 || xhr.status >= 300) return reject(Object.assign(new Error(`Upload failed (${xhr.status})`), { status: xhr.status }));
      const etag = xhr.getResponseHeader('ETag');
      // Missing means the bucket's CORS doesn't expose ETag (template.yaml).
      if (!etag) return reject(new Error('Upload finished but the storage server hid its receipt (ETag)'));
      resolve(etag);
    };
    xhr.onerror = () => reject(Object.assign(new Error('Upload interrupted by the connection'), { network: true }));
    xhr.send(blob);
  });
}

/** Uploads bigger than this ask first on mobile data. */
const BIG_ON_CELLULAR = 20 * 1024 * 1024;

/** On mobile data (or in data-saver mode). Only some browsers say (Chrome on
 * Android); elsewhere, e.g. iPhone, this is always false. */
export function onCellular() {
  const c = navigator.connection;
  return !!c && (c.type === 'cellular' || c.saveData === true);
}

/**
 * Before a big upload on mobile data: asks whether to upload now or wait for
 * Wi-Fi. Resolves `{ waitForWifi }`, or null if the person cancelled. Small
 * uploads, and browsers that can't tell, go straight through.
 */
export function askBeforeCellular(bytes) {
  if (bytes < BIG_ON_CELLULAR || !onCellular()) return Promise.resolve({ waitForWifi: false });
  return new Promise((resolve) => {
    let answered = false;
    const answer = (v) => { answered = true; sh.close(); resolve(v); };
    const sh = sheet(
      h('h2', {}, 'You’re on mobile data'),
      h('p', {}, `This video is about ${Math.round(bytes / (1024 * 1024))} MB. Upload it now, or keep it on this device and send it when you’re on Wi-Fi?`),
      h('button', { class: 'primary block', onclick: () => answer({ waitForWifi: true }) }, 'Wait for Wi-Fi'),
      h('button', { class: 'block', onclick: () => answer({ waitForWifi: false }) }, 'Upload now'),
      h('button', { class: 'ghost block', onclick: () => answer(null) }, 'Cancel'),
    );
    sh.onClose = () => { if (!answered) resolve(null); };
  });
}

/** Plays a coach's reply (`url` for a recording, `note`) in a sheet. `onPlay`
 * runs the first time it starts playing (the client marks it seen). */
export function playReply(r, { title = 'Coach’s reply', onPlay } = {}) {
  const player = r.url && h('video', { src: r.url, controls: true, playsinline: true, preload: 'metadata', class: 'player' });
  let played = false;
  const first = () => { if (!played) { played = true; onPlay?.(); } };
  player?.addEventListener('play', first);
  const sh = sheet(
    h('h2', {}, title),
    player,
    r.note && h('p', { style: 'white-space:pre-wrap' }, r.note),
    h('button', { class: 'block', onclick: () => sh.close() }, 'Close'),
  );
  if (!player) first(); // a written reply is seen once opened
  sh.onClose = () => player?.pause();
  return sh;
}

/** Plays a listed video (with `url`, and `posterUrl` when there is one) in a
 * sheet, keeping to its trim points when it has them. `actions` are extra
 * buttons shown above Close. Returns the sheet. */
export function playVideo(v, ...actions) {
  const player = h('video', { src: v.url, poster: v.posterUrl || null, controls: true, playsinline: true, preload: 'metadata', class: 'player' });
  const trimmed = v.trimStart != null && v.trimEnd != null;
  if (trimmed) {
    const [a, b] = [v.trimStart, v.trimEnd];
    player.addEventListener('loadedmetadata', () => { player.currentTime = a; });
    player.addEventListener('timeupdate', () => { if (player.currentTime >= b) { player.pause(); player.currentTime = a; } });
    player.addEventListener('seeking', () => {
      if (player.currentTime < a - 0.05 || player.currentTime > b) player.currentTime = Math.min(Math.max(player.currentTime, a), b);
    });
  }
  const sh = sheet(
    h('h2', {}, `${v.exerciseName || 'Video'} · set ${v.setIndex + 1}`),
    player,
    trimmed && h('p', { class: 'small muted' }, `Plays ${v.trimStart.toFixed(1)}–${v.trimEnd.toFixed(1)} s of the original.`),
    actions,
    h('button', { class: 'block', onclick: () => sh.close() }, 'Close'),
  );
  sh.onClose = () => player.pause();
  return sh;
}

// ------------------------------------------------------------ filming guidance (VS-5)

export const ANGLES = [['side', 'From the side'], ['diagonal', 'From 45°, between side and front'], ['front', 'From the front'], ['behind', 'From behind']];

/** How to film a set of `exercise`, for the video sheet; generic tips when it has none. */
export function filmingTip(exercise, profile) {
  const f = exercise?.filming;
  const imperial = profile?.units === 'imperial';
  const dist = f?.distanceM != null && (imperial ? `about ${Math.round(f.distanceM * 3.281)} ft away` : `about ${f.distanceM} m away`);
  const angle = f && ANGLES.find(([k]) => k === f.angle)?.[1];
  return h('div', { class: 'film-tip' },
    h('strong', {}, '📐 How to film'),
    h('ul', {},
      f && h('li', {}, [angle, dist].filter(Boolean).join(', ')),
      f?.notes && h('li', {}, f.notes),
      h('li', {}, 'Whole body and the weight in frame for the whole set; prop the phone up so it stays still.')));
}
