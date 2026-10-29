// App bootstrap: auth, profile, role-based tabs and routes.

import { flush, get, onSync, put } from './api.js';
import { handleCallback, signedIn } from './auth.js';
import { kv } from './idb.js';
import { kick as resumeUploads } from './uploads.js';
import { route, start } from './router.js';
import { session } from './session.js';
import { $app, errorToast, h, mount, toast } from './ui.js';
import { renderAdmin } from './views/admin.js';
import { renderClient, renderRoster } from './views/coach.js';
import { renderCoachSetup } from './views/coachprofile.js';
import { renderConsent, renderSuspended } from './views/consent.js';
import { renderCoachPage, renderDirectory } from './views/directory.js';
import { renderRequest } from './views/request.js';
import { refreshMessageBadge, renderInbox } from './views/messages.js';
import { renderReview } from './views/review.js';
import { onLive, startLive } from './realtime.js';
import { disable as pushOff, resync as resyncPush } from './push.js';
import { loadExercises, renderExercises } from './views/exercises.js';
import { renderIntake, renderWaiver } from './views/intake.js';
import { renderMetrics } from './views/metrics.js';
import { captureInvite, renderOnboarding } from './views/onboarding.js';
import { landing } from './views/landing.js';
import { renderInviteSignedIn, renderInviteSignUp, resumeAfterSignOut } from './views/signup.js';
import { renderThread } from './views/comments.js';
import { renderCheckin } from './views/checkins.js';
import { renderAutomation } from './views/automation.js';
import { coachHours, renderSchedule, renderSessions } from './views/sessions.js';
import { renderReport } from './views/report.js';
import { startUsage } from './usage.js';
import { renderProfile, signOut } from './views/profile.js';
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
      a.classList.toggle('active', p === '/' ? current === '/' || /^\/(workout|c)\//.test(current)
        : current === p || current.startsWith(`${p}/`) || (p === '/videos' && current.startsWith('/review/')));
    }
  };
}

async function boot() {
  resumeAfterSignOut();
  captureInvite();
  try { await handleCallback(); } catch (e) { errorToast(e); }
  if (!signedIn()) {
    // An invitation link opens the sign-up page, with the invited email filled in.
    const invite = location.hash.match(/^#\/join\/([A-Za-z0-9]+)/);
    if (invite) return renderInviteSignUp(invite[1].toUpperCase());
    // Signed out: the landing page, the public coach directory, and a
    // coach's own sign-up page (the link on their website).
    route('/', landing);
    route('/coaches', renderDirectory);
    route('/coaches/:id', renderCoachPage);
    route('/request/:id', renderRequest);
    return start({ home: '/' });
  }

  const out = document.getElementById('signout');
  out.hidden = false;
  out.onclick = () => signOut().catch(errorToast);

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
      if (localStorage.getItem(OWNER)) {
        await kv.clear();
        // The browser's push subscription was the previous user's; this user
        // turns notifications on for themselves if they want them.
        await pushOff();
      }
      localStorage.setItem(OWNER, me.userId);
    }
  } catch { /* storage unavailable: nothing cached to leak */ }
  // An invitation link opened while signed in (for example as the admin who sent it).
  const invite = location.hash.match(/^#\/join\/([A-Za-z0-9]+)/);
  if (invite && await renderInviteSignedIn(invite[1].toUpperCase(), me)) return;
  if (!me.profile) return renderOnboarding(me);
  if (me.profile.suspendedAt) return renderSuspended();
  // Accounts made before the data agreement existed agree to it once.
  if (me.profile.role !== 'admin' && !me.profile.consentedAt) return renderConsent(me, () => location.reload());
  session.profile = me.profile;
  // The header carries the coach's brand, for the coach and their clients alike.
  const brand = session.isCoach ? me.profile : me.coach;
  if (brand) document.getElementById('title').textContent = brand.businessName || brand.name || 'Personal Training Coach';
  // Coaches fill in their profile before anything else (tasks/coaches).
  if (session.isCoach && me.coachProfileComplete === false) return renderCoachSetup({ onboarding: true });
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
    highlight = tabs([['/', '🧑‍🏫', 'Coaches'], ['/coaches', '🔎', 'Directory'], ['/me', '⚙️', 'Profile']]);
    route('/', renderAdmin);
  } else if (session.isCoach) {
    highlight = tabs([['/', '👥', 'Clients'], ['/messages', '💬', 'Messages'], ['/videos', '🎥', 'Videos'], ['/programs', '🗓️', 'Programs'], ['/exercises', '🏋️', 'Exercises'], ['/me', '⚙️', 'Profile']]);
    refreshVideoBadge();
    startUsage(); // time per client (Phase 2 exit gate)
    route('/', renderRoster);
    route('/c/:id/report', (id) => renderReport(id)); // before /c/:id/:tab, which would match it too
    route('/c/:id', (id) => renderClient(id));
    route('/c/:id/:tab', (id, tab) => renderClient(id, tab));
    route('/c/:id/:tab/:focus', (id, tab, focus) => renderClient(id, tab, focus));
    route('/programs', renderPrograms);
    route('/programs/:id', renderProgramEditor);
    route('/exercises', renderExercises);
    route('/videos', renderVideoQueue);
    route('/messages', renderInbox);
    route('/review/:cid/:vid', renderReview);
    route('/automation', renderAutomation);
    route('/schedule', renderSchedule);
  } else {
    highlight = tabs([['/', '🏋️', 'Today'], ['/progress', '📈', 'Progress'], ['/coach', '💬', 'Coach'], ['/me', '⚙️', 'Profile']]);
    route('/', renderToday);
    route('/workout/:date/:id', renderWorkout);
    route('/progress', () => renderMetrics(mount($app(), h('div', { class: 'spread page-head' }, h('h1', {}, 'Progress'), h('a', { class: 'btn small', href: '#/report' }, '📄 Report')), h('div')).lastChild, session.profile));
    route('/coach', () => {
      const el = h('div');
      const hours = h('div');
      mount($app(), h('h1', {}, 'Messages'), hours, el);
      coachHours(hours); // MS-5: when the coach is around (online only)
      return renderThread(el, session.userId);
    });
    route('/intake', renderIntake);
    route('/checkin', renderCheckin);
    route('/sessions', renderSessions);
    route('/report', () => renderReport(session.userId));
    route('/waiver', renderWaiver);
  }
  // Unread messages: badges now and whenever a message arrives (realtime.js).
  if (!session.isAdmin) {
    startLive(session.userId);
    resyncPush(); // the server keeps this device's push subscription current
    refreshMessageBadge();
    onLive(() => refreshMessageBadge());
  }
  route('/me', renderProfile);
  route('/coaches', renderDirectory);
  route('/coaches/:id', renderCoachPage);
  route('/request/:id', renderRequest);
  if (session.isCoach) route('/coach-profile', () => renderCoachSetup());
  start({ home: '/', afterRender: highlight });
}

boot();
