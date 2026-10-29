import { api, get, myOutbox, post, put } from '../api.js';
import { pendingUploads, removeUpload } from '../uploads.js';
import { devMode, logout, MIN_LENGTH, policyProblem } from '../auth.js';
import { kv, outbox } from '../idb.js';
import { disable as pushOff, enable as pushOn, needsHomeScreen, prefetchKey, previewOn, setPreview, status as pushStatus, supported as pushSupported } from '../push.js';
import { session } from '../session.js';
import { alertSettingsCard } from './alerts.js';
import { checkinSettingsCard } from './checkins.js';
import { automationCard } from './automation.js';
import { calendarCard } from './sessions.js';
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
  await pushOff(); // the next person on this device doesn't get this user's notifications
  logout();
}

/** A client's own data (privacy): download all of it, or delete the account. */
function yourDataCard(p) {
  const download = h('button', {
    class: 'block',
    onclick: (e) => busy(e.currentTarget, async () => {
      const data = await get('/me/export');
      const a = h('a', {
        href: URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' })),
        download: `pt-coach-data-${new Date().toISOString().slice(0, 10)}.json`,
      });
      document.body.append(a);
      a.click();
      a.remove();
      toast('Downloaded. Links to your photos and videos in it work for 15 minutes.');
    }),
  }, 'Download my data');
  const remove = h('button', { class: 'block danger', onclick: () => deleteSheet(p) }, 'Delete my account');
  return h('div', { class: 'card' },
    h('h3', {}, 'Your data'),
    h('p', { class: 'small muted' }, 'Download everything stored about you, or delete your account and all of it.'),
    download, h('div', { style: 'height:8px' }), remove);
}

function deleteSheet(p) {
  const confirmIn = h('input', { type: 'email', autocomplete: 'off', placeholder: p.email });
  const problem = h('p', { class: 'banner', hidden: true });
  const sh = sheet(
    h('h2', {}, 'Delete your account?'),
    h('p', {}, 'This permanently deletes your account and everything in it: workouts, measurements, photos, videos, messages and your coach’s replies. Your coach will no longer see you. It can’t be undone.'),
    h('p', { class: 'small muted' }, 'You may want to download your data first.'),
    field('Type your email to confirm', confirmIn),
    problem,
    h('button', {
      class: 'danger block',
      onclick: (e) => busy(e.currentTarget, async () => {
        problem.hidden = true;
        try {
          await api('DELETE', '/me/account', { confirm: confirmIn.value });
        } catch (err) {
          problem.textContent = err.status === 400 ? 'That isn’t your email address.' : err.message;
          problem.hidden = false;
          return;
        }
        sh.close();
        // Nothing of this account stays on the device.
        await kv.clear();
        for (const i of await myOutbox()) await outbox.del(i.key);
        for (const v of await pendingUploads()) await removeUpload(v.key);
        await pushOff();
        try { localStorage.clear(); sessionStorage.setItem('pt.accountDeleted', '1'); } catch { /* nothing stored */ }
        logout(); // the home page then says the account was deleted
      }),
    }, 'Delete my account'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
}

const BLOCKED = 'Notifications are blocked for this app. Allow them in your browser’s site settings (on iPhone: Settings → Notifications → PT Coach), then come back.';

/** Push notifications for this device (push.js): on/off, and whether they show the message text. */
function notificationsCard() {
  const card = h('div', { class: 'card' }, h('h3', {}, 'Notifications'));
  if (needsHomeScreen()) {
    card.append(
      h('p', {}, 'On iPhone and iPad, notifications only work in the app on your Home Screen.'),
      h('ol', { class: 'small' },
        h('li', {}, 'In Safari, tap the Share button.'),
        h('li', {}, 'Choose “Add to Home Screen”.'),
        h('li', {}, 'Open PT Coach from your Home Screen and turn notifications on here.')));
    return card;
  }
  if (!pushSupported()) {
    card.append(h('p', { class: 'small muted' }, 'This browser can’t show notifications from the app.'));
    return card;
  }
  prefetchKey();
  const on = h('input', { type: 'checkbox' });
  const preview = h('input', { type: 'checkbox', checked: previewOn() });
  const note = h('p', { class: 'small muted' });
  const draw = (state) => {
    on.checked = state === 'on';
    on.disabled = state === 'denied';
    preview.disabled = state !== 'on';
    note.textContent = state === 'denied' ? BLOCKED
      : state === 'unavailable' ? 'Notifications aren’t set up on this server yet.'
        : state === 'on' ? 'You’ll be notified about new messages and video replies on this device.' : '';
  };
  on.addEventListener('change', async () => {
    on.disabled = true;
    try {
      draw(on.checked ? await pushOn() : (await pushOff(), 'off'));
    } catch (e) {
      draw(await pushStatus());
      toast(`Couldn’t change notifications: ${e.message}`, 'error');
    }
  });
  preview.addEventListener('change', () => setPreview(preview.checked).catch(() => toast('Couldn’t save that; try again when online', 'error')));
  pushStatus().then(draw);
  card.append(
    h('label', { class: 'check' }, on, 'Notify me about new messages and video replies'),
    h('label', { class: 'check' }, preview, 'Show the message text in notifications'),
    note);
  return card;
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
    p.role !== 'admin' && notificationsCard(),
    p.role !== 'admin' && calendarCard(),
    p.role === 'coach' && alertSettingsCard(),
    p.role === 'coach' && checkinSettingsCard(),
    p.role === 'coach' && automationCard(),
    p.role === 'coach' && timeCard(),
    p.role === 'client' && yourDataCard(p),
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

/** The coach's own time per client (counted while on a client's pages). */
function timeCard() {
  const box = h('div', { class: 'card' }, h('h3', {}, 'Your coaching time'));
  get('/usage?weeks=4').then((u) => {
    const week = u.weeks[u.weeks.length - 1];
    box.append(
      h('p', {}, `This week: ${week.minutes} min across ${week.clientsSeen} client${week.clientsSeen === 1 ? '' : 's'}.`),
      u.avgMinutesPerClientPerWeek != null && h('p', { class: 'muted small' }, `Last 4 weeks: ${u.avgMinutesPerClientPerWeek} min per client per week. Counted while you’re active on a client’s pages.`));
  }).catch(() => box.append(h('p', { class: 'muted small' }, 'Couldn’t load your time.')));
  return box;
}
