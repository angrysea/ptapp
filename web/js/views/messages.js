// The coach's messages page (MS-1): every client's conversation, newest and
// unread first, with the unread messages shown and a reply box on each, so
// the coach can answer without leaving the page. Also the unread badges on
// the coach's Messages tab and the client's Coach tab.

import { get, post } from '../api.js';
import { onLive } from '../realtime.js';
import { session } from '../session.js';
import { $app, busy, fmtDateTime, h, mount, toast } from '../ui.js';

const ONLY_NEW = 'pt.messages.onlyNew';

/** Sets the unread badge on the messages tab; fetches the count if not given. */
export async function refreshMessageBadge(count) {
  const tab = document.querySelector(`#tabs a[data-path="${session.isCoach ? '/messages' : '/coach'}"]`);
  if (!tab) return;
  if (count == null) {
    try { count = (await get('/messages')).unreadTotal; } catch { return; }
  }
  let badge = tab.querySelector('.badge');
  if (!count) { badge?.remove(); return; }
  if (!badge) { badge = h('span', { class: 'badge' }); tab.append(badge); }
  badge.textContent = String(count);
  badge.setAttribute('aria-label', `${count} unread`);
}

/** The signed-in user has seen `clientId`'s conversation. */
export async function markRead(clientId) {
  try {
    await post(`/clients/${clientId}/messages/read`);
    refreshMessageBadge();
  } catch { /* offline: it's marked next time */ }
}

export async function renderInbox() {
  let onlyNew = false;
  try { onlyNew = localStorage.getItem(ONLY_NEW) === '1'; } catch { /* storage unavailable */ }
  const filter = h('input', { type: 'checkbox', checked: onlyNew });
  const listEl = h('div');
  const open = new Set(); // client ids with the reply box open
  const drafts = {}; // typed replies, kept across refreshes

  filter.addEventListener('change', () => {
    onlyNew = filter.checked;
    try { localStorage.setItem(ONLY_NEW, onlyNew ? '1' : '0'); } catch { /* not remembered */ }
    draw();
  });

  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Messages'), h('label', { class: 'check' }, filter, 'Only new messages')),
    listEl);

  let data = null;
  async function load() {
    data = await get('/messages', { cache: true });
    refreshMessageBadge(data.unreadTotal);
    draw();
  }

  function draw() {
    const rows = data.threads.filter((t) => !onlyNew || t.unread > 0);
    mount(listEl,
      rows.length === 0 && h('div', { class: 'card muted' }, onlyNew ? 'No new messages.' : 'No clients yet.'),
      rows.map(card));
  }

  function card(t) {
    const replying = open.has(t.clientId);
    const reply = h('textarea', { placeholder: `Reply to ${t.clientName}…`, 'aria-label': `Reply to ${t.clientName}`, 'data-cid': t.clientId, value: drafts[t.clientId] || '' });
    reply.addEventListener('input', () => { drafts[t.clientId] = reply.value; });
    const send = h('button', {
      class: 'primary',
      onclick: (e) => busy(e.currentTarget, async () => {
        if (!reply.value.trim()) return;
        await post(`/clients/${t.clientId}/comments`, { text: reply.value });
        await post(`/clients/${t.clientId}/messages/read`);
        delete drafts[t.clientId];
        open.delete(t.clientId);
        toast(`Sent to ${t.clientName}`);
        await load();
      }),
    }, 'Send');
    const last = t.lastText && h('p', { class: 'small muted', style: 'white-space:pre-wrap' },
      `${t.lastAuthorRole === 'coach' ? 'You' : t.clientName}: ${t.lastText}`);
    return h('div', { class: `card thread-card ${t.unread ? 'unread' : ''}` },
      h('div', { class: 'spread' },
        h('a', { href: `#/c/${encodeURIComponent(t.clientId)}/messages` }, h('strong', {}, t.clientName)),
        t.unread > 0 ? h('span', { class: 'chip bad' }, `${t.unread} new`) : t.lastAt && h('span', { class: 'muted small' }, fmtDateTime(t.lastAt))),
      t.newMessages.length > 0
        ? t.newMessages.map((m) => h('div', { class: 'msg' },
          h('div', { class: 'meta' }, fmtDateTime(m.createdAt)),
          h('div', { style: 'white-space:pre-wrap' }, m.text || (m.attachment?.kind === 'voice' ? '🎤 Voice note' : m.attachment ? '📷 Photo' : '')),
          m.text && m.attachment && h('div', { class: 'small muted' }, m.attachment.kind === 'voice' ? '🎤 with a voice note' : '📷 with a photo')))
        : last || h('p', { class: 'small muted' }, 'No messages yet.'),
      replying
        ? h('div', {}, reply, h('div', { class: 'row', style: 'margin-top:8px' },
          h('button', { class: 'ghost', onclick: () => { open.delete(t.clientId); draw(); } }, 'Cancel'),
          send,
          t.unread > 0 && h('button', {
            class: 'ghost',
            onclick: (e) => busy(e.currentTarget, async () => { await post(`/clients/${t.clientId}/messages/read`); await load(); }),
          }, 'Mark read')))
        : h('div', { class: 'row' },
          h('button', { onclick: () => { open.add(t.clientId); draw(); listEl.querySelector(`textarea[data-cid="${CSS.escape(t.clientId)}"]`)?.focus(); } }, 'Reply'),
          t.unread > 0 && h('button', {
            class: 'ghost',
            onclick: (e) => busy(e.currentTarget, async () => { await post(`/clients/${t.clientId}/messages/read`); await load(); }),
          }, 'Mark read')));
  }

  await load();
  // New messages appear without a reload; not while the coach is typing a reply.
  const off = onLive(() => {
    if (!listEl.isConnected) return off();
    if (listEl.contains(document.activeElement) && document.activeElement.tagName === 'TEXTAREA') return;
    load().catch(() => {});
  });
}
