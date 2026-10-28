// Video uploads that survive lost connections, reloads and closed apps
// (VS-3): clients' set videos, and coaches' replies to them (VR-2, `kind:
// 'reply'`). A video is saved on the device first (IndexedDB, with the parts
// already sent), then uploaded part by part whenever there's a connection;
// each part's receipt (ETag) is saved as it lands, so a resumed upload starts
// at the next part. The server side is crates/api/src/routes/videos.rs and
// replies.rs. In a record, `videoId` is the id of what's being uploaded (the
// set video, or the reply); a reply's `replyTo` is the set video it answers.

import { ApiError, post } from './api.js';
import { uploads } from './idb.js';
import { session } from './session.js';
import { onCellular, putPart, videoType } from './videos.js';

const POSTER_URL_TTL_MS = 14 * 60 * 1000; // presigned URLs last 15 minutes
const MAX_ATTEMPTS = 6; // restarts before an upload is marked failed

const listeners = new Set();
const progress = {}; // upload key → fraction sent, kept in memory only
let running = false;

/** Calls `fn(list)` with this user's pending uploads whenever they change. Returns an unsubscribe. */
export function onUploads(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** This user's uploads that haven't finished, each with `progress` (0–1) once it has started. */
export async function pendingUploads() {
  const me = session.userId;
  const list = me ? (await uploads.all()).filter((u) => u.owner === me) : [];
  return list.map((u) => ({ ...u, progress: progress[u.key] ?? null }));
}

async function notify() {
  const list = await pendingUploads();
  listeners.forEach((fn) => fn(list));
}
let soon = null;
const notifySoon = () => { soon ||= setTimeout(() => { soon = null; notify(); }, 250); };

const save = (r) => uploads.put(r);

/**
 * Saves a clip for upload as the video of one logged set and starts sending
 * it if there's a connection. `file` is the converted clip (with its `poster`)
 * or, when the device couldn't convert, the original with `trim` points.
 */
export async function queueVideo({ clientId, logId, set, file, durationSec, trim, poster, waitForWifi = false }) {
  const owner = session.userId;
  const r = {
    key: `${owner}|${crypto.randomUUID()}`, owner, clientId, logId,
    set: { itemId: set.itemId, setIndex: set.setIndex, exerciseId: set.exerciseId, exerciseName: set.exerciseName },
    blob: file, contentType: videoType(file), size: file.size, durationSec: durationSec ?? null, trim: trim || null,
    poster: poster || null, posterTarget: null, waitForWifi,
    videoId: null, partSize: null, parts: [], etags: {},
    state: 'waiting', error: null, attempts: 0, createdAt: new Date().toISOString(),
  };
  await save(r);
  // Ask the browser not to evict queued clips when storage runs low.
  navigator.storage?.persist?.().catch(() => {});
  notify();
  kick();
  return r.key;
}

/** Saves a coach's recorded reply to a client's set video for upload and starts sending it. */
export async function queueReply({ clientId, replyTo, file, durationSec, note, reviewMs, label, waitForWifi = false }) {
  const owner = session.userId;
  const r = {
    key: `${owner}|${crypto.randomUUID()}`, owner, kind: 'reply', clientId, replyTo, label: label || 'Reply',
    blob: file, contentType: videoType(file), size: file.size, durationSec: durationSec ?? null,
    note: note || '', reviewMs: reviewMs ?? null, poster: null, posterTarget: null, waitForWifi,
    videoId: null, partSize: null, parts: [], etags: {},
    state: 'waiting', error: null, attempts: 0, createdAt: new Date().toISOString(),
  };
  await save(r);
  navigator.storage?.persist?.().catch(() => {});
  notify();
  kick();
  return r.key;
}

/** The API path of the thing being uploaded. */
const base = (r) => (r.kind === 'reply' ? `/clients/${r.clientId}/replies/${r.videoId}` : `/clients/${r.clientId}/videos/${r.videoId}`);

export async function retryUpload(key) {
  const r = await uploads.get(key);
  if (!r) return;
  Object.assign(r, { state: 'waiting', error: null, attempts: 0 });
  await save(r);
  notify();
  kick();
}

/** Drops a pending upload from this device. A half-sent upload is left for the server's cleanup. */
export async function removeUpload(key) {
  await uploads.del(key);
  delete progress[key];
  notify();
}

/** Starts the upload from scratch next time (the server lost or never finished it). */
function restart(r) {
  Object.assign(r, { videoId: null, partSize: null, parts: [], etags: {}, posterTarget: null });
}

async function create(r) {
  const created = r.kind === 'reply'
    ? await post(`/clients/${r.clientId}/videos/${r.replyTo}/replies`, {
      contentType: r.contentType, size: r.size, durationSec: r.durationSec, note: r.note, reviewMs: r.reviewMs,
    })
    : await post(`/clients/${r.clientId}/videos`, {
      logId: r.logId, itemId: r.set.itemId, setIndex: r.set.setIndex, exerciseId: r.set.exerciseId,
      exerciseName: r.set.exerciseName, contentType: r.contentType, size: r.size, durationSec: r.durationSec,
      trimStart: r.trim ? r.trim.start : null, trimEnd: r.trim ? r.trim.end : null,
      posterSize: r.poster ? r.poster.size : null,
    });
  const made = created.video || created.reply;
  r.videoId = made.id;
  r.partSize = made.partSize;
  r.parts = created.parts.map((p) => ({ n: p.n, size: p.size }));
  r.etags = {};
  r.posterTarget = created.poster ? { ...created.poster, at: Date.now() } : null;
  await save(r);
  return Object.fromEntries(created.parts.map((p) => [p.n, p]));
}

async function freshUrls(r, numbers) {
  const res = await post(`${base(r)}/parts`, { parts: numbers });
  return Object.fromEntries(res.parts.map((p) => [p.n, p]));
}

async function send(r) {
  r.state = 'uploading';
  r.attempts += 1;
  await save(r);
  notify();
  let urls = r.videoId ? {} : await create(r);
  const missing = r.parts.filter((p) => !r.etags[p.n]);
  if (missing.some((p) => !urls[p.n])) urls = { ...urls, ...(await freshUrls(r, missing.map((p) => p.n))) };
  const sentBefore = () => r.parts.filter((p) => r.etags[p.n]).reduce((sum, p) => sum + p.size, 0);
  for (const p of missing) {
    const start = (p.n - 1) * r.partSize;
    const body = r.blob.slice(start, start + p.size);
    const onBytes = (b) => { progress[r.key] = (sentBefore() + b) / r.size; notifySoon(); };
    let etag;
    try {
      etag = await putPart(urls[p.n].url, urls[p.n].headers, body, onBytes);
    } catch (e) {
      if (e.status === 403) { // the URL expired while this was offline: get a new one and try once more
        urls = { ...urls, ...(await freshUrls(r, [p.n])) };
        etag = await putPart(urls[p.n].url, urls[p.n].headers, body, onBytes);
      } else if (e.status === 404) { // S3 already discarded the unfinished upload: begin again
        restart(r);
        await save(r);
        throw Object.assign(new Error('upload expired; starting again'), { retry: true });
      } else throw e;
    }
    r.etags[p.n] = etag;
    await save(r);
  }
  // The poster's URL can't be renewed; after a long gap the video just goes without one.
  if (r.poster && r.posterTarget && Date.now() - r.posterTarget.at < POSTER_URL_TTL_MS) {
    await fetch(r.posterTarget.url, { method: 'PUT', headers: r.posterTarget.headers, body: r.poster }).catch(() => {});
  }
  try {
    await post(`${base(r)}/complete`, { parts: r.parts.map((p) => ({ n: p.n, etag: r.etags[p.n] })) });
  } catch (e) {
    // 400: S3 is missing parts (e.g. the upload was cleaned up); 404: the record is gone.
    if (e instanceof ApiError && (e.status === 400 || e.status === 404)) {
      restart(r);
      await save(r);
      throw Object.assign(new Error('upload incomplete; starting again'), { retry: true });
    }
    throw e;
  }
  await uploads.del(r.key);
  delete progress[r.key];
}

const offlineError = (e) => e?.network || e instanceof TypeError || (e instanceof ApiError && e.status === 401 && !navigator.onLine);

/** Sends this user's pending uploads, one at a time. Safe to call often. */
export async function kick() {
  if (running || !navigator.onLine || !session.userId) return;
  running = true;
  try {
    for (const r of await uploads.all()) {
      if (r.owner !== session.userId || r.state === 'failed') continue;
      if (r.waitForWifi && onCellular()) continue; // the person chose to wait for Wi-Fi
      try {
        await send(r);
      } catch (e) {
        if (offlineError(e)) { r.state = 'waiting'; await save(r); break; } // try again when back online
        const permanent = e instanceof ApiError && e.status >= 400 && e.status < 500 && e.status !== 401;
        if (permanent || r.attempts >= MAX_ATTEMPTS) {
          Object.assign(r, { state: 'failed', error: e.message });
        } else {
          r.state = 'waiting'; // a server hiccup or a restart: the next kick tries again
        }
        await save(r);
      }
    }
  } finally {
    running = false;
    notify();
  }
}

window.addEventListener('online', () => kick());
navigator.connection?.addEventListener?.('change', () => kick()); // e.g. back on Wi-Fi
setInterval(() => kick(), 30_000);
