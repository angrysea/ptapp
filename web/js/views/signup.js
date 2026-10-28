// The invitation sign-up page (ON-1). An invite link (#/join/CODE) opened
// while signed out lands here: the invitation is read from the API, its email
// is filled in and locked, and the account is created with Cognito directly.
// Then Cognito's sign-in page opens with that email filled in; once signed
// in, the onboarding screen accepts the invitation, and the link stops
// working. Coaches only ever arrive this way (no open coach signup).

import cfg from '../config.js';
import { confirmSignUp, devMode, login, logout, MIN_LENGTH, policyProblem, resendSignUpCode, signUp } from '../auth.js';
import { $app, busy, field, h, mount, passwordField } from '../ui.js';


/** The open invitation behind `code`, read without signing in; null if it's invalid, used or expired. */
async function publicInvite(code) {
  const res = await fetch(`${cfg.apiUrl}/invites/${encodeURIComponent(code)}/public`);
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`Couldn’t load the invitation (${res.status}). Check your connection and try again.`);
  return res.json();
}

// Cognito's sign-out can only return to the app's plain address, so the
// invitation to resume is kept for this tab across it (see resumeAfterSignOut).
const RESUME = 'pt.resumeAfterSignOut';

/** After signing out to use an invitation: puts its link back in the address. */
export function resumeAfterSignOut() {
  try {
    const hash = sessionStorage.getItem(RESUME);
    sessionStorage.removeItem(RESUME);
    if (hash) history.replaceState(null, '', hash);
  } catch { /* storage unavailable: the person reopens the link */ }
}

/**
 * An invitation link opened while already signed in, e.g. an admin testing
 * the email on their own device. Returns false when the invitation is this
 * account's own and still open, so normal onboarding accepts it; otherwise
 * shows what to do and returns true.
 */
export async function renderInviteSignedIn(code, me) {
  const back = h('button', { class: 'block', onclick: () => { location.hash = '#/'; location.reload(); } }, 'Back to the app');
  let inv;
  try {
    inv = await publicInvite(code);
  } catch (e) {
    shell(h('h1', {}, 'PT Coach'), h('p', { class: 'banner' }, e.message), back);
    return true;
  }
  if (!inv) {
    shell(
      h('h1', {}, 'This link doesn’t work'),
      h('p', {}, 'The invitation is invalid, has already been used, or has expired.'),
      back);
    return true;
  }
  if (!me.profile && inv.email.toLowerCase() === (me.email || '').toLowerCase()) return false;
  const who = inv.kind === 'coach' ? 'a coach account' : `an account to train with ${inv.coachName}`;
  shell(
    h('h1', {}, 'You’re signed in'),
    h('p', {}, `This invitation is for ${inv.email} to create ${who}, but this device is signed in as ${me.email}.`),
    h('p', { class: 'small muted' }, `If you’re ${inv.email}, sign out first. Otherwise, forward the email to them.`),
    h('button', {
      class: 'primary block',
      onclick: () => {
        try { sessionStorage.setItem(RESUME, `#/join/${code}`); } catch { /* reopen the link after signing out */ }
        logout();
      },
    }, 'Sign out and create the account'),
    back);
  return true;
}

const shell = (...kids) => mount($app(), h('div', { class: 'auth-page' }, h('img', { src: 'icon.svg', alt: '', width: 64, height: 64 }), ...kids));

export async function renderInviteSignUp(code) {
  let inv;
  try {
    inv = await publicInvite(code);
  } catch (e) {
    return shell(h('h1', {}, 'PT Coach'), h('p', { class: 'banner' }, e.message));
  }
  if (!inv) {
    return shell(
      h('h1', {}, 'This link doesn’t work'),
      h('p', {}, 'The invitation is invalid, has already been used, or has expired. Ask whoever invited you for a new one.'),
      h('button', { class: 'block', onclick: () => login() }, 'I already have an account: sign in'),
    );
  }
  const asCoach = inv.kind === 'coach';
  const intro = asCoach ? 'You’ve been invited to set up a coach account.' : `${inv.coachName} invited you to train with them.`;
  const email = h('input', { type: 'email', value: inv.email, readonly: true, 'aria-readonly': 'true' });

  if (devMode()) {
    // Local development has no Cognito: dev users are {id}@dev.local.
    const user = inv.email.split('@')[0];
    return shell(
      h('h1', {}, asCoach ? 'Coach on PT Coach' : `Join ${inv.coachName}`),
      h('p', {}, intro),
      h('div', { class: 'card' }, field('Email', email),
        h('button', { class: 'primary block', onclick: () => { localStorage.setItem('pt.devUser', user); location.reload(); } }, `Continue as ${user} (dev)`)),
    );
  }

  const pw = passwordField('Password', { autocomplete: 'new-password' });
  const pw2 = passwordField('Confirm password', { autocomplete: 'new-password' });
  const problem = h('p', { class: 'small', 'aria-live': 'polite' });
  shell(
    h('h1', {}, asCoach ? 'Create your coach account' : `Join ${inv.coachName}`),
    h('p', {}, intro),
    h('div', { class: 'card' },
      field('Email', email, 'This is the address your invitation was sent to.'),
      pw.el, pw2.el,
      h('p', { class: 'small muted' }, `At least ${MIN_LENGTH} characters, with a lowercase letter and a number.`),
      problem,
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          const bad = policyProblem(pw.input.value) || (pw.input.value !== pw2.input.value ? 'The passwords don’t match.' : null);
          problem.textContent = bad || '';
          if (bad) return;
          try {
            await signUp(inv.email, pw.input.value);
          } catch (err) {
            if (err.code === 'UsernameExistsException') return renderExisting(inv.email);
            problem.textContent = err.message;
            return;
          }
          renderCode(inv.email);
        }),
      }, 'Create account')),
    h('p', { class: 'center small' }, 'Already have an account? ',
      h('button', { class: 'ghost small', onclick: () => login({ loginHint: inv.email }) }, 'Sign in')),
  );
}

function renderExisting(email) {
  shell(
    h('h1', {}, 'You already have an account'),
    h('p', {}, `There’s already an account for ${email}. Sign in to accept the invitation.`),
    h('button', { class: 'primary block', onclick: () => login({ loginHint: email }) }, 'Sign in'),
  );
}

function renderCode(email) {
  const code = h('input', { inputmode: 'numeric', autocomplete: 'one-time-code', maxlength: 8 });
  const msg = h('p', { class: 'small', 'aria-live': 'polite' });
  shell(
    h('h1', {}, 'Check your email'),
    h('p', {}, `We sent a verification code to ${email}. It can take a minute; check spam if you don’t see it.`),
    h('div', { class: 'card' },
      field('Verification code', code),
      msg,
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          try {
            await confirmSignUp(email, code.value.trim());
          } catch (err) {
            msg.textContent = err.code === 'CodeMismatchException' ? 'That code isn’t right. Check it and try again.'
              : err.code === 'ExpiredCodeException' ? 'That code has expired. Send a new one.' : err.message;
            return;
          }
          // Account verified: sign in (the email is filled in) to finish joining.
          await login({ loginHint: email });
        }),
      }, 'Verify and sign in'),
      h('button', {
        class: 'ghost block',
        onclick: (e) => busy(e.currentTarget, async () => {
          await resendSignUpCode(email);
          msg.textContent = 'A new code is on its way.';
        }),
      }, 'Send a new code')),
  );
}
