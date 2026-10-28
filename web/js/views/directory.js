// The public coach directory (tasks/coaches): a list of coaches and each
// coach's page, open to anyone, signed in or not. People contact a coach by
// email; the coach then sends them an invitation. Reads only /public/*.

import cfg from '../config.js';
import { session } from '../session.js';
import { $app, h, mount } from '../ui.js';

const FORMATS = { inPerson: 'In person', online: 'Online' };

async function pub(path) {
  const res = await fetch(`${cfg.apiUrl}/public${path}`);
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`Couldn’t load coaches (${res.status}). Check your connection and try again.`);
  return res.json();
}

const where = (c) => [c.city, c.state].filter(Boolean).join(', ');
const formats = (c) => c.formats.map((f) => FORMATS[f] || f).join(' · ');
const photo = (c, cls) => h('img', { class: cls, alt: '', src: c.photoUrl || 'icon.svg', loading: 'lazy' });

/** Signed out, the page has no tab bar; give it a way back. */
const topLink = () => (session.profile
  ? null
  : h('p', {}, h('a', { href: '#/' }, '← PT Coach home')));

export async function renderDirectory() {
  let coaches;
  try {
    coaches = await pub('/coaches');
  } catch (e) {
    return mount($app(), topLink(), h('h1', {}, 'Find a coach'), h('p', { class: 'banner' }, e.message));
  }
  const q = h('input', { type: 'search', placeholder: 'Search by name, specialty or city', 'aria-label': 'Search coaches' });
  const list = h('div');
  const draw = () => {
    const words = q.value.toLowerCase().split(/\s+/).filter(Boolean);
    const shown = coaches.filter((c) => {
      const text = [c.displayName, c.headline, where(c), ...c.specialties, formats(c)].join(' ').toLowerCase();
      return words.every((w) => text.includes(w));
    });
    mount(list,
      shown.length === 0 && h('div', { class: 'card muted' }, coaches.length ? 'No coaches match that search.' : 'No coaches are listed yet.'),
      shown.map((c) => h('a', { class: 'card coach-card', href: `#/coaches/${encodeURIComponent(c.coachId)}` },
        photo(c, 'coach-photo'),
        h('div', { class: 'grow' },
          h('strong', {}, c.displayName),
          h('div', {}, c.headline),
          h('div', { class: 'muted small' }, [where(c), formats(c)].filter(Boolean).join(' · ')),
          h('div', { class: 'chips' }, c.specialties.slice(0, 4).map((s) => h('span', { class: 'chip' }, s)),
            !c.acceptingClients && h('span', { class: 'chip gray' }, 'Not taking new clients'))))));
  };
  q.addEventListener('input', draw);
  draw();
  mount($app(),
    topLink(),
    h('h1', {}, 'Find a coach'),
    h('p', { class: 'muted' }, 'Browse our coaches and email the one that suits you. They’ll send you an invitation to start training.'),
    q,
    list);
}

export async function renderCoachPage(id) {
  let c;
  try {
    c = await pub(`/coaches/${encodeURIComponent(id)}`);
  } catch (e) {
    return mount($app(), topLink(), h('p', { class: 'banner' }, e.message));
  }
  const back = h('p', {}, h('a', { href: '#/coaches' }, '← All coaches'));
  if (!c) return mount($app(), back, h('h1', {}, 'Coach not found'), h('p', {}, 'This coach isn’t listed in the directory.'));

  // The address stays out of the page until asked for, away from simple scrapers.
  const subject = encodeURIComponent('Training enquiry from PT Coach');
  const contact = h('div');
  const reveal = () => mount(contact,
    h('a', { class: 'btn primary block', href: `mailto:${c.contactEmail}?subject=${subject}` }, `Email ${c.displayName}`),
    h('p', { class: 'center small' }, c.contactEmail));
  mount(contact, h('button', { class: 'primary block', onclick: reveal }, 'Show email'));

  mount($app(),
    back,
    h('div', { class: 'coach-head' },
      photo(c, 'coach-photo large'),
      h('div', {},
        h('h1', {}, c.displayName),
        h('p', {}, c.headline),
        h('p', { class: 'muted small' }, [where(c), formats(c), c.yearsExperience ? `${c.yearsExperience} years coaching` : null].filter(Boolean).join(' · ')),
        h('span', { class: `chip ${c.acceptingClients ? 'good' : 'gray'}` }, c.acceptingClients ? 'Accepting new clients' : 'Not taking new clients'))),
    h('div', { class: 'card' }, contact),
    h('div', { class: 'card' }, h('h2', {}, 'About'), h('p', { style: 'white-space:pre-wrap' }, c.bio)),
    h('div', { class: 'card' },
      h('h2', {}, 'Specialties'),
      h('div', { class: 'chips' }, c.specialties.map((s) => h('span', { class: 'chip' }, s)))),
    h('div', { class: 'card' },
      h('h2', {}, 'Certifications'),
      h('ul', {}, c.certifications.map((x) => h('li', {}, `${x.name} (${x.issuer})`)))),
    h('div', { class: 'card' },
      c.languages.length > 0 && h('p', {}, h('strong', {}, 'Languages: '), c.languages.join(', ')),
      c.rates && h('p', {}, h('strong', {}, 'Rates: '), c.rates),
      c.website && h('p', {}, h('a', { href: c.website, rel: 'noopener nofollow', target: '_blank' }, 'Website')),
      c.instagram && h('p', {}, h('a', { href: `https://instagram.com/${encodeURIComponent(c.instagram)}`, rel: 'noopener nofollow', target: '_blank' }, `Instagram @${c.instagram}`))),
  );
}
