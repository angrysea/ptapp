// Public landing: the first screen for anyone not signed in. Every account
// starts from an invitation, so this page's job is to say what an account
// gives you and point at the door that exists (sign in, invitation link,
// the public coach directory). Keep the "Find a coach" link — the browser
// journey (tests/ui/journey.py) clicks it.

import { login } from '../auth.js';
import cfg from '../config.js';
import { $app, busy, field, h, mount, toast } from '../ui.js';

// Public call, like the directory's plain fetch: the landing page works
// signed out, so there is no token to attach. Returns the body, `{ problem }`
// with the server's message for a refused request, or null when unreachable.
async function pubPost(path, body) {
  let res;
  try {
    res = await fetch(`${cfg.apiUrl}/public${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
  } catch { return null; } // offline or the API is down
  const out = await res.json().catch(() => null);
  if (res.status === 400) return { problem: out?.error || 'Check what you typed and try again.' };
  return res.ok ? out : null;
}

// One card per feature area the MVP actually ships (README table); roadmap
// items stay out until they exist, and each line is one breath.
const FEATURES = [
  ['🗓️', 'A program per client',
   'Your coach builds it once: sets, reps, loads and tempos in straight, superset, circuit, AMRAP, EMOM and interval blocks, laid over a multi-week calendar — copy a week, move a workout, reuse a template.'],
  ['🏋️', 'Logging works offline',
   'Each set is logged as it happens — reps, load and RPE pre-filled from the plan, rest and interval timers that beep. Do the workout in a park with no signal: it queues and syncs when you are back.'],
  ['👥', 'The coach sees the whole story',
   'Per client: 28-day adherence, missed days, weekly volume and e1RM trends; check-ins on soreness, sleep and pain — with flags for who needs attention.'],
  ['🎥', 'Feedback, not group chat',
   'Comments on a whole workout or one exact set; film a set in the app and the coach reviews it oldest first; weight, tape and body-fat trends keep progress honest over the year.'],
];

export function landing() {
  // A previous session ended by signing out of a deleted account; say so once
  // (views/profile.js sets the flag on that page's sign-out).
  let deleted = false;
  try { deleted = sessionStorage.getItem('pt.accountDeleted') === '1'; sessionStorage.removeItem('pt.accountDeleted'); } catch { /* no storage */ }
  mount($app(),
    deleted && h('p', { class: 'banner' }, 'Your account and everything in it have been deleted.'),
    h('div', { class: 'center', style: 'padding-top:7vh' },
      h('img', { src: 'icon.svg', alt: '', width: 88, height: 88 }),
      h('h1', {}, 'PT Coach'),
      h('p', { class: 'muted' }, 'Train with your coach: workouts, progress and feedback in one place.')),
    h('h2', {}, 'What an account gives you'),
    h('div', { class: 'landing-grid' }, FEATURES.map(([ico, title, copy]) =>
      h('div', { class: 'card' }, h('h3', {}, ico, ' ', title), h('p', { class: 'small' }, copy)))),
    h('h2', {}, 'How it starts'),
    h('div', { class: 'landing-split' },
      h('div', {},
        h('h3', {}, 'New coach'),
        h('p', { class: 'small' },
          'Coaches don’t sign up on their own: someone who runs the club vouches for you and sends an invitation tied to your email — the link fills itself in. Then two things before anything else: finish your profile, and invite your first client.'),
        h('p', { class: 'small muted' }, 'No link yet? Ask below: your email lands in the club admin’s queue, and approving it sends you that invitation.'),
        requestForm()),
      h('div', {},
        h('h3', {}, 'New client'),
        h('p', { class: 'small' },
          'Your coach sends the same kind of link. After that, one screen does the job: Today shows what’s scheduled, the timers count your rest, and the workout syncs when you say it’s done.'))),
    h('p', { class: 'small muted' },
      'Every account is apart and private: another coach’s clients never appear in your app, photos and videos live in private buckets, and when a coach opens sensitive client data, the access is logged.'),
    h('button', { class: 'primary', onclick: () => login() }, 'Sign in'),
    h('p', {}, h('a', { class: 'btn', href: '#/coaches' }, 'Find a coach')),
    h('p', { class: 'small muted' }, 'New here? Accounts are by invitation: use the link in your invitation email.'));
}

/** The "ask to join" box: one record per email lands in the admin's queue. */
function requestForm() {
  const shownAt = Date.now(); // the server drops sends faster than a person types
  const email = h('input', { type: 'email', autocomplete: 'off', placeholder: 'you@gym.com' });
  const name = h('input', { autocomplete: 'off', placeholder: 'gym or coach name' });
  // Hidden from people (and screen readers); form-filling scripts fill it.
  const website = h('input', { name: 'website', tabindex: '-1', autocomplete: 'off' });
  const problem = h('p', { class: 'small', 'aria-live': 'polite' });
  return [
    field('Email', email),
    field('Coach or gym name', name),
    h('div', { class: 'hp', 'aria-hidden': 'true' }, h('label', {}, 'Leave this empty', website)),
    problem,
    h('button', {
      class: 'grow',
      onclick: (e) => busy(e.currentTarget, async () => {
        const value = email.value.trim();
        if (!value.includes('@')) { problem.textContent = 'Type the email you want to be invited with.'; return; }
        problem.textContent = '';
        const r = await pubPost('/coach-requests', { email: value, name: name.value, website: website.value, elapsedMs: Date.now() - shownAt });
        if (!r) { problem.textContent = 'Couldn’t reach the club right now; try again in a bit.'; return; }
        if (r.problem) { problem.textContent = r.problem.charAt(0).toUpperCase() + r.problem.slice(1); return; }
        // The same answer for everyone: the form never says who was invited.
        toast('Thanks. If the club approves you, an invitation arrives at that email.');
      }),
    }, 'Request an invitation'),
  ];
}
