// Assistant coaches (routes/team.rs, ptcore::team). The head coach invites
// assistants by email with a title and permissions, and assigns them
// clients; an assistant sees their own role on Profile. What each
// permission opens is enforced by the API; the app just hides what an
// assistant can't use.

import { api, get, post, put } from '../api.js';
import { session } from '../session.js';
import { busy, confirmSheet, field, fmtDate, h, mount, sheet, toast } from '../ui.js';

export const PERMS = [
  ['programming', 'Programming', 'Programs and starter programs, assigning them, clients’ programs and 1RMs, custom exercises.'],
  ['nutrition', 'Nutrition', 'Recipes, recipe books, meal plans, the food journal and nutrition targets.'],
  ['messaging', 'Messaging', 'Messages with the client, including the photos and voice notes they send.'],
  ['review', 'Video review', 'Set videos waiting for review, and video replies.'],
  ['progress', 'Progress (view)', 'Workouts and logs, body metrics and photos, check-ins and habits. Read only.'],
  ['health', 'Health & intake', 'The intake questionnaire and health screening (PAR-Q, waiver).'],
];
const permLabel = (p) => PERMS.find(([k]) => k === p)?.[1] ?? p;
const DEFAULT_TITLE = 'Assistant Coach';

function permBoxes(selected) {
  return PERMS.map(([k, label, help]) => {
    const box = h('input', { type: 'checkbox', checked: selected.includes(k), value: k, 'aria-label': label });
    return { k, box, el: h('div', { class: 'field' }, h('label', { class: 'check' }, box, h('strong', {}, label)), h('div', { class: 'muted small' }, help)) };
  });
}
const picked = (boxes) => boxes.filter((b) => b.box.checked).map((b) => b.k);

/** Head coach's Profile card: the team, invitations, invite and edit. */
export function teamCard() {
  const card = h('div', { class: 'card' });
  let latest = 0; // a slow earlier load must not draw over a newer one
  const draw = async () => {
    const mine = ++latest;
    const [team, roster] = await Promise.all([get('/team'), get('/clients', { cache: true }).catch(() => [])]);
    if (mine !== latest) return;
    const names = Object.fromEntries(roster.map((r) => [r.client.clientId, r.client.name]));
    mount(card,
      h('div', { class: 'spread' }, h('h3', {}, 'Your team'), h('button', { class: 'small primary', onclick: () => inviteSheet(draw) }, '+ Invite assistant')),
      h('p', { class: 'muted small' }, 'Assistant coaches help with the clients you assign them, for only what you allow. Clients see their name and title.'),
      team.members.length === 0 && team.invites.length === 0 && h('p', { class: 'muted small' }, 'No assistants yet.'),
      team.members.map((m) => h('div', { class: 'team-row' },
        h('div', { class: 'spread' },
          h('div', {}, h('strong', {}, m.name), ' ', h('span', { class: 'chip' }, m.title), h('div', { class: 'muted small' }, m.email)),
          h('button', { class: 'small', onclick: () => memberSheet(m, roster, draw) }, 'Edit')),
        h('div', { class: 'small' }, m.perms.length ? m.perms.map(permLabel).join(' · ') : h('span', { class: 'muted' }, 'No permissions yet')),
        h('div', { class: 'muted small' }, m.clients.length ? `Clients: ${m.clients.map((c) => names[c] ?? c).join(', ')}` : 'No clients assigned yet'))),
      team.invites.map((i) => h('div', { class: 'spread small team-row' },
        h('span', {}, `Invited ${i.email} as ${i.title || DEFAULT_TITLE} · ${fmtDate(i.createdAt.slice(0, 10))}`),
        h('button', {
          class: 'small ghost', onclick: async () => {
            if (!(await confirmSheet(`Cancel the invitation to ${i.email}?`, 'Cancel invitation'))) return;
            await api('DELETE', `/team/invites/${i.code}`);
            draw();
          },
        }, 'Cancel'))));
  };
  draw().catch((e) => mount(card, h('h3', {}, 'Your team'), h('p', { class: 'muted small' }, e.message)));
  return card;
}

function inviteSheet(onDone) {
  const email = h('input', { type: 'email', placeholder: 'name@example.com' });
  const name = h('input', { placeholder: 'Their name' });
  const title = h('input', { value: DEFAULT_TITLE, maxlength: 40, placeholder: DEFAULT_TITLE });
  const boxes = permBoxes(['progress']);
  const result = h('div');
  const sh = sheet(
    h('h2', {}, 'Invite an assistant coach'),
    field('Email', email), field('Name', name), field('Title', title, 'e.g. Nutrition Coach, Programming Assistant. Clients see it.'),
    h('h3', {}, 'What they can do'), boxes.map((b) => b.el),
    h('p', { class: 'muted small' }, 'You assign them clients once they’ve joined. Billing, plans and credits, invitations and your team stay yours.'),
    result,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        if (!email.value.includes('@')) throw new Error('Enter their email');
        const inv = await post('/team/invites', { email: email.value, name: name.value, title: title.value, perms: picked(boxes) });
        const link = `${location.origin}${location.pathname}#/join/${inv.code}`;
        mount(result, h('p', { class: 'small' }, inv.emailed ? `Invitation emailed to ${inv.email}.` : 'Email isn’t set up here; share this link:', !inv.emailed && h('div', {}, h('input', { value: link, readonly: true, 'aria-label': 'Invitation link' }))));
        toast('Invitation sent');
        onDone();
      }),
    }, 'Send invitation'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'));
}

function memberSheet(m, roster, onDone) {
  const title = h('input', { value: m.title, maxlength: 40 });
  const boxes = permBoxes(m.perms);
  const clientBoxes = roster.map((r) => {
    const box = h('input', { type: 'checkbox', checked: m.clients.includes(r.client.clientId), 'aria-label': `Assign ${r.client.name}` });
    return { id: r.client.clientId, box, el: h('label', { class: 'check' }, box, r.client.name) };
  });
  const sh = sheet(
    h('h2', {}, m.name),
    field('Title', title),
    h('h3', {}, 'What they can do'), boxes.map((b) => b.el),
    h('h3', {}, 'Their clients'),
    roster.length ? clientBoxes.map((c) => c.el) : h('p', { class: 'muted small' }, 'You have no clients yet.'),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await put(`/team/${m.userId}`, { title: title.value, perms: picked(boxes), clients: clientBoxes.filter((c) => c.box.checked).map((c) => c.id) });
        toast('Saved');
        sh.close();
        onDone();
      }),
    }, 'Save'),
    h('button', {
      class: 'danger block',
      onclick: async () => {
        if (!(await confirmSheet(`Remove ${m.name} from your team? They lose access at once; what they made stays with you.`, 'Remove'))) return;
        await api('DELETE', `/team/${m.userId}`);
        toast(`${m.name} removed`);
        sh.close();
        onDone();
      },
    }, 'Remove from team'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

/** An assistant's Profile card: their head coach, title, permissions and clients. */
export function myRoleCard() {
  const m = session.member;
  if (!m) return null;
  return h('div', { class: 'card' },
    h('h3', {}, m.title),
    h('p', { class: 'small' }, 'You can help with: ', m.perms.length ? m.perms.map(permLabel).join(' · ') : 'nothing yet'),
    h('p', { class: 'muted small' }, `${m.clients.length} client${m.clients.length === 1 ? '' : 's'} assigned to you. Your coach manages what you can do and who you work with.`));
}
