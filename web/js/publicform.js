// The anonymous "request an invitation" forms (routes/requests.rs): the
// landing page's, for coaches asking the club, and a coach's own sign-up page
// (views/request.js), for would-be clients. Both send an email and a name,
// with the hidden field, the page's timing and, when the deploy set it up,
// Cloudflare Turnstile's token; the API answers every well-formed request the
// same way, so neither form says who was invited.

import cfg from './config.js';
import { busy, field, h, toast } from './ui.js';

// Public call, like the directory's plain fetch: these pages work signed
// out, so there is no token to attach. Returns the body, `{ problem }` with
// the server's message for a refused request, or null when unreachable.
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

/** Cloudflare Turnstile's "are you a person" widget, when the deploy set a
 * site key (scripts/deploy.sh); the API checks its token. Without one there
 * is nothing to show and the token is empty. */
function humanCheck() {
  const sitekey = cfg.turnstileSiteKey;
  if (!sitekey) return { el: null, token: () => '', reset() {} };
  const el = h('div', { class: 'turnstile' });
  let token = '';
  let id = null;
  const render = () => {
    if (id != null || !el.isConnected) return;
    id = window.turnstile.render(el, {
      sitekey,
      callback: (t) => { token = t; },
      'expired-callback': () => { token = ''; },
      'error-callback': () => { token = ''; },
    });
  };
  // Rendered once the form is on the page and the script has loaded.
  const ready = () => requestAnimationFrame(render);
  if (window.turnstile) ready();
  else {
    let s = document.getElementById('turnstile-js');
    if (!s) {
      s = h('script', { id: 'turnstile-js', src: 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit', async: true });
      document.head.append(s);
    }
    s.addEventListener('load', ready);
  }
  return { el, token: () => token, reset: () => { token = ''; if (id != null) window.turnstile.reset(id); } };
}

/**
 * An "ask to join" form posting to `/public{path}`: one record per email
 * lands in that queue. `opts`: nameLabel, namePlaceholder, emailPlaceholder,
 * thanks (the toast after sending).
 */
export function requestForm(path, opts) {
  const shownAt = Date.now(); // the server drops sends faster than a person types
  const email = h('input', { type: 'email', autocomplete: 'email', placeholder: opts.emailPlaceholder || 'you@example.com' });
  const name = h('input', { autocomplete: 'name', placeholder: opts.namePlaceholder || '' });
  // Hidden from people (and screen readers); form-filling scripts fill it.
  const website = h('input', { name: 'website', tabindex: '-1', autocomplete: 'off' });
  const problem = h('p', { class: 'small', 'aria-live': 'polite' });
  const check = humanCheck();
  return [
    field('Email', email),
    field(opts.nameLabel, name),
    h('div', { class: 'hp', 'aria-hidden': 'true' }, h('label', {}, 'Leave this empty', website)),
    check.el,
    problem,
    h('button', {
      class: 'grow',
      onclick: (e) => busy(e.currentTarget, async () => {
        const value = email.value.trim();
        if (!value.includes('@')) { problem.textContent = 'Type the email you want to be invited with.'; return; }
        if (check.el && !check.token()) { problem.textContent = 'One moment: the check above confirms you’re a person.'; return; }
        problem.textContent = '';
        const r = await pubPost(path, { email: value, name: name.value, website: website.value, elapsedMs: Date.now() - shownAt, turnstile: check.token() });
        check.reset(); // each token works once
        if (!r) { problem.textContent = 'Couldn’t send that right now; try again in a bit.'; return; }
        if (r.problem) { problem.textContent = r.problem.charAt(0).toUpperCase() + r.problem.slice(1); return; }
        // The same answer for everyone: the form never says who was invited.
        toast(opts.thanks);
      }),
    }, 'Request an invitation'),
  ];
}
