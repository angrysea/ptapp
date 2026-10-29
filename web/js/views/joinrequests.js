// The coach's side of client requests (routes/requests.rs), on Clients:
// their own sign-up link (and a button to paste into their website), the
// auto-approve switch, and the queue of requests with each one's status.
// Inviting a request sends the same invitation as "+ Invite".

import { get, post, put } from '../api.js';
import { session } from '../session.js';
import { busy, fmtDate, h, sheet, toast } from '../ui.js';

const STATUS = { pending: ['Waiting', 'warn'], invited: ['Invited', ''], denied: ['Denied', 'gray'] };

/** The public page a would-be client opens (views/request.js). */
export const signupLink = () => `${location.origin}${location.pathname}#/request/${encodeURIComponent(session.userId)}`;

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

/** HTML for the coach's own website: a button linking to their sign-up page. */
function buttonHtml(name) {
  return `<a href="${esc(signupLink())}" target="_blank" rel="noopener" style="display:inline-block;padding:12px 22px;background:#0f766e;color:#ffffff;border-radius:8px;font:600 16px/1.2 system-ui,sans-serif;text-decoration:none">Train with ${esc(name)}</a>`;
}

async function copy(text, what) {
  try { await navigator.clipboard.writeText(text); toast(`${what} copied`); } catch { toast('Couldn’t copy: select the text and copy it yourself', 'error'); }
}

function websiteSheet() {
  const name = session.profile.businessName || session.profile.name;
  const html = buttonHtml(name);
  const sh = sheet(
    h('h2', {}, 'Your sign-up link'),
    h('p', { class: 'small' }, 'Put this on your website or social profile. People who use it ask you for an invitation, and their request shows up here.'),
    h('input', { value: signupLink(), readonly: true, 'aria-label': 'Sign-up link', onfocus: (e) => e.target.select() }),
    h('div', { class: 'row', style: 'margin-top:8px' },
      h('button', { class: 'grow', onclick: () => copy(signupLink(), 'Link') }, 'Copy link'),
      h('a', { class: 'btn grow', href: signupLink(), target: '_blank', rel: 'noopener' }, 'Open the page')),
    h('h3', {}, 'A button for your website'),
    h('p', { class: 'small muted' }, 'Paste this HTML where the button should appear (most site builders have an “HTML” or “embed code” block):'),
    h('div', { style: 'margin:8px 0' }, h('a', { href: signupLink(), target: '_blank', rel: 'noopener', class: 'btn primary' }, `Train with ${name}`)),
    h('textarea', { readonly: true, rows: 4, 'aria-label': 'Button HTML', style: 'font-family:monospace;font-size:0.8rem', onfocus: (e) => e.target.select() }, html),
    h('button', { class: 'block', style: 'margin-top:8px', onclick: () => copy(html, 'Button HTML') }, 'Copy button HTML'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Done'),
  );
}

/**
 * The requests card for Clients. `onChange` redraws the page (an invite adds
 * a pending invitation); `emailResult` reports whether the email went out.
 */
export async function requestsCard({ onChange, emailResult }) {
  const [requests, settings] = await Promise.all([
    get('/requests').catch(() => []),
    get('/requests/settings').catch(() => ({ autoApprove: false, humanCheck: false })),
  ]);
  const pending = requests.filter((r) => r.status === 'pending');
  // Waiting first, then the rest, newest first within each.
  const shown = [...pending, ...requests.filter((r) => r.status !== 'pending')].slice(0, 30);

  const autoControls = settings.humanCheck || settings.autoApprove
    ? h('div', { class: 'row small' },
      h('span', { class: `chip ${settings.autoApprove ? 'warn' : 'gray'}` }, settings.autoApprove ? 'Auto-approve on' : 'You approve each one'),
      h('button', {
        class: 'small',
        onclick: (e) => busy(e.currentTarget, async () => {
          const s = await put('/requests/settings', { autoApprove: !settings.autoApprove });
          toast(s.autoApprove ? 'New requests get an invitation email right away' : 'New requests wait here for you');
          onChange();
        }),
      }, settings.autoApprove ? 'Approve by hand instead' : 'Auto-approve requests'))
    : null;

  return h('div', { class: 'card' },
    h('div', { class: 'spread' },
      h('h3', {}, 'Join requests', pending.length > 0 && h('span', { class: 'chip warn', style: 'margin-left:6px' }, `${pending.length} waiting`)),
      h('button', { class: 'small', onclick: websiteSheet }, '🔗 Your sign-up link')),
    autoControls,
    settings.maxPending && pending.length >= settings.maxPending && h('p', { class: 'banner' },
      `${pending.length} requests are waiting, the most the queue holds: new ones are dropped until you invite or deny some.`),
    shown.length === 0
      ? h('p', { class: 'small muted' }, 'No requests yet. Share your sign-up link, or put its button on your website, and requests show up here.')
      : shown.map((r) => {
        const [label, chip] = STATUS[r.status] || [r.status, 'gray'];
        return h('div', { class: 'spread request-row' },
          h('div', {},
            h('strong', {}, r.name || r.email),
            h('div', { class: 'muted small' }, `${r.name ? `${r.email} · ` : ''}asked ${fmtDate(r.requestedAt.slice(0, 10))}`)),
          h('div', { class: 'row' },
            h('span', { class: `chip ${chip}` }, label),
            r.status !== 'invited' && h('button', {
              class: 'small primary',
              onclick: (e) => busy(e.currentTarget, async () => {
                const out = await post(`/requests/${encodeURIComponent(r.email)}/invite`);
                emailResult(out.invite);
                onChange();
              }),
            }, 'Invite'),
            r.status === 'pending' && h('button', {
              class: 'small',
              onclick: (e) => busy(e.currentTarget, async () => {
                await post(`/requests/${encodeURIComponent(r.email)}/deny`);
                toast(`${r.email} denied. They can ask again in 30 days.`);
                onChange();
              }),
            }, 'Deny')));
      }));
}
