// Admin: onboard coaches by emailed invitation, and see who is coaching.
// There is no open coach signup: an invitation is tied to one email address,
// its link opens the sign-up page with that address filled in, and it stops
// working once used.

import { del, get, post, put } from '../api.js';
import { $app, busy, field, fmtDate, h, mount, sheet, toast } from '../ui.js';

const appLink = (path) => `${location.origin}${location.pathname}#${path}`;

export async function renderAdmin() {
  const [coaches, invites] = await Promise.all([get('/admin/coaches', { cache: true }), get('/admin/invites')]);
  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Coaches'), h('button', { class: 'primary', onclick: inviteSheet }, '+ Invite coach')),

    coaches.length === 0 && h('div', { class: 'card muted' }, 'No coaches yet. Invite one by email.'),
    coaches.map((c) => h('div', { class: 'card link', role: 'button', tabindex: 0, onclick: () => coachSheet(c.coachId), onkeydown: (e) => e.key === 'Enter' && coachSheet(c.coachId) },
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

/** A coach's full profile, private details included, and the directory switch. */
async function coachSheet(id) {
  const { account, profile: p, photoUrl, listed } = await get(`/admin/coaches/${encodeURIComponent(id)}`);
  const row = (label, value) => value && h('p', {}, h('strong', {}, `${label}: `), value);
  const date = (d) => (d ? fmtDate(d, { year: 'numeric', month: 'short', day: 'numeric' }) : 'no date given');
  const sh = sheet(
    h('h2', {}, account.name),
    h('p', { class: 'muted small' }, account.email),
    !p && h('p', { class: 'banner' }, 'This coach hasn’t filled in their profile yet.'),
    p && [
      h('div', { class: 'row' }, h('img', { class: 'coach-photo', alt: '', src: photoUrl || 'icon.svg' }),
        h('span', { class: `chip ${listed ? 'good' : 'gray'}` }, listed ? 'In the directory' : p.hiddenByAdmin ? 'Hidden by admin' : 'Not in the directory')),
      h('h3', {}, 'Business details (private)'),
      row('Legal name', p.legalName),
      row('Phone', p.phone),
      row('Address', [p.address.line1, p.address.line2, `${p.address.city}, ${p.address.state} ${p.address.postalCode}`, p.address.country].filter(Boolean).join(', ')),
      row('CPR/AED expires', date(p.cprExpires)),
      row('Insurance', p.insurance && `${p.insurance.provider}, policy ${p.insurance.policyNumber}, expires ${date(p.insurance.expires)}`),
      h('h3', {}, 'Certifications'),
      h('ul', {}, p.certifications.map((c) => h('li', {}, `${c.name} (${c.issuer})${c.number ? ` · no. ${c.number}` : ''}${c.expires ? ` · expires ${date(c.expires)}` : ''}`))),
      h('h3', {}, 'Public profile'),
      row('Headline', p.headline),
      row('Contact email', p.contactEmail),
      h('a', { href: `#/coaches/${encodeURIComponent(id)}`, onclick: () => sh.close() }, 'View public page'),
      h('div', { style: 'height:12px' }),
      h('button', {
        class: `block ${p.hiddenByAdmin ? '' : 'danger'}`,
        onclick: (e) => busy(e.currentTarget, async () => {
          const r = await put(`/admin/coaches/${encodeURIComponent(id)}/hidden`, { hidden: !p.hiddenByAdmin });
          toast(r.hidden ? 'Hidden from the directory' : r.listed ? 'Back in the directory' : 'Unhidden (the coach has unlisted themselves)');
          sh.close();
        }),
      }, p.hiddenByAdmin ? 'Show in the directory' : 'Hide from the directory')],
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'),
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
