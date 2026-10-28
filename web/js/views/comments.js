// FB-1 coach feedback and the coach-client message thread (MS-1). The thread
// refreshes itself on live events (realtime.js) and marks itself read while
// it's on screen, which clears the unread badges (views/messages.js).

import { get, post } from '../api.js';
import { onLive } from '../realtime.js';
import { session } from '../session.js';
import { busy, fmtDateTime, h, mount } from '../ui.js';
import { playReply } from '../videos.js';
import { markRead } from './messages.js';

export async function renderThread(el, clientId, logsById = {}) {
  const listEl = h('div', { class: 'thread' });
  const text = h('textarea', { placeholder: 'Write a message…', style: 'min-height:70px', 'aria-label': 'Message' });
  const anchor = (c) => {
    if (!c.logId) return null;
    const log = logsById[c.logId];
    const what = log ? `${log.workoutName}` : 'a workout';
    const set = c.setIndex != null ? `, set ${c.setIndex + 1}` : '';
    return h('div', { class: 'meta' }, `on ${what}${set}`);
  };
  let shown = null; // id of the newest message drawn, to redraw and mark read only on change
  async function draw() {
    const list = await get(`/clients/${clientId}/comments`, { cache: true });
    const newest = list.at(-1)?.id ?? '';
    if (newest === shown) return;
    const first = shown === null;
    shown = newest;
    // Messages that carry a coach's video reply (VR-2) get a play button.
    let replies = null; // null: couldn't load them (offline)
    if (list.some((c) => c.replyId)) {
      try { replies = Object.fromEntries((await get(`/clients/${clientId}/replies`)).map((r) => [r.id, r])); } catch { /* offline */ }
    }
    const play = (c) => {
      if (!c.replyId) return null;
      const r = replies?.[c.replyId];
      if (!r?.url) return h('div', { class: 'small muted' }, replies ? 'Video reply (no longer available)' : 'Video reply (needs a connection)');
      return h('button', {
        class: 'small reply', style: 'margin-top:6px',
        onclick: () => playReply(r, {
          onPlay: () => { if (!session.isCoach && !r.seenAt) post(`/clients/${clientId}/replies/${r.id}/seen`).catch(() => {}); },
        }),
      }, '▶ Play video reply');
    };
    mount(listEl,
      list.length === 0 && h('p', { class: 'muted' }, 'No messages yet.'),
      list.map((c) => h('div', { class: `msg ${c.authorId === session.userId ? 'mine' : ''}` },
        h('div', { class: 'meta' }, `${c.authorName} · ${fmtDateTime(c.createdAt)}`),
        anchor(c),
        h('div', { style: 'white-space:pre-wrap' }, c.text),
        play(c))));
    listEl.lastElementChild?.scrollIntoView({ block: 'nearest', behavior: first ? 'auto' : 'smooth' });
    if (list.some((c) => c.authorId !== session.userId) && document.visibilityState === 'visible') markRead(clientId);
  }
  mount(el,
    listEl,
    h('div', { class: 'card' }, text,
      h('button', {
        class: 'primary block', style: 'margin-top:8px',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!text.value.trim()) return;
          await post(`/clients/${clientId}/comments`, { text: text.value });
          text.value = '';
          await draw();
        }),
      }, 'Send')),
  );
  await draw();
  // New messages appear while the thread is open; stops once it's gone.
  const off = onLive(() => {
    if (!el.isConnected) return off();
    draw().catch(() => {});
  });
}

/** Prompt for a comment anchored to a workout log or one set in it. */
export async function commentOn(clientId, logId, setIndex, itemId) {
  const t = prompt(setIndex != null ? `Comment on set ${setIndex + 1}` : 'Comment on this workout');
  if (!t?.trim()) return false;
  await post(`/clients/${clientId}/comments`, { text: t, logId, setIndex: setIndex ?? null, itemId: itemId ?? null });
  return true;
}
