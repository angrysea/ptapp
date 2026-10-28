import { myOutbox, post, put } from '../api.js';
import { pendingUploads, removeUpload } from '../uploads.js';
import { devMode, logout, MIN_LENGTH, policyProblem } from '../auth.js';
import { kv, outbox } from '../idb.js';
import { session } from '../session.js';
import { $app, busy, field, h, mount, passwordField, select, sheet, toast } from '../ui.js';

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

/** Current password, then the new one twice; Cognito checks the current one. */
function passwordSheet() {
  const current = passwordField('Current password', { autocomplete: 'current-password' });
  const next = passwordField('New password', { autocomplete: 'new-password' });
  const again = passwordField('Confirm new password', { autocomplete: 'new-password' });
  const problem = h('p', { class: 'banner', hidden: true });
  const show = (msg) => { problem.textContent = msg; problem.hidden = !msg; };
  const sh = sheet(
    h('h2', {}, 'Change password'),
    current.el, next.el, again.el,
    h('p', { class: 'small muted' }, `At least ${MIN_LENGTH} characters, with a lowercase letter and a number.`),
    problem,
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        show('');
        const bad = !current.input.value ? 'Enter your current password.'
          : policyProblem(next.input.value)
            || (next.input.value !== again.input.value ? 'The new passwords don’t match.' : null)
            || (next.input.value === current.input.value ? 'Choose a password different from your current one.' : null);
        if (bad) return show(bad);
        // The API checks the current password with Cognito (routes/password.rs).
        try {
          await post('/me/password', { current: current.input.value, new: next.input.value });
        } catch (err) {
          if (err.status !== 400) throw err;
          return show(err.message.charAt(0).toUpperCase() + err.message.slice(1) + '.');
        }
        sh.close();
        toast('Password changed');
      }),
    }, 'Change password'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
  current.input.focus();
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
    !devMode() && h('div', { class: 'card' },
      h('h3', {}, 'Password'),
      h('button', { class: 'block', onclick: passwordSheet }, 'Change password')),
    h('div', { class: 'card' },
      h('p', { class: 'small muted' }, pending
        ? [mine.length && `${mine.length} change(s)`, videos.length && `${videos.length} video(s)`].filter(Boolean).join(' and ') + ' waiting to sync.'
        : 'Everything is synced.'),
      h('button', { class: 'block danger', onclick: signOut }, 'Sign out')),
  );
}
