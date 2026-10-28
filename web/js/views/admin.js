// Admin: onboard coaches by emailed invitation, and see who is coaching.
// There is no open coach signup: an invitation is tied to one email address,
// its link opens the sign-up page with that address filled in, and it stops
// working once used.

import { del, get, post } from '../api.js';
import { $app, busy, field, fmtDate, h, mount, sheet, toast } from '../ui.js';

const appLink = (path) => `${location.origin}${location.pathname}#${path}`;

export async function renderAdmin() {
  const [coaches, invites] = await Promise.all([get('/admin/coaches', { cache: true }), get('/admin/invites')]);
  mount($app(),
    h('div', { class: 'spread' }, h('h1', {}, 'Coaches'), h('button', { class: 'primary', onclick: inviteSheet }, '+ Invite coach')),

    h('h2', {}, 'Coaches'),
    coaches.length === 0 && h('div', { class: 'card muted' }, 'No coaches yet. Invite one by email.'),
    coaches.map((c) => h('div', { class: 'card' },
      h('strong', {}, c.name), h('div', { class: 'muted small' }, `${c.email} · joined ${fmtDate(c.joinedAt.slice(0, 10))}`))),

    invites.length > 0 && [h('h2', {}, 'Pending invitations'), invites.map((i) => h('div', { class: 'card spread' },
      h('div', {}, h('strong', {}, i.clientName || i.email), h('div', { class: 'muted small' }, `${i.email} · expires ${fmtDate(new Date(i.ttl * 1000).toISOString().slice(0, 10))}`)),
      h('div', { class: 'row' },
        h('button', {
          class: 'small',
          onclick: (e) => busy(e.currentTarget, async () => emailResult(await post(`/admin/invites/${i.code}/resend`))),
        }, 'Resend email'),
        h('button', { class: 'small', onclick: () => shareInvite(i) }, 'Link'),
        h('button', { class: 'small danger', onclick: async () => { await del(`/admin/invites/${i.code}`); renderAdmin(); } }, 'Revoke'))))],
  );
}

/** Tells the admin whether the invitation email went out; if not, offers the link. */
function emailResult(inv) {
  if (inv.emailed) toast(`Invitation emailed to ${inv.email}`);
  else shareInvite(inv, inv.emailError ? 'The email couldn’t be sent, so share this link with them yourself.' : 'Email isn’t set up yet, so share this link with them yourself.');
}

function shareInvite(inv, note) {
  const link = appLink(`/join/${inv.code}`);
  const sh = sheet(
    h('h2', {}, 'Invitation link'),
    note && h('p', { class: 'banner' }, note),
    h('p', { class: 'small' }, `For ${inv.email} only. It opens sign-up with that address filled in, works once, and expires in 14 days.`),
    h('input', { value: link, readonly: true, onfocus: (e) => e.target.select() }),
    h('div', { class: 'row', style: 'margin-top:10px' },
      h('button', { class: 'grow', onclick: async () => { await navigator.clipboard.writeText(link); toast('Copied'); } }, 'Copy link')),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Done'),
  );
}

function inviteSheet() {
  const name = h('input', { autocomplete: 'off' });
  const email = h('input', { type: 'email', autocomplete: 'off' });
  const sh = sheet(
    h('h2', {}, 'Invite a coach'),
    h('p', { class: 'small muted' }, 'They get an email with a sign-up link for this address.'),
    field('Coach name', name),
    field('Email', email),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const inv = await post('/admin/invites', { name: name.value, email: email.value });
        sh.close();
        emailResult(inv);
        renderAdmin();
      }),
    }, 'Send invitation'),
  );
}
