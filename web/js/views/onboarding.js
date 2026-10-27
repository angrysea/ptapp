// First run: join a coach with an invite code (ON-1) or register as a coach.

import { get, post } from '../api.js';
import { logout } from '../auth.js';
import { $app, busy, errorToast, field, h, mount } from '../ui.js';

const PENDING = 'pt.pendingInvite';

/** Remember an invite code from a #/join/CODE link across the login redirect. */
export function captureInvite() {
  const m = location.hash.match(/^#\/join\/([A-Za-z0-9]+)/);
  if (m) localStorage.setItem(PENDING, m[1].toUpperCase());
}

export async function renderOnboarding(me) {
  const pending = localStorage.getItem(PENDING);
  if (pending) return renderJoin(pending, me);

  const code = h('input', { placeholder: 'e.g. K7MPQ2RXTA', autocapitalize: 'characters' });
  const coachName = h('input', { autocomplete: 'name' });
  const business = h('input', {});
  mount($app(),
    h('h1', {}, 'Welcome'),
    h('p', { class: 'muted' }, `Signed in as ${me.email}`),
    h('div', { class: 'card' },
      h('h2', {}, 'I’m a client'),
      h('p', { class: 'muted small' }, 'Use the invite link or code your coach sent you.'),
      field('Invite code', code),
      h('button', { class: 'primary block', onclick: () => code.value.trim() && renderJoin(code.value.trim().toUpperCase(), me) }, 'Continue')),
    h('div', { class: 'card' },
      h('h2', {}, 'I’m a coach'),
      field('Your name', coachName),
      field('Business name (optional)', business),
      h('button', {
        class: 'block',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!coachName.value.trim()) throw new Error('Enter your name');
          await post('/coach/register', { name: coachName.value, businessName: business.value || null });
          location.hash = '#/';
          location.reload();
        }),
      }, 'Create coach account')),
    h('p', { class: 'center' }, h('button', { class: 'ghost', onclick: logout }, 'Sign out')),
  );
}

async function renderJoin(code, me) {
  let invite;
  try {
    invite = await get(`/invites/${encodeURIComponent(code)}`);
  } catch (e) {
    localStorage.removeItem(PENDING);
    errorToast(e.status === 404 ? 'That invite code is invalid, expired or already used.' : e);
    return renderOnboarding(me);
  }
  const name = h('input', { value: invite.clientName || '', autocomplete: 'name' });
  // The API only lets the invited address accept; say so up front.
  const wrongEmail = (me.email || '').toLowerCase() !== invite.email.toLowerCase();
  mount($app(),
    h('h1', {}, `Join ${invite.coachName}`),
    h('div', { class: 'card' },
      h('p', {}, `${invite.coachName} invited ${invite.email} to train with them.`),
      wrongEmail && h('p', { class: 'banner' },
        `You’re signed in as ${me.email}. Sign out and create an account with ${invite.email} to accept.`),
      wrongEmail && h('button', { class: 'block', onclick: logout }, 'Sign out'),
      !wrongEmail && field('Your name', name),
      !wrongEmail && h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          await post(`/invites/${encodeURIComponent(code)}/accept`, { name: name.value });
          localStorage.removeItem(PENDING);
          location.hash = '#/intake?onboarding=1';
          location.reload();
        }),
      }, 'Accept invite'),
      h('button', { class: 'ghost block', onclick: () => { localStorage.removeItem(PENDING); renderOnboarding(me); } }, 'Use a different code')),
  );
}
