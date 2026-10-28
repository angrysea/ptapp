import { myOutbox, put } from '../api.js';
import { pendingUploads, removeUpload } from '../uploads.js';
import { logout } from '../auth.js';
import { kv, outbox } from '../idb.js';
import { session } from '../session.js';
import { $app, busy, field, h, mount, select, toast } from '../ui.js';

/** Signs out from this device: warns about unsynced changes, then clears
 * this user's cached data, queued changes and pending videos. Used by the
 * Profile page and the header's sign-out button. */
export async function signOut() {
  const mine = await myOutbox();
  const videos = await pendingUploads();
  if ((mine.length || videos.length) && !confirm('Some changes have not synced yet and will be lost. Sign out anyway?')) return;
  // Only this user's queued writes go; another user's stay until they sign in.
  await kv.clear(); for (const i of mine) await outbox.del(i.key);
  for (const v of videos) await removeUpload(v.key); // clips stay off a shared device
  logout();
}

export async function renderProfile() {
  const p = session.profile;
  const name = h('input', { value: p.name });
  const unitsSel = select([['metric', 'Metric (kg, cm)'], ['imperial', 'Imperial (lb, in)']], p.units);
  const business = p.role === 'coach' && h('input', { value: p.businessName || '' });
  const mine = await myOutbox();
  const videos = await pendingUploads();
  const pending = mine.length + videos.length;
  mount($app(),
    h('h1', {}, 'Profile'),
    h('div', { class: 'card' },
      h('p', { class: 'muted small' }, `${p.email} · ${p.role}`),
      field('Name', name),
      business && field('Business name', business),
      field('Units', unitsSel),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          session.profile = await put('/me', { name: name.value, units: unitsSel.value, businessName: business ? business.value : undefined });
          toast('Saved');
        }),
      }, 'Save')),
    p.role === 'coach' && h('div', { class: 'card' },
      h('a', { class: 'btn block', href: '#/coach-profile' }, 'Edit coach profile'),
      h('div', { style: 'height:8px' }),
      h('a', { class: 'btn block', href: `#/coaches/${p.userId}` }, 'View my public page')),
    p.role === 'client' && h('div', { class: 'card' },
      h('a', { class: 'btn block', href: '#/coaches' }, 'Browse coaches'),
      h('div', { style: 'height:8px' }),
      h('a', { class: 'btn block', href: '#/intake' }, 'Intake questionnaire'),
      h('div', { style: 'height:8px' }),
      h('a', { class: 'btn block', href: '#/waiver' }, 'Health screening & waiver')),
    h('div', { class: 'card' },
      h('p', { class: 'small muted' }, pending
        ? [mine.length && `${mine.length} change(s)`, videos.length && `${videos.length} video(s)`].filter(Boolean).join(' and ') + ' waiting to sync.'
        : 'Everything is synced.'),
      h('button', { class: 'block danger', onclick: signOut }, 'Sign out')),
  );
}
