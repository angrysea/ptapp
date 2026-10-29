// Coach time per client (the Phase 2 exit gate). While a coach is on one of
// a client's pages (#/c/{id}…, #/review/{id}/…), the tab is visible and they
// have touched or typed in the last minute, each second counts toward that
// client. Totals are posted about once a minute and when the tab is hidden.

import { api } from './api.js';

const TICK_S = 5;
const IDLE_S = 60;
const SEND_EVERY_S = 60;

let pending = {}; // clientId -> seconds not yet sent
let lastInput = Date.now();
let sinceSend = 0;

function clientOnScreen() {
  const m = location.hash.match(/^#\/(?:c|review)\/([^/?]+)/);
  return m ? decodeURIComponent(m[1]) : null;
}

async function send() {
  const entries = Object.entries(pending).map(([clientId, seconds]) => ({ clientId, seconds }));
  if (!entries.length || !navigator.onLine) return;
  pending = {};
  sinceSend = 0;
  try {
    await api('POST', '/usage', { entries });
  } catch {
    // Put it back for the next try; a few lost seconds don't matter.
    for (const e of entries) pending[e.clientId] = Math.min(900, (pending[e.clientId] || 0) + e.seconds);
  }
}

/** Starts counting (coaches only). */
export function startUsage() {
  for (const ev of ['pointerdown', 'keydown', 'scroll', 'wheel', 'touchstart']) {
    window.addEventListener(ev, () => { lastInput = Date.now(); }, { passive: true, capture: true });
  }
  setInterval(() => {
    const cid = clientOnScreen();
    const active = document.visibilityState === 'visible' && Date.now() - lastInput < IDLE_S * 1000;
    if (cid && active) pending[cid] = Math.min(900, (pending[cid] || 0) + TICK_S);
    sinceSend += TICK_S;
    if (sinceSend >= SEND_EVERY_S) send();
  }, TICK_S * 1000);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') send(); });
}
