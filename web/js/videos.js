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
