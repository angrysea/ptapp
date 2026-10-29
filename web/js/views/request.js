// A coach's own sign-up page (#/request/{coachId}): the link a coach puts on
// their website. A would-be client asks for an invitation; it lands in the
// coach's queue (routes/requests.rs), and approving it emails the same
// invitation the coach sends from Clients. Works signed out.

import cfg from '../config.js';
import { requestForm } from '../publicform.js';
import { $app, h, mount } from '../ui.js';

export async function renderRequest(coachId) {
  let coach = null;
  try {
    const res = await fetch(`${cfg.apiUrl}/public/join/${encodeURIComponent(coachId)}`);
    if (res.ok) coach = await res.json();
  } catch { /* offline: shown as not found below */ }
  if (!coach) {
    return mount($app(),
      h('h1', {}, 'Coach not found'),
      h('p', { class: 'muted' }, 'This sign-up link doesn’t lead to a coach taking requests. Check the link with your coach.'));
  }
  const name = coach.coachName;
  mount($app(),
    h('div', { class: 'center', style: 'padding-top:5vh' },
      h('img', { src: 'icon.svg', alt: '', width: 64, height: 64 }),
      h('h1', {}, `Train with ${name}`),
      h('p', { class: 'muted' }, `Ask ${name} for an invitation. Once they approve it, you get an email with a link to set up your account.`)),
    h('div', { class: 'card' },
      requestForm(`/join/${encodeURIComponent(coachId)}`, {
        nameLabel: 'Your name', namePlaceholder: 'First and last name',
        thanks: `Thanks. If ${name} approves you, an invitation arrives at that email.`,
      })),
    h('p', { class: 'small muted' }, 'Your workouts, progress and messages with your coach, in one app. Already invited? Use the link in your invitation email.'));
}
