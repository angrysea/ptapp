// Team and group conversations (routes/conversations.rs), beside each
// client's private thread (comments.js): teammates message each other one to
// one, coaches start groups of teammates and clients, and a broadcast sends
// one message to many clients' private threads. Clients see the groups
// they're in on their Coach tab and reply there.

import { api, get, post, put } from '../api.js';
import { onLive } from '../realtime.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { $app, busy, confirmSheet, field, fmtDateTime, h, mount, sheet, toast } from '../ui.js';
import { attachmentView, mediaTools } from './comments.js';
import { refreshMessageBadge } from './messages.js';

const ROLE = { coach: 'Head coach', assistant: 'Team', client: 'Client' };

/** One conversation in a list: name, last message, unread count. */
function row(c) {
  const open = () => go(`/conversations/${encodeURIComponent(c.id)}`);
  return h('div', {
    class: `card link thread-card ${c.unread ? 'unread' : ''}`, role: 'link', tabindex: 0,
    onclick: open, onkeydown: (e) => { if (e.key === 'Enter') open(); },
  },
    h('div', { class: 'spread' },
      h('strong', {}, c.kind === 'group' ? `👥 ${c.title}` : c.title),
      c.unread > 0 ? h('span', { class: 'chip bad' }, `${c.unread} new`) : c.lastAt && h('span', { class: 'muted small' }, fmtDateTime(c.lastAt))),
    h('p', { class: 'small muted', style: 'white-space:pre-wrap' },
      c.lastText ? `${c.lastAuthor}: ${c.lastText}` : c.kind === 'group' ? `${c.members.length} people · no messages yet` : 'No messages yet.'));
}

/** The caller's team and group conversations, for the Messages page and the
 * client's Coach tab; `onlyNew` shows just those with unread messages. */
export function conversationRows(list, { onlyNew = false } = {}) {
  return list.filter((c) => !onlyNew || c.unread > 0).map(row);
}

/** Checkboxes for people, filtered by a search box; `picked()` is their ids. */
function peoplePicker(people, selected = []) {
  const search = h('input', { type: 'search', placeholder: 'Search people', 'aria-label': 'Search people' });
  const boxes = people.map((p) => {
    const box = h('input', { type: 'checkbox', checked: selected.includes(p.userId), 'aria-label': p.name });
    return { p, box, el: h('label', { class: 'check' }, box, p.name, h('span', { class: 'muted small' }, ` · ${ROLE[p.role] ?? p.role}`)) };
  });
  const section = (title, role) => {
    const mine = boxes.filter((b) => (role === 'client') === (b.p.role === 'client'));
    return mine.length > 0 && h('div', {}, h('h3', {}, title), mine.map((b) => b.el));
  };
  search.addEventListener('input', () => {
    const q = search.value.trim().toLowerCase();
    for (const b of boxes) b.el.hidden = !!q && !b.p.name.toLowerCase().includes(q);
  });
  return {
    el: h('div', {}, people.length > 8 && search, section('Team', 'team'), section('Clients', 'client'),
      people.length === 0 && h('p', { class: 'muted small' }, 'No one to message yet.')),
    picked: () => boxes.filter((b) => b.box.checked).map((b) => b.p),
  };
}

/** Start a conversation: one teammate opens your direct conversation;
 * more people, or any name, start a group. */
export async function newConversationSheet() {
  const people = await get('/conversations/people');
  const picker = peoplePicker(people);
  const name = h('input', { maxlength: 60, placeholder: 'e.g. Saturday crew', 'aria-label': 'Group name' });
  const sh = sheet(
    h('h2', {}, 'New message'),
    h('p', { class: 'muted small' }, 'Pick one teammate to message them directly, or several people for a group. Clients in a group see everyone’s name and messages; to reach clients privately, use Broadcast.'),
    picker.el,
    field('Group name (optional)', name),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const who = picker.picked();
        if (who.length === 0) throw new Error('Pick who to message');
        // One client is their private thread.
        if (who.length === 1 && who[0].role === 'client' && !name.value.trim()) {
          sh.close();
          return go(`/c/${encodeURIComponent(who[0].userId)}/messages`);
        }
        const c = await post('/conversations', { members: who.map((p) => p.userId), name: name.value });
        sh.close();
        go(`/conversations/${encodeURIComponent(c.id)}`);
      }),
    }, 'Start'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

/** One message to many clients, each in their own private thread. */
export async function broadcastSheet() {
  const clients = (await get('/conversations/people')).filter((p) => p.role === 'client');
  const picker = peoplePicker(clients);
  const all = h('input', { type: 'checkbox', 'aria-label': 'All clients' });
  all.addEventListener('change', () => picker.el.querySelectorAll('input[type=checkbox]').forEach((b) => { b.checked = all.checked; }));
  const text = h('textarea', { placeholder: 'Your message', 'aria-label': 'Broadcast message', maxlength: 4000, style: 'min-height:90px' });
  const sh = sheet(
    h('h2', {}, '📣 Broadcast'),
    h('p', { class: 'muted small' }, 'Sends the message to each client’s private conversation with you. No one sees who else got it, and replies come back privately.'),
    h('label', { class: 'check' }, all, h('strong', {}, 'All clients')),
    picker.el,
    field('Message', text),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const who = picker.picked();
        if (who.length === 0) throw new Error('Pick at least one client');
        if (!text.value.trim()) throw new Error('Write the message');
        const r = await post('/broadcasts', { clients: who.map((p) => p.userId), text: text.value });
        toast(`Sent privately to ${r.sent} client${r.sent === 1 ? '' : 's'}`);
        sh.close();
      }),
    }, 'Send'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

