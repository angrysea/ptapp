// First run after signing in, before the user has an account here: accept
// an invite (clients from their coach, coaches from an admin), or set up the
// admin account. There is no open coach signup; accounts come from emailed
// invitations (views/signup.js is the page an invitation link opens).

import { get, post } from '../api.js';
import { logout } from '../auth.js';
import { $app, busy, errorToast, field, h, mount } from '../ui.js';
import { consentField } from './consent.js';

const PENDING = 'pt.pendingInvite';

const reloadHome = (hash = '#/') => { location.hash = hash; location.reload(); };

/** Remember an invite code (#/join/CODE) across the sign-up and sign-in redirects. */
export function captureInvite() {
  const m = location.hash.match(/^#\/join\/([A-Za-z0-9]+)/);
  if (m) localStorage.setItem(PENDING, m[1].toUpperCase());
}

export async function renderOnboarding(me) {
  const pending = localStorage.getItem(PENDING);
  if (pending) return renderJoin(pending, me);

  const code = h('input', { placeholder: 'e.g. K7MPQ2RXTA', autocapitalize: 'characters' });
  mount($app(),
    h('h1', {}, 'Welcome'),
    h('p', { class: 'muted' }, `Signed in as ${me.email}`),
    me.adminEligible && adminCard(),
    h('div', { class: 'card' },
      h('h2', {}, 'I have an invite'),
      h('p', { class: 'muted small' }, 'Use the link in your invitation email, or enter its code.'),
      field('Invite code', code),
      h('button', { class: 'primary block', onclick: () => code.value.trim() && renderJoin(code.value.trim().toUpperCase(), me) }, 'Continue')),
    h('p', { class: 'small muted' }, 'Accounts are by invitation. Coaches are invited by the PT Coach team; clients by their coach.'),
    h('p', { class: 'center' }, h('button', { class: 'ghost', onclick: logout }, 'Sign out')),
  );
}

/** Only shown to verified emails on the server's ADMIN_EMAILS list. */
function adminCard() {
  const name = h('input', { autocomplete: 'name' });
  return h('div', { class: 'card' },
    h('h2', {}, 'Admin'),
    h('p', { class: 'muted small' }, 'This email can set up the admin account, which invites coaches.'),
    field('Your name', name),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await post('/admin/register', { name: name.value });
        reloadHome();
      }),
    }, 'Set up admin account'));
}

async function renderJoin(code, me) {
  let invite;
  try {
    invite = await get(`/invites/${encodeURIComponent(code)}`);
  } catch (e) {
    localStorage.removeItem(PENDING);
    errorToast(e.status === 404 ? 'That invite is invalid, expired or already used.' : e);
    return renderOnboarding(me);
  }
  const asCoach = invite.kind === 'coach';
  const name = h('input', { value: invite.clientName || '', autocomplete: 'name' });
  // The API only lets the invited address accept; say so up front.
  const wrongEmail = (me.email || '').toLowerCase() !== invite.email.toLowerCase();
  const consent = consentField(asCoach);
  const problem = h('p', { class: 'small', 'aria-live': 'polite' });
  mount($app(),
    h('h1', {}, asCoach ? `Coach on ${invite.coachName}` : `Join ${invite.coachName}`),
    h('div', { class: 'card' },
      h('p', {}, asCoach
        ? `You’ve been invited to set up a coach account for ${invite.email}.`
        : `${invite.coachName} invited ${invite.email} to train with them.`),
      wrongEmail && h('p', { class: 'banner' },
        `You’re signed in as ${me.email}. Sign out and sign in with ${invite.email} to accept.`),
      wrongEmail && h('button', { class: 'block', onclick: logout }, 'Sign out'),
      !wrongEmail && field('Your name', name),
      !wrongEmail && consent.el,
      !wrongEmail && problem,
      !wrongEmail && h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!consent.agreed()) { problem.textContent = 'Tick “I agree” to continue.'; return; }
          await post(`/invites/${encodeURIComponent(code)}/accept`, { name: name.value, consent: true });
          localStorage.removeItem(PENDING);
          reloadHome(asCoach ? '#/' : '#/intake?onboarding=1');
        }),
      }, 'Accept invite'),
      h('button', { class: 'ghost block', onclick: () => { localStorage.removeItem(PENDING); renderOnboarding(me); } }, 'Use a different code')),
  );
}
