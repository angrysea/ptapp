// FB-1 coach feedback + a simple async message thread between coach and
// client (real-time chat with push arrives in Phase 1b, see docs/ROADMAP.md).

import { get, post } from '../api.js';
import { session } from '../session.js';
import { busy, fmtDateTime, h, mount } from '../ui.js';

export async function renderThread(el, clientId, logsById = {}) {
  const list = await get(`/clients/${clientId}/comments`, { cache: true });
  const text = h('textarea', { placeholder: 'Write a message…', style: 'min-height:70px' });
  const anchor = (c) => {
    if (!c.logId) return null;
    const log = logsById[c.logId];
    const what = log ? `${log.workoutName}` : 'a workout';
    const set = c.setIndex != null ? `, set ${c.setIndex + 1}` : '';
    return h('div', { class: 'meta' }, `on ${what}${set}`);
  };
  mount(el,
    h('div', { class: 'thread' },
      list.length === 0 && h('p', { class: 'muted' }, 'No messages yet.'),
      list.map((c) => h('div', { class: `msg ${c.authorId === session.userId ? 'mine' : ''}` },
        h('div', { class: 'meta' }, `${c.authorName} · ${fmtDateTime(c.createdAt)}`),
        anchor(c),
        h('div', { style: 'white-space:pre-wrap' }, c.text)))),
    h('div', { class: 'card' }, text,
      h('button', {
        class: 'primary block', style: 'margin-top:8px',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!text.value.trim()) return;
          await post(`/clients/${clientId}/comments`, { text: text.value });
          renderThread(el, clientId, logsById);
        }),
      }, 'Send')),
  );
  el.querySelector('.thread .msg:last-child')?.scrollIntoView({ block: 'nearest' });
}

/** Prompt for a comment anchored to a workout log or one set in it. */
export async function commentOn(clientId, logId, setIndex, itemId) {
  const t = prompt(setIndex != null ? `Comment on set ${setIndex + 1}` : 'Comment on this workout');
  if (!t?.trim()) return false;
  await post(`/clients/${clientId}/comments`, { text: t, logId, setIndex: setIndex ?? null, itemId: itemId ?? null });
  return true;
}
