// The data agreement (privacy NFR): what PT Coach keeps, who sees it, and
// how to get it back or delete it. Ticked when accepting an invitation
// (views/onboarding.js); accounts made before it existed agree once, on
// their next sign-in (renderConsent).

import { post } from '../api.js';
import { logout } from '../auth.js';
import { $app, busy, h, mount } from '../ui.js';

/** What the agreement says, for a client or a coach. */
function terms(asCoach) {
  return asCoach
    ? [
      'PT Coach keeps what you enter to run your coaching business: your account and coach profile, your exercises and programs, and your messages and video replies to clients.',
      'Your clients’ data (workouts, measurements, health answers, photos, videos and messages) is kept for them; you can see it only while they train with you. Keep it private and use it only to coach them.',
      'If you list yourself in the coach directory, your public profile and contact email are visible to anyone.',
      'Your data is stored with Amazon Web Services in the United States. To close your account, contact the PT Coach team.',
    ]
    : [
      'PT Coach keeps what you enter to run your coaching: your workouts, notes and ratings, body measurements, health and intake answers, progress photos, exercise videos and messages.',
      'Your coach can see all of it. Nobody else can, apart from the PT Coach team when needed to keep the service running.',
      'Your data is stored with Amazon Web Services in the United States.',
      'You can download everything or delete your account at any time, from Profile → Your data.',
    ];
}

/** The agreement and its "I agree" checkbox; `agreed()` reads the tick. */
export function consentField(asCoach) {
  const box = h('input', { type: 'checkbox', id: 'consent' });
  const el = h('div', { class: 'consent' },
    h('details', {}, h('summary', {}, 'How your data is used'), h('ul', { class: 'small' }, terms(asCoach).map((t) => h('li', {}, t)))),
    h('label', { class: 'check', for: 'consent' }, box, ' I agree to how PT Coach uses my data'));
  return { el, agreed: () => box.checked };
}

/** For accounts made before the agreement: asked once, then never again. */
export function renderConsent(me, done) {
  const consent = consentField(me.profile.role === 'coach');
  consent.el.querySelector('details').open = true;
  const problem = h('p', { class: 'small', 'aria-live': 'polite' });
  mount($app(),
    h('h1', {}, 'Your data'),
    h('div', { class: 'card' },
      h('p', {}, 'Before you carry on, please read how PT Coach uses your data.'),
      consent.el,
      problem,
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!consent.agreed()) { problem.textContent = 'Tick “I agree” to continue.'; return; }
          me.profile = await post('/me/consent', {});
          done();
        }),
      }, 'Continue')),
    h('p', { class: 'center' }, h('button', { class: 'ghost', onclick: logout }, 'Sign out')),
  );
}

/** A suspended coach sees only this. */
export function renderSuspended() {
  mount($app(),
    h('h1', {}, 'Account suspended'),
    h('div', { class: 'card' }, h('p', {}, 'Your coach account has been suspended by the PT Coach team. Contact them to find out more.')),
    h('p', { class: 'center' }, h('button', { class: 'ghost', onclick: logout }, 'Sign out')),
  );
}