/** Rename a group, change who's in it, or delete it (its starter or the head coach). */
async function manageSheet(c, onDone) {
  const people = await get('/conversations/people');
  // People already in it stay pickable, even ones the editor couldn't add now.
  const others = c.members.filter((m) => m.userId !== session.userId);
  const union = [...others, ...people.filter((p) => !others.some((m) => m.userId === p.userId))];
  const picker = peoplePicker(union, others.map((m) => m.userId));
  const name = h('input', { value: c.name, maxlength: 60, 'aria-label': 'Group name' });
  const sh = sheet(
    h('h2', {}, 'Edit group'),
    field('Group name', name),
    picker.el,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await put(`/conversations/${encodeURIComponent(c.id)}`, { name: name.value, members: picker.picked().map((p) => p.userId) });
        toast('Group saved');
        sh.close();
        onDone();
      }),
    }, 'Save'),
    h('button', {
      class: 'danger block',
      onclick: async () => {
        if (!(await confirmSheet(`Delete “${c.title}” and all its messages for everyone?`, 'Delete group'))) return;
        await api('DELETE', `/conversations/${encodeURIComponent(c.id)}`);
        sh.close();
        toast('Group deleted');
        go(session.isCoach ? '/messages' : '/coach');
      },
    }, 'Delete group'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

/** `#/conversations/:id`: one team or group conversation. */
export async function renderConversation(id) {
  const back = session.isCoach ? '#/messages' : '#/coach';
  const head = h('div');
  const listEl = h('div', { class: 'thread' });
  const text = h('textarea', { placeholder: 'Write a message…', style: 'min-height:70px', 'aria-label': 'Message' });
  const path = `/conversations/${encodeURIComponent(id)}`;
  const send = async (attachment) => {
    await post(`${path}/messages`, { text: text.value, ...(attachment && { attachment }) });
    text.value = '';
    await draw();
  };
  const media = mediaTools({ uploadPath: `${path}/attachments`, send });
  mount($app(),
    h('a', { href: back, class: 'small' }, '← Messages'),
    head, listEl,
    h('div', { class: 'card' }, text, media.pending,
      h('div', { class: 'row', style: 'margin-top:8px' }, media.buttons,
        h('button', {
          class: 'primary grow',
          onclick: (e) => busy(e.currentTarget, async () => { if (text.value.trim()) await send(); }),
        }, 'Send'))));

  let shown = null; // newest message drawn, so a refresh redraws only on change
  async function draw(force = false) {
    const { conversation: c, messages } = await get(path);
    const newest = `${messages.at(-1)?.id ?? ''}|${c.title}|${c.members.length}`;
    if (newest === shown && !force) return;
    const first = shown === null;
    shown = newest;
    mount(head,
      h('div', { class: 'spread page-head' },
        h('h1', {}, c.kind === 'group' ? `👥 ${c.title}` : c.title),
        h('div', { class: 'row' },
          c.canManage && h('button', { class: 'small', onclick: () => manageSheet(c, () => draw(true)) }, 'Edit group'),
          c.kind === 'group' && h('button', {
            class: 'small ghost',
            onclick: async () => {
              if (!(await confirmSheet(`Leave “${c.title}”? You won’t see its messages any more.`, 'Leave'))) return;
              await post(`${path}/leave`);
              toast(`You left ${c.title}`);
              go(session.isCoach ? '/messages' : '/coach');
            },
          }, 'Leave'))),
      c.kind === 'group' && h('details', { class: 'small' },
        h('summary', {}, `${c.members.length} people`),
        h('ul', {}, c.members.map((m) => h('li', {}, m.userId === session.userId ? `${m.name} (you)` : m.name, h('span', { class: 'muted' }, ` · ${ROLE[m.role] ?? m.role}`))))));
    mount(listEl,
      messages.length === 0 && h('p', { class: 'muted' }, 'No messages yet.'),
      messages.map((m) => h('div', { class: `msg ${m.authorId === session.userId ? 'mine' : ''}` },
        h('div', { class: 'meta' }, `${m.authorName} · ${fmtDateTime(m.createdAt)}`),
        attachmentView(m),
        m.text && h('div', { style: 'white-space:pre-wrap' }, m.text))));
    listEl.lastElementChild?.scrollIntoView({ block: 'nearest', behavior: first ? 'auto' : 'smooth' });
    if (c.unread > 0 && document.visibilityState === 'visible') {
      post(`${path}/read`).then(() => refreshMessageBadge()).catch(() => {});
    }
  }
  try {
    await draw();
  } catch (e) {
    mount(head, h('h1', {}, 'Conversation'), h('p', { class: 'muted' }, e.status === 404 ? 'This conversation isn’t available. You may have left it, or it was deleted.' : e.message));
    return;
  }
  const off = onLive(() => {
    if (!listEl.isConnected) return off();
    draw().catch(() => {});
  });
}
