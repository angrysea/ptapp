// The coach's profile (tasks/coaches): private business details for the
// admin, and the public profile people see in the coach directory. Required
// before a coach uses the app (app.js shows it until it's complete). The
// server checks the same rules (ptcore::coach::CoachProfile::problems).

import { get, post, put } from '../api.js';
import { session } from '../session.js';
import { $app, busy, field, h, mount, shrinkImage, toast } from '../ui.js';

// Mirrors ptcore::coach::SPECIALTIES.
const SPECIALTIES = [
  'Strength training', 'Weight loss', 'Muscle gain', 'General fitness', 'Mobility and flexibility', 'Sports performance',
  'Injury recovery', 'Pre- and postnatal', 'Older adults', 'Youth athletes', 'Powerlifting', 'Olympic weightlifting',
  'Endurance', 'Nutrition coaching',
];
const CERTS = [
  'NASM Certified Personal Trainer (NASM-CPT)', 'ACE Certified Personal Trainer', 'ACSM Certified Personal Trainer',
  'NSCA Certified Strength and Conditioning Specialist (CSCS)', 'NSCA Certified Personal Trainer (NSCA-CPT)',
  'ISSA Certified Personal Trainer', 'NCSF Certified Personal Trainer', 'NESTA Certified Personal Fitness Trainer',
  'CrossFit Level 1 Trainer', 'USA Weightlifting Level 1 Coach', 'Precision Nutrition Level 1',
];
const MIN_BIO = 80;

const reloadHome = (hash = '#/') => { location.hash = hash; location.reload(); };

