// The invitation sign-up page (ON-1). An invite link (#/join/CODE) opened
// while signed out lands here: the invitation is read from the API, its email
// is filled in and locked, and the account is created with Cognito directly.
// Then Cognito's sign-in page opens with that email filled in; once signed
// in, the onboarding screen accepts the invitation, and the link stops
// working. Coaches only ever arrive this way (no open coach signup).

import cfg from '../config.js';
import { confirmSignUp, devMode, login, resendSignUpCode, signUp } from '../auth.js';
import { $app, busy, field, h, mount, passwordField } from '../ui.js';

// Mirrors the user pool's password policy in template.yaml.
const MIN_LENGTH = 10;
const policyProblem = (pw) =>
  pw.length < MIN_LENGTH ? `Use at least ${MIN_LENGTH} characters.`
    : !/[a-z]/.test(pw) ? 'Include a lowercase letter.'
      : !/[0-9]/.test(pw) ? 'Include a number.'
        : null;

/** The open invitation behind `code`, read without signing in; null if it's invalid, used or expired. */
async function publicInvite(code) {
  const res = await fetch(`${cfg.apiUrl}/invites/${encodeURIComponent(code)}/public`);
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`Couldn’t load the invitation (${res.status}). Check your connection and try again.`);
  return res.json();
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
