// Admin: onboard coaches by emailed invitation, and see who is coaching.
// There is no open coach signup: an invitation is tied to one email address,
// its link opens the sign-up page with that address filled in, and it stops
// working once used.

import { api, del, get, post, put } from '../api.js';
import { $app, busy, confirmSheet, field, fmtDate, h, mount, select, sheet, toast } from '../ui.js';

const appLink = (path) => `${location.origin}${location.pathname}#${path}`;

export async function renderAdmin() {
  const [coaches, invites] = await Promise.all([get('/admin/coaches', { cache: true }), get('/admin/invites')]);
  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Coaches'), h('button', { class: 'primary', onclick: inviteSheet }, '+ Invite coach')),

    coaches.length === 0 && h('div', { class: 'card muted' }, 'No coaches yet. Invite one by email.'),
    coaches.map((c) => h('div', { class: 'card link', role: 'button', tabindex: 0, onclick: () => coachSheet(c.coachId), onkeydown: (e) => e.key === 'Enter' && coachSheet(c.coachId) },
      h('div', { class: 'spread' }, h('strong', {}, c.name), c.suspendedAt && h('span', { class: 'chip warn' }, 'Suspended')),
      h('div', { class: 'muted small' }, `${c.email} · joined ${fmtDate(c.joinedAt.slice(0, 10))}`))),

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
  const [{ account, profile: p, photoUrl, listed }, usage, clients] = await Promise.all([
    get(`/admin/coaches/${encodeURIComponent(id)}`),
    get(`/admin/coaches/${encodeURIComponent(id)}/usage?weeks=4`).catch(() => null),
    get(`/admin/coaches/${encodeURIComponent(id)}/clients`).catch(() => []),
  ]);
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
    usage && h('div', {},
      h('h3', {}, 'Coaching time (last 4 weeks)'),
      h('p', {}, usage.avgMinutesPerClientPerWeek == null ? 'No clients yet.'
        : `${usage.avgMinutesPerClientPerWeek} min per client per week, across ${usage.clients} client${usage.clients === 1 ? '' : 's'}.`),
      h('p', { class: 'muted small' }, usage.weeks.map((w) => `${fmtDate(w.weekStart, { month: 'short', day: 'numeric' })}: ${w.minutes} min`).join(' · '))),
    h('h3', {}, `Clients (${clients.length})`),
    clients.length === 0 ? h('p', { class: 'muted small' }, 'No clients.')
      : clients.map((c) => h('div', { class: 'spread', style: 'padding:4px 0' },
        h('span', {}, c.name, h('span', { class: 'muted small' }, ` · ${c.email}`)),
        h('button', { class: 'small', onclick: () => { sh.close(); moveSheet(c, id, account.name); } }, 'Move…'))),
    h('h3', {}, 'Account'),
    account.suspendedAt && h('p', { class: 'banner' }, `Suspended ${fmtDate(account.suspendedAt.slice(0, 10))}. They can’t sign in; their clients can still log workouts.`),
    h('button', {
      class: `block ${account.suspendedAt ? '' : 'danger'}`,
      onclick: (e) => busy(e.currentTarget, async () => {
        const suspend = !account.suspendedAt;
        if (suspend && !(await confirmSheet(`Suspend ${account.name}? They’re signed out and can’t sign in or appear in the directory until you lift it. Their clients keep their accounts.`, 'Suspend'))) return;
        await put(`/admin/coaches/${encodeURIComponent(id)}/suspended`, { suspended: suspend });
        toast(suspend ? 'Coach suspended' : 'Suspension lifted');
        sh.close();
        renderAdmin();
      }),
    }, account.suspendedAt ? 'Lift suspension' : 'Suspend coach'),
    h('button', { class: 'block danger', onclick: () => { sh.close(); deleteCoachSheet(id, account); } }, 'Delete coach account'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'),
  );
}

/** Deleting a coach is permanent, so the admin types their email first. */
function deleteCoachSheet(id, account) {
  const confirm = h('input', { type: 'email', autocomplete: 'off' });
  const problem = h('p', { class: 'small', 'aria-live': 'polite' });
  const sh = sheet(
    h('h2', {}, `Delete ${account.name}`),
    h('p', {}, 'This permanently deletes the coach’s account, exercises, programs, profile and directory entry. A coach who still has clients can’t be deleted: move their clients to another coach first (from this coach’s sheet).'),
    field(`Type ${account.email} to confirm`, confirm),
    problem,
    h('button', {
      class: 'danger block',
      onclick: (e) => busy(e.currentTarget, async () => {
        try {
          await api('DELETE', `/admin/coaches/${encodeURIComponent(id)}`, { confirm: confirm.value });
        } catch (err) {
          problem.textContent = err.status === 400 ? 'That isn’t this coach’s email address.' : err.body?.current?.message || err.message;
          return;
        }
        sh.close();
        toast('Coach account deleted');
        renderAdmin();
      }),
    }, 'Delete permanently'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
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

/** Move a client to another coach: their own data goes with them. */
async function moveSheet(client, fromId, fromName) {
  const coaches = (await get('/admin/coaches')).filter((c) => c.coachId !== fromId && !c.suspendedAt);
  if (!coaches.length) return toast('There’s no other coach to move them to.', 'error');
  const to = select(coaches.map((c) => [c.coachId, c.name]), coaches[0].coachId);
  const sh = sheet(
    h('h2', {}, `Move ${client.name}`),
    h('p', {}, `From ${fromName} to:`),
    field('New coach', to),
    h('ul', { class: 'small' },
      h('li', {}, 'Their workouts, measurements, photos, videos, messages and the programs on their calendar go with them, so the new coach sees their history.'),
      h('li', {}, 'Videos waiting for review move to the new coach’s queue.'),
      h('li', {}, `Sessions still to come with ${fromName} are cancelled.`),
      h('li', {}, `${fromName} loses access. Both the client and the new coach are told.`)),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const r = await post(`/admin/clients/${encodeURIComponent(client.clientId)}/move`, { coachId: to.value });
        toast(`${client.name} moved${r.cancelledSessions ? `; ${r.cancelledSessions} session${r.cancelledSessions === 1 ? '' : 's'} cancelled` : ''}`);
        sh.close();
        renderAdmin();
      }),
    }, 'Move client'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
}