/** `onboarding`: the first-time setup, which must be finished before the app opens. */
export async function renderCoachSetup({ onboarding = false } = {}) {
  const { profile, photoUrl } = await get('/me/coach-profile');
  const p = profile || {
    legalName: '', phone: '', address: { line1: '', line2: '', city: '', state: '', postalCode: '', country: 'US' },
    cprExpires: null, insurance: null, displayName: session.profile.name || '', headline: '', bio: '', photoKey: null,
    specialties: [], certifications: [], yearsExperience: 0, formats: [], languages: ['English'], rates: '',
    contactEmail: session.profile.email || '', website: '', instagram: '', acceptingClients: true, listed: true,
  };

  const input = (value, attrs = {}) => h('input', { value: value ?? '', ...attrs });
  const f = {
    legalName: input(p.legalName, { autocomplete: 'name', required: true }),
    phone: input(p.phone, { type: 'tel', autocomplete: 'tel', required: true }),
    line1: input(p.address.line1, { autocomplete: 'address-line1', required: true }),
    line2: input(p.address.line2, { autocomplete: 'address-line2' }),
    city: input(p.address.city, { autocomplete: 'address-level2', required: true }),
    state: input(p.address.state, { autocomplete: 'address-level1', required: true }),
    postalCode: input(p.address.postalCode, { autocomplete: 'postal-code', required: true }),
    country: input(p.address.country, { autocomplete: 'country', required: true }),
    cprExpires: input(p.cprExpires, { type: 'date', required: true }),
    insProvider: input(p.insurance?.provider),
    insPolicy: input(p.insurance?.policyNumber),
    insExpires: input(p.insurance?.expires, { type: 'date' }),
    displayName: input(p.displayName, { required: true }),
    headline: input(p.headline, { maxlength: 120, placeholder: 'e.g. Strength training for busy parents', required: true }),
    bio: h('textarea', { value: p.bio, rows: 6, placeholder: 'Your approach, who you help, and what clients can expect.', required: true }),
    years: input(p.yearsExperience, { type: 'number', min: 0, max: 70, inputmode: 'numeric' }),
    languages: input(p.languages.join(', '), { placeholder: 'English, Spanish' }),
    rates: input(p.rates, { placeholder: 'e.g. From $60/session' }),
    contactEmail: input(p.contactEmail, { type: 'email', required: true }),
    website: input(p.website, { type: 'url', placeholder: 'https://' }),
    instagram: input(p.instagram, { placeholder: '@yourhandle' }),
    accepting: h('input', { type: 'checkbox', checked: p.acceptingClients }),
    listed: h('input', { type: 'checkbox', checked: p.listed }),
  };
  const bioCount = h('span', { class: 'muted small' });
  const countBio = () => { const n = f.bio.value.trim().length; bioCount.textContent = n < MIN_BIO ? `${n} / ${MIN_BIO} characters minimum` : `${n} characters`; };
  f.bio.addEventListener('input', countBio);
  countBio();

  const checks = (options, chosen) => options.map(([value, label]) => {
    const box = h('input', { type: 'checkbox', value, checked: chosen.includes(value) });
    return { box, el: h('label', { class: 'check' }, box, label) };
  });
  const known = SPECIALTIES.map((s) => [s, s]);
  const specialties = checks(known, p.specialties);
  const otherSpecialty = input(p.specialties.filter((s) => !SPECIALTIES.includes(s)).join(', '), { placeholder: 'Other (comma-separated)' });
  const formats = checks([['inPerson', 'In person'], ['online', 'Online']], p.formats);

  // ---- certifications: a row each
  const certList = h('div');
  const certRows = [];
  const certOptions = h('datalist', { id: 'cert-names' }, CERTS.map((c) => h('option', { value: c })));
  function addCert(c = { name: '', issuer: '', number: '', expires: null }) {
    const row = {
      name: input(c.name, { list: 'cert-names', placeholder: 'Certification' }),
      issuer: input(c.issuer, { placeholder: 'Issued by, e.g. NASM' }),
      number: input(c.number, { placeholder: 'Certificate number (private)' }),
      expires: input(c.expires, { type: 'date' }),
    };
    row.el = h('div', { class: 'card cert' },
      field('Certification', row.name), field('Issuer', row.issuer),
      h('div', { class: 'row' }, h('div', { class: 'grow' }, field('Number', row.number)), h('div', { class: 'grow' }, field('Expires', row.expires))),
      h('button', { type: 'button', class: 'small ghost', onclick: () => { certRows.splice(certRows.indexOf(row), 1); row.el.remove(); } }, 'Remove'));
    certRows.push(row);
    certList.append(row.el);
  }
  (p.certifications.length ? p.certifications : [undefined]).forEach((c) => addCert(c));

  // ---- photo: uploaded as soon as it's picked; saved with the profile
  let photoKey = p.photoKey;
  const photo = h('img', { class: 'coach-photo', alt: '', src: photoUrl || 'icon.svg' });
  const photoPick = h('input', { type: 'file', accept: 'image/*', hidden: true });
  const photoBtn = h('button', { type: 'button', class: 'small', onclick: () => photoPick.click() }, photoKey ? 'Change photo' : 'Add a photo');
  photoPick.addEventListener('change', () => busy(photoBtn, async () => {
    const file = photoPick.files[0];
    if (!file) return;
    const blob = await shrinkImage(file, 1000);
    const up = await post('/me/coach-profile/photo', { contentType: 'image/jpeg', size: blob.size });
    const res = await fetch(up.uploadUrl, { method: 'PUT', headers: up.uploadHeaders, body: blob });
    if (!res.ok) throw new Error(`Photo upload failed (${res.status})`);
    photoKey = up.photoKey;
    photo.src = URL.createObjectURL(blob);
    photoBtn.textContent = 'Change photo';
  }));

  const problems = h('div');
  const collect = () => ({
    legalName: f.legalName.value,
    phone: f.phone.value,
    address: { line1: f.line1.value, line2: f.line2.value, city: f.city.value, state: f.state.value, postalCode: f.postalCode.value, country: f.country.value },
    cprExpires: f.cprExpires.value || null,
    insurance: f.insProvider.value.trim() || f.insPolicy.value.trim() || f.insExpires.value
      ? { provider: f.insProvider.value, policyNumber: f.insPolicy.value, expires: f.insExpires.value || null } : null,
    displayName: f.displayName.value,
    headline: f.headline.value,
    bio: f.bio.value,
    photoKey,
    specialties: [...specialties.filter((s) => s.box.checked).map((s) => s.box.value), ...otherSpecialty.value.split(',')],
    certifications: certRows.map((r) => ({ name: r.name.value, issuer: r.issuer.value, number: r.number.value, expires: r.expires.value || null })),
    yearsExperience: Math.max(0, Math.round(Number(f.years.value) || 0)),
    formats: formats.filter((x) => x.box.checked).map((x) => x.box.value),
    languages: f.languages.value.split(','),
    rates: f.rates.value,
    contactEmail: f.contactEmail.value,
    website: f.website.value,
    instagram: f.instagram.value,
    acceptingClients: f.accepting.checked,
    listed: f.listed.checked,
  });

  const save = h('button', {
    class: 'primary block',
    onclick: (e) => busy(e.currentTarget, async () => {
      mount(problems);
      try {
        const res = await put('/me/coach-profile', collect());
        session.profile.name = res.profile.displayName;
        if (onboarding) return reloadHome('#/');
        toast(res.listed ? 'Saved. Your profile is in the coach directory.' : 'Saved');
      } catch (err) {
        if (err.status !== 400) throw err;
        mount(problems, h('div', { class: 'banner' }, h('strong', {}, 'Please fix:'),
          h('ul', {}, err.message.split('; ').map((m) => h('li', {}, m)))));
        problems.scrollIntoView({ block: 'center', behavior: 'smooth' });
      }
    }),
  }, onboarding ? 'Save and continue' : 'Save');

  mount($app(),
    h('h1', {}, onboarding ? 'Set up your coach profile' : 'Coach profile'),
    onboarding && h('p', { class: 'muted' }, 'Tell clients about yourself. Your business details stay private; the public profile appears in the coach directory, where people looking for a coach can find you and email you.'),
    !onboarding && profile && h('p', {}, h('a', { href: `#/coaches/${session.userId}` }, 'View my public page')),
    certOptions,

    h('h2', {}, 'Business details'),
    h('p', { class: 'muted small' }, 'Private: only you and the PT Coach admins see these.'),
    h('div', { class: 'card' },
      field('Legal name', f.legalName),
      field('Phone', f.phone),
      field('Street address', f.line1),
      field('Apartment, suite (optional)', f.line2),
      h('div', { class: 'row' }, h('div', { class: 'grow' }, field('City', f.city)), h('div', { class: 'grow' }, field('State', f.state))),
      h('div', { class: 'row' }, h('div', { class: 'grow' }, field('ZIP code', f.postalCode)), h('div', { class: 'grow' }, field('Country', f.country))),
      field('CPR/AED certification expires', f.cprExpires)),
    h('div', { class: 'card' },
      h('h3', {}, 'Liability insurance (optional)'),
      field('Provider', f.insProvider),
      h('div', { class: 'row' }, h('div', { class: 'grow' }, field('Policy number', f.insPolicy)), h('div', { class: 'grow' }, field('Expires', f.insExpires)))),

    h('h2', {}, 'Public profile'),
    h('p', { class: 'muted small' }, 'Shown in the coach directory. Your city and state appear, not your street address.'),
    h('div', { class: 'card' },
      h('div', { class: 'row' }, photo, h('div', {}, photoBtn, h('div', { class: 'muted small' }, 'A clear, friendly photo of you.'))),
      photoPick,
      field('Display name', f.displayName, 'The name clients see.'),
      field('Headline', f.headline),
      field('About you', f.bio), bioCount,
      field('Years of experience', f.years),
      h('div', { class: 'field' }, h('label', {}, 'Specialties'), h('div', { class: 'checks' }, specialties.map((s) => s.el)), otherSpecialty),
      h('div', { class: 'field' }, h('label', {}, 'How you train'), h('div', { class: 'checks' }, formats.map((x) => x.el))),
      field('Languages', f.languages),
      field('Rates (optional)', f.rates),
      field('Contact email', f.contactEmail, 'Shown on your public page so people can reach you.'),
      field('Website (optional)', f.website),
      field('Instagram (optional)', f.instagram)),

    h('h2', {}, 'Certifications'),
    h('p', { class: 'muted small' }, 'Names and issuers are public; numbers stay private.'),
    certList,
    h('button', { type: 'button', class: 'block', onclick: () => addCert() }, '+ Add certification'),

    h('h2', {}, 'Directory'),
    h('div', { class: 'card' },
      h('label', { class: 'check' }, f.listed, 'Show me in the coach directory'),
      h('label', { class: 'check' }, f.accepting, 'I’m accepting new clients')),

    problems,
    save,
  );
}
