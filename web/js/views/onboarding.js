// First run, before the user has an account: join with an invite (clients
// from their coach, coaches from an admin), request a coach account, or set
// up the admin account. There is no open coach signup.

import { get, post, put } from '../api.js';
import { logout } from '../auth.js';
import { $app, busy, errorToast, field, fmtDateTime, h, mount } from '../ui.js';

const PENDING = 'pt.pendingInvite';
const COACH_INTENT = 'pt.coachIntent';
/** Shareable link to the coach request page: <app>#/become-a-coach */
export const COACH_REQUEST_PATH = '/become-a-coach';

const reloadHome = (hash = '#/') => { location.hash = hash; location.reload(); };

/** Remember an invite code (#/join/CODE) or a coach-request link across the
 * login redirect. */
export function captureInvite() {
  const m = location.hash.match(/^#\/join\/([A-Za-z0-9]+)/);
  if (m) localStorage.setItem(PENDING, m[1].toUpperCase());
  if (location.hash === `#${COACH_REQUEST_PATH}`) localStorage.setItem(COACH_INTENT, '1');
}

export async function renderOnboarding(me) {
  const pending = localStorage.getItem(PENDING);
  if (pending) return renderJoin(pending, me);
  if (localStorage.getItem(COACH_INTENT) || me.coachRequest) return renderCoachRequest(me);

  const code = h('input', { placeholder: 'e.g. K7MPQ2RXTA', autocapitalize: 'characters' });
  mount($app(),
    h('h1', {}, 'Welcome'),
    h('p', { class: 'muted' }, `Signed in as ${me.email}`),
    me.adminEligible && adminCard(),
    h('div', { class: 'card' },
      h('h2', {}, 'I have an invite'),
      h('p', { class: 'muted small' }, 'Use the invite link or code you were sent.'),
      field('Invite code', code),
      h('button', { class: 'primary block', onclick: () => code.value.trim() && renderJoin(code.value.trim().toUpperCase(), me) }, 'Continue')),
    h('div', { class: 'card' },
      h('h2', {}, 'I’m a coach'),
      h('p', { class: 'muted small' }, 'Coach accounts are set up by our team. Tell us about yourself and we’ll review your request.'),
      h('button', { class: 'block', onclick: () => renderCoachRequest(me) }, 'Request a coach account')),
    h('p', { class: 'center' }, h('button', { class: 'ghost', onclick: logout }, 'Sign out')),
  );
}

/** Only shown to verified emails on the server's ADMIN_EMAILS list. */
function adminCard() {
  const name = h('input', { autocomplete: 'name' });
  return h('div', { class: 'card' },
    h('h2', {}, 'Admin'),
    h('p', { class: 'muted small' }, 'This email can set up the admin account, which onboards coaches.'),
    field('Your name', name),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await post('/admin/register', { name: name.value });
        reloadHome();
      }),
    }, 'Set up admin account'));
}

function renderCoachRequest(me) {
  const r = me.coachRequest;
  const name = h('input', { value: r?.name || '', autocomplete: 'name' });
  const business = h('input', { value: r?.businessName || '' });
  const message = h('textarea', { value: r?.message || '', rows: 5, maxlength: 2000,
    placeholder: 'Your experience, certifications, and how many clients you coach.' });
  const status = r?.status === 'pending'
    ? h('div', { class: 'banner' }, `Your request was sent ${fmtDateTime(r.createdAt)} and is being reviewed. We’ll set up your coach account when it’s approved; check back here. You can still update it below.`)
    : r?.status === 'declined'
      ? h('div', { class: 'banner' }, 'Your previous request wasn’t approved. You can send a new one.')
      : null;
  const back = () => { localStorage.removeItem(COACH_INTENT); renderOnboarding({ ...me, coachRequest: null }); };
  mount($app(),
    h('h1', {}, 'Request a coach account'),
    h('p', { class: 'muted' }, `Signed in as ${me.email}. Your coach account will use this email.`),
    status,
    h('div', { class: 'card' },
      field('Your name', name),
      field('Business name (optional)', business),
      field('About your coaching', message),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (!name.value.trim()) throw new Error('Enter your name');
          const saved = await put('/coach-request', { name: name.value, businessName: business.value || null, message: message.value });
          localStorage.removeItem(COACH_INTENT);
          renderCoachRequest({ ...me, coachRequest: saved });
        }),
      }, r?.status === 'pending' ? 'Update request' : 'Send request')),
    h('p', { class: 'center' },
      h('button', { class: 'ghost', onclick: back }, 'I have an invite code instead'), ' ',
      h('button', { class: 'ghost', onclick: logout }, 'Sign out')),
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
  const asCoach = invite.kind === 'coach';
  const name = h('input', { value: invite.clientName || '', autocomplete: 'name' });
  // The API only lets the invited address accept; say so up front.
  const wrongEmail = (me.email || '').toLowerCase() !== invite.email.toLowerCase();
  mount($app(),
    h('h1', {}, asCoach ? `Coach on ${invite.coachName}` : `Join ${invite.coachName}`),
    h('div', { class: 'card' },
      h('p', {}, asCoach
        ? `You’ve been invited to set up a coach account for ${invite.email}.`
        : `${invite.coachName} invited ${invite.email} to train with them.`),
      wrongEmail && h('p', { class: 'banner' },
        `You’re signed in as ${me.email}. Sign out and create an account with ${invite.email} to accept.`),
      wrongEmail && h('button', { class: 'block', onclick: logout }, 'Sign out'),
      !wrongEmail && field('Your name', name),
      !wrongEmail && h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          await post(`/invites/${encodeURIComponent(code)}/accept`, { name: name.value });
          localStorage.removeItem(PENDING);
          reloadHome(asCoach ? '#/' : '#/intake?onboarding=1');
        }),
      }, 'Accept invite'),
      h('button', { class: 'ghost block', onclick: () => { localStorage.removeItem(PENDING); renderOnboarding(me); } }, 'Use a different code')),
  );
}
