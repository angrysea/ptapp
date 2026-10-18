// API client with offline support:
//  * get(path, {cache:true}) falls back to the last good response when offline
//  * enqueue() persists a write to the outbox and replays it when online

import cfg from './config.js';
import { devMode, devUser, idToken } from './auth.js';
import { kv, outbox } from './idb.js';

export class ApiError extends Error {
  constructor(status, message, body) { super(message); this.status = status; this.body = body; }
}

export async function api(method, path, body) {
  const headers = { 'content-type': 'application/json' };
  if (devMode()) headers['x-dev-user'] = devUser();
  else {
    const t = await idToken();
    if (!t) throw new ApiError(401, navigator.onLine ? 'Session expired; please sign in again.' : 'You are offline.');
    headers.authorization = `Bearer ${t}`;
  }
  const res = await fetch(cfg.apiUrl + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await res.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { /* plain-text error body */ }
  if (!res.ok) throw new ApiError(res.status, data?.error || text || res.statusText, data);
  return data;
}

const isNetwork = (e) => e instanceof TypeError || (e instanceof ApiError && e.status === 401 && !navigator.onLine);

export async function get(path, { cache = false } = {}) {
  try {
    const data = await api('GET', path);
    if (cache) kv.set(`get:${path}`, data).catch(() => {});
    return data;
  } catch (e) {
    if (cache && isNetwork(e)) {
      const hit = await kv.get(`get:${path}`);
      if (hit !== undefined) return hit;
    }
    throw e;
  }
}

export const post = (p, b = {}) => api('POST', p, b);
export const put = (p, b) => api('PUT', p, b);
export const del = (p) => api('DELETE', p);

// ------------------------------------------------------------ outbox

const listeners = new Set();
export const onSync = (fn) => listeners.add(fn);
const notify = async () => { const n = (await outbox.all()).length; listeners.forEach((fn) => fn(n)); };

/** Queue a write. Pass `key` to dedupe (a newer write with the same key replaces
 * the old one, e.g. `log:{id}`); the default key is unique so same-path writes
 * (chat, video) never silently replace each other. */
export async function enqueue(method, path, body, key = null) {
  await outbox.put({ key: key ?? `${method} ${path} ${Date.now()}-${Math.random().toString(36).slice(2)}`, method, path, body });
  await notify();
  flush();
}

let flushing = false;
export async function flush() {
  if (flushing || !navigator.onLine) return notify();
  flushing = true;
  try {
    for (const item of await outbox.all()) {
      try {
        await api(item.method, item.path, item.body);
        // Only drop the queued write if it is still the one we just sent; if
        // the user re-queued this key while the request was in flight, keep
        // the newer item so it replays on the next flush.
        const cur = await outbox.get(item.key);
        if (cur?.seq === item.seq) await outbox.del(item.key);
      } catch (e) {
        if (isNetwork(e)) break; // offline: stop, everything retries later
        if (e instanceof ApiError && (e.status >= 500 || e.status === 401)) continue; // retry this write later, don't stall the queue
        // 409 = server already has a newer version; other 4xx will never succeed.
        console.warn('dropping queued write', item, e);
        await outbox.del(item.key);
        listeners.forEach((fn) => fn(null, { item, error: e }));
      }
    }
  } finally {
    flushing = false;
    await notify();
  }
}

window.addEventListener('online', flush);
setInterval(flush, 30_000);
