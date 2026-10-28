// App bootstrap: auth, profile, role-based tabs and routes.

import { flush, get, onSync, put } from './api.js';
import { handleCallback, login, signedIn } from './auth.js';
import { kv } from './idb.js';
import { kick as resumeUploads } from './uploads.js';
import { route, start } from './router.js';
import { session } from './session.js';
import { $app, errorToast, h, mount, toast } from './ui.js';
import { renderAdmin } from './views/admin.js';
import { renderClient, renderRoster } from './views/coach.js';
import { loadExercises, renderExercises } from './views/exercises.js';
import { renderIntake, renderWaiver } from './views/intake.js';
import { renderMetrics } from './views/metrics.js';
import { COACH_REQUEST_PATH, captureInvite, renderOnboarding } from './views/onboarding.js';
import { renderThread } from './views/comments.js';
import { renderProfile } from './views/profile.js';
import { renderProgramEditor, renderPrograms } from './views/programs.js';
import { refreshVideoBadge, renderVideoQueue } from './views/videos.js';
import { renderToday, renderWorkout } from './views/workout.js';

const OWNER = 'pt.owner';

if ('serviceWorker' in navigator) navigator.serviceWorker.register('sw.js').catch(() => {});

function tabs(items) {
  const nav = document.getElementById('tabs');
  nav.hidden = false;
  mount(nav, items.map(([path, ico, label]) => h('a', { href: `#${path}`, 'data-path': path }, h('span', { class: 'ico' }, ico), label)));
  return (current) => {
    for (const a of nav.querySelectorAll('a')) {
      const p = a.dataset.path;
      a.classList.toggle('active', p === '/' ? current === '/' || /^\/(workout|c)\//.test(current) : current.startsWith(p));
    }
  };
}

function landing() {
  mount($app(),
    h('div', { class: 'center', style: 'padding-top:12vh' },
      h('img', { src: 'icon.svg', alt: '', width: 88, height: 88 }),
      h('h1', {}, 'PT Coach'),
      h('p', { class: 'muted' }, 'Train with your coach: workouts, progress and feedback in one place.'),
      h('button', { class: 'primary', onclick: login }, 'Sign in or create account'),
      location.hash.startsWith('#/join/') && h('p', { class: 'small muted' }, 'You’ve been invited. Create an account with the email your invite was sent to.'),
      location.hash === `#${COACH_REQUEST_PATH}` && h('p', { class: 'small muted' }, 'Coaches: create an account (or sign in), then send us a short request.')));
}

async function boot() {
  captureInvite();
  try { await handleCallback(); } catch (e) { errorToast(e); }
  if (!signedIn()) return landing();

  let me;
  try {
    me = await get('/me', { cache: true });
  } catch (e) {
    if (e.status === 401) return landing();
    return mount($app(), h('p', { class: 'center muted' }, 'Can’t reach the server and nothing is cached yet. Check your connection.'));
  }
  // Cached reads, drafts and unsynced copies belong to whoever was signed in.
  // A different user on this device must not see them (queued writes are
  // kept per user in the outbox, see api.js).
  try {
    if (localStorage.getItem(OWNER) !== me.userId) {
      if (localStorage.getItem(OWNER)) await kv.clear();
      localStorage.setItem(OWNER, me.userId);
    }
  } catch { /* storage unavailable: nothing cached to leak */ }
  if (!me.profile) return renderOnboarding(me);
  session.profile = me.profile;
  // The server judges "today" (missed workouts, adherence) in the user's own
  // time zone, so keep it current; travel or a DST rule change updates it.
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
  if (tz && me.profile.timeZone !== tz) put('/me', { timeZone: tz }).then((p) => { session.profile = p; }).catch(() => {});

  const syncEl = document.getElementById('sync');
  onSync((n, failure) => {
    if (failure) toast(`A change couldn’t be saved: ${failure.error.message}`, 'error');
    if (n == null) return;
    syncEl.hidden = n === 0 && navigator.onLine;
    syncEl.textContent = !navigator.onLine ? `Offline${n ? ` · ${n} queued` : ''}` : `Syncing ${n}…`;
  });
  window.addEventListener('offline', () => flush());
  flush();
  resumeUploads(); // set videos queued earlier carry on from where they stopped
  loadExercises().catch(() => {});

  let highlight;
  if (session.isAdmin) {
    highlight = tabs([['/', '🧑‍🏫', 'Coaches'], ['/me', '⚙️', 'Profile']]);
    route('/', renderAdmin);
  } else if (session.isCoach) {
    document.getElementById('title').textContent = me.profile.businessName || 'PT Coach';
    highlight = tabs([['/', '👥', 'Clients'], ['/videos', '🎥', 'Videos'], ['/programs', '🗓️', 'Programs'], ['/exercises', '🏋️', 'Exercises'], ['/me', '⚙️', 'Profile']]);
    refreshVideoBadge();
    route('/', renderRoster);
    route('/c/:id', (id) => renderClient(id));
    route('/c/:id/:tab', (id, tab) => renderClient(id, tab));
    route('/programs', renderPrograms);
    route('/programs/:id', renderProgramEditor);
    route('/exercises', renderExercises);
    route('/videos', renderVideoQueue);
  } else {
    highlight = tabs([['/', '🏋️', 'Today'], ['/progress', '📈', 'Progress'], ['/coach', '💬', 'Coach'], ['/me', '⚙️', 'Profile']]);
    route('/', renderToday);
    route('/workout/:date/:id', renderWorkout);
    route('/progress', () => renderMetrics(mount($app(), h('h1', {}, 'Progress'), h('div')).lastChild, session.profile));
    route('/coach', () => { const el = h('div'); mount($app(), h('h1', {}, 'Messages'), el); return renderThread(el, session.userId); });
    route('/intake', renderIntake);
    route('/waiver', renderWaiver);
  }
  route('/me', renderProfile);
  start({ home: '/', afterRender: highlight });
}

boot();
