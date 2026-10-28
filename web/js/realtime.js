// Live updates (MS-1). The API publishes an event to the recipient's inbox
// channel on AppSync Events (template.yaml EventApi) when a message arrives;
// open views refresh when they hear one. Events are only hints: the API is
// the source of truth. Without a connection (local development, a blocked
// network, a failed sign-in), views are refreshed by polling instead, and
// always when the app comes back to the foreground.

import cfg from './config.js';
import { idToken } from './auth.js';

const POLL_MS = 10_000;
const listeners = new Set();
let connected = false;

/** Calls `fn(event)` on each live event or poll tick; returns an unsubscribe. */
export function onLive(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function emit(event) {
  for (const fn of [...listeners]) {
    try { fn(event); } catch (e) { console.error(e); }
  }
}

const b64url = (s) => btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

async function connect(userId, attempt = 0) {
  const token = await idToken().catch(() => null);
  if (!token) return retry(userId, attempt);
  const auth = { host: cfg.eventsHttp, Authorization: token };
  let ws;
  try {
    ws = new WebSocket(`wss://${cfg.eventsRealtime}/event/realtime`, ['aws-appsync-event-ws', `header-${b64url(JSON.stringify(auth))}`]);
  } catch {
    return retry(userId, attempt);
  }
  ws.onopen = () => ws.send(JSON.stringify({ type: 'connection_init' }));
  ws.onmessage = (m) => {
    let msg;
    try { msg = JSON.parse(m.data); } catch { return; }
    if (msg.type === 'connection_ack') {
      ws.send(JSON.stringify({ type: 'subscribe', id: crypto.randomUUID(), channel: `/inbox/${userId}`, authorization: auth }));
    } else if (msg.type === 'subscribe_success') {
      connected = true;
      attempt = 0;
      emit({ type: 'connected' }); // catch up on anything missed while disconnected
    } else if (msg.type === 'data') {
      let event = {};
      try { event = JSON.parse(msg.event); } catch { /* still a nudge to refresh */ }
      emit(event);
    } else if (msg.type === 'subscribe_error' || msg.type === 'connection_error') {
      ws.close();
    }
  };
  ws.onclose = () => {
    connected = false;
    retry(userId, attempt + 1);
  };
}

function retry(userId, attempt) {
  // Back off up to five minutes; polling covers the gap.
  setTimeout(() => connect(userId, attempt), Math.min(300_000, 2_000 * 2 ** attempt));
}

/** Starts live updates for the signed-in user; call once after sign-in. */
export function startLive(userId, { realtime = true } = {}) {
  if (realtime && cfg.eventsRealtime && cfg.eventsHttp && 'WebSocket' in window) connect(userId);
  setInterval(() => {
    if (!connected && document.visibilityState === 'visible') emit({ type: 'poll' });
  }, POLL_MS);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') emit({ type: 'poll' });
  });
}
