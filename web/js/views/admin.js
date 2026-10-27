// Admin: onboard coaches. Review coach requests, invite coaches by email or
// link, and see who is coaching. There is no open coach signup.

import { del, get, post } from '../api.js';
import { $app, busy, confirmSheet, field, fmtDate, fmtDateTime, h, mount, sheet, toast } from '../ui.js';
import { COACH_REQUEST_PATH } from './onboarding.js';

const appLink = (path) => `${location.origin}${location.pathname}#${path}`;

export async function renderAdmin() {
  const [requests, coaches, invites] = await Promise.all([
    get('/admin/requests'), get('/admin/coaches', { cache: true }), get('/admin/invites'),
  ]);
  const pending = requests.filter((r) => r.status === 'pending');
  const decided = requests.filter((r) => r.status !== 'pending').slice(0, 5);
  mount($app(),
    h('div', { class: 'spread' }, h('h1', {}, 'Coaches'), h('button', { class: 'primary', onclick: inviteSheet }, '+ Invite coach')),

    h('h2', {}, `Requests${pending.length ? ` (${pending.length})` : ''}`),
    pending.length === 0 && h('div', { class: 'card muted' }, 'No requests waiting.'),
    pending.map(requestCard),

    h('h2', {}, 'Coaches'),
    coaches.length === 0 && h('div', { class: 'card muted' }, 'No coaches yet. Invite one, or approve a request.'),
    coaches.map((c) => h('div', { class: 'card' },
      h('strong', {}, c.name), h('div', { class: 'muted small' }, `${c.email} · joined ${fmtDate(c.joinedAt.slice(0, 10))}`))),

    invites.length > 0 && [h('h2', {}, 'Pending coach invites'), invites.map((i) => h('div', { class: 'card spread' },
      h('div', {}, h('strong', {}, i.clientName || i.email), h('div', { class: 'muted small' }, `${i.email} · code ${i.code}`)),
      h('div', { class: 'row' },
        h('button', { class: 'small', onclick: () => shareInvite(i) }, 'Share'),
        h('button', { class: 'small danger', onclick: async () => { await del(`/admin/invites/${i.code}`); renderAdmin(); } }, 'Revoke'))))],

    h('h2', {}, 'Coach request link'),
    h('div', { class: 'card' },
      h('p', { class: 'small muted' }, 'Share this with coaches who want to join. They sign in and send a request, which shows up above.'),
      h('input', { value: appLink(COACH_REQUEST_PATH), readonly: true, onfocus: (e) => e.target.select() }),
      h('button', { class: 'block', style: 'margin-top:8px',
        onclick: async () => { await navigator.clipboard.writeText(appLink(COACH_REQUEST_PATH)); toast('Copied'); } }, 'Copy link')),

    decided.length > 0 && [h('h2', {}, 'Recently decided'), decided.map((r) => h('div', { class: 'card spread small' },
      h('div', {}, h('strong', {}, r.name), h('div', { class: 'muted' }, r.email)),
      h('span', { class: `chip ${r.status === 'approved' ? 'good' : 'gray'}` }, r.status === 'approved' ? 'Approved' : 'Declined')))],
  );
}

function requestCard(r) {
  return h('div', { class: 'card' },
    h('div', { class: 'spread' },
      h('div', {}, h('strong', {}, r.name), h('div', { class: 'muted small' }, r.email)),
      h('span', { class: 'muted small' }, fmtDateTime(r.createdAt))),
    r.businessName && h('div', { class: 'small' }, r.businessName),
    r.message && h('p', { style: 'white-space:pre-wrap' }, r.message),
    h('div', { class: 'row' },
      h('button', {
        class: 'grow',
        onclick: async () => {
          if (!(await confirmSheet(`Decline ${r.name}’s request? They can send a new one.`, 'Decline'))) return;
          await post(`/admin/requests/${encodeURIComponent(r.userId)}/decline`);
          renderAdmin();
        },
      }, 'Decline'),
      h('button', {
        class: 'primary grow',
        onclick: (e) => busy(e.currentTarget, async () => {
          await post(`/admin/requests/${encodeURIComponent(r.userId)}/approve`);
          toast(`${r.name} is now a coach`);
          renderAdmin();
        }),
      }, 'Approve')));
}

function shareInvite(inv) {
  const link = appLink(`/join/${inv.code}`);
  const subject = encodeURIComponent('You’re invited to coach on PT Coach');
  const body = encodeURIComponent(`Hi ${inv.clientName || ''},\n\nSet up your coach account on PT Coach: ${link}\n\nCreate an account with this email (${inv.email}) and accept the invite. Your invite code is ${inv.code}.`);
  const sh = sheet(
    h('h2', {}, 'Coach invite'),
    h('p', { class: 'small' }, 'Valid for 14 days. Code: ', h('strong', {}, inv.code)),
    h('input', { value: link, readonly: true, onfocus: (e) => e.target.select() }),
    h('div', { class: 'row', style: 'margin-top:10px' },
      h('button', { class: 'grow', onclick: async () => { await navigator.clipboard.writeText(link); toast('Copied'); } }, 'Copy link'),
      h('a', { class: 'btn primary grow', href: `mailto:${inv.email}?subject=${subject}&body=${body}` }, 'Email')),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Done'),
  );
}

function inviteSheet() {
  const name = h('input', { autocomplete: 'off' });
  const email = h('input', { type: 'email', autocomplete: 'off' });
  const sh = sheet(
    h('h2', {}, 'Invite a coach'),
    field('Coach name', name),
    field('Email', email),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const inv = await post('/admin/invites', { name: name.value, email: email.value });
        sh.close();
        shareInvite(inv);
        renderAdmin();
      }),
    }, 'Create invite'),
  );
}
