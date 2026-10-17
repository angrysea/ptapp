// ON-2 intake questionnaire and ON-3 health screening + liability waiver.
// Clients edit; coaches see a read-only summary.

import { get, put, post } from '../api.js';
import { go, query } from '../router.js';
import { session } from '../session.js';
import { $app, busy, field, fmtDateTime, h, mount, num, round, select, toast, units } from '../ui.js';

export const WAIVER_VERSION = '2026-10';

// Health screening questions modelled on the PAR-Q+ general section.
// Have the final wording and waiver text reviewed before launch (see docs).
export const PARQ = [
  'Has a doctor ever said you have a heart condition or high blood pressure?',
  'Do you feel pain in your chest at rest, during daily activities, or when you are physically active?',
  'In the past 12 months, have you lost balance because of dizziness or lost consciousness?',
  'Have you been diagnosed with another chronic medical condition?',
  'Do you currently take prescribed medication for a chronic medical condition?',
  'In the past 12 months, have you had a bone, joint or soft-tissue problem that could get worse with more activity?',
  'Has a doctor ever said you should only do medically supervised physical activity?',
];

const WAIVER_TEXT = `I understand that physical exercise carries an inherent risk of injury. I confirm that the health information I have provided is accurate and that I will tell my coach about any change in my health. I will stop exercising and seek medical advice if I feel pain, dizziness or shortness of breath. I voluntarily assume the risks of participating and release my coach from liability for injuries arising from my participation, except where caused by their negligence.`;

const EQUIPMENT = ['barbell', 'dumbbells', 'kettlebells', 'machines', 'cables', 'bands', 'pull-up bar', 'bench', 'cardio machines', 'bodyweight only'];

export async function renderIntake() {
  const me = session.profile;
  const onboarding = query().get('onboarding') === '1';
  const intake = (await get(`/clients/${me.userId}/intake`, { cache: true })) || {};
  const u = units(me);
  const f = {
    goals: h('textarea', { value: intake.goals || '', placeholder: 'What do you want to achieve, and by when?' }),
    history: h('textarea', { value: intake.trainingHistory || '', placeholder: 'How long have you trained? What kind?' }),
    schedule: h('input', { value: intake.schedule || '', placeholder: 'e.g. Mon/Wed/Fri mornings, 60 min' }),
    injuries: h('textarea', { value: intake.injuries || '', placeholder: 'Past or current injuries, surgeries, limitations' }),
    sex: select([['', '—'], ['female', 'Female'], ['male', 'Male']], intake.sex || ''),
    birth: h('input', { type: 'date', value: intake.birthDate || '' }),
    height: h('input', { type: 'number', inputmode: 'decimal', value: intake.heightCm ? u.toL(intake.heightCm) : '' }),
  };
  const equip = EQUIPMENT.map((e) => h('input', { type: 'checkbox', value: e, checked: (intake.equipment || []).includes(e) }));

  mount($app(),
    h('h1', {}, onboarding ? 'Step 1 of 2: About you' : 'Intake questionnaire'),
    h('div', { class: 'card' },
      field('Goals', f.goals),
      field('Training history', f.history),
      h('div', { class: 'field' }, h('label', {}, 'Equipment you can use'),
        h('div', { class: 'grid2' }, equip.map((cb) => h('label', { class: 'check' }, cb, cb.value)))),
      field('When can you train?', f.schedule),
      field('Injuries or limitations', f.injuries),
      h('div', { class: 'grid2' },
        field('Sex (for body-fat equations)', f.sex),
        field('Birth date', f.birth),
        field(`Height (${u.l})`, f.height)),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          await put(`/clients/${me.userId}/intake`, {
            goals: f.goals.value, trainingHistory: f.history.value, schedule: f.schedule.value, injuries: f.injuries.value,
            equipment: equip.filter((c) => c.checked).map((c) => c.value),
            sex: f.sex.value || null, birthDate: f.birth.value || null, heightCm: u.fromL(f.height.value),
          });
          session.profile.sex = f.sex.value || null;
          session.profile.birthDate = f.birth.value || null;
          toast('Saved');
          go(onboarding ? '/waiver?onboarding=1' : '/me');
        }),
      }, onboarding ? 'Next' : 'Save')),
  );
}

export async function renderWaiver() {
  const me = session.profile;
  const onboarding = query().get('onboarding') === '1';
  const existing = await get(`/clients/${me.userId}/waiver`, { cache: true });
  if (existing && !onboarding && existing.waiverVersion === WAIVER_VERSION) {
    return mount($app(), h('h1', {}, 'Health screening & waiver'), waiverSummary(existing));
  }
  const answers = PARQ.map(() => ({ yes: h('input', { type: 'radio' }), no: h('input', { type: 'radio' }) }));
  answers.forEach((a, i) => { a.yes.name = a.no.name = `q${i}`; });
  const name = h('input', { autocomplete: 'name', placeholder: 'Type your full name' });
  const agree = h('input', { type: 'checkbox' });
  mount($app(),
    h('h1', {}, onboarding ? 'Step 2 of 2: Health screening' : 'Health screening & waiver'),
    h('div', { class: 'card' },
      h('p', { class: 'muted small' }, 'Answer yes or no. A “yes” doesn’t stop you training; your coach may ask for clearance from a doctor first.'),
      PARQ.map((q, i) => h('div', { class: 'field' },
        h('div', {}, `${i + 1}. ${q}`),
        h('div', { class: 'row' },
          h('label', { class: 'check' }, answers[i].yes, 'Yes'),
          h('label', { class: 'check' }, answers[i].no, 'No'))))),
    h('div', { class: 'card' },
      h('h3', {}, 'Liability waiver'),
      h('p', { class: 'small' }, WAIVER_TEXT),
      field('Signature', name),
      h('label', { class: 'check' }, agree, 'I have read and agree to the waiver'),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          if (answers.some((a) => !a.yes.checked && !a.no.checked)) throw new Error('Answer every question');
          const w = await post(`/clients/${me.userId}/waiver`, {
            waiverVersion: WAIVER_VERSION,
            parq: PARQ.map((q, i) => ({ question: q, answer: answers[i].yes.checked })),
            signedName: name.value, agreed: agree.checked,
          });
          if (w.parqFlagged) toast('Thanks. Your coach will follow up about medical clearance.');
          else toast('Signed. You’re all set!');
          go('/');
        }),
      }, 'Sign')),
  );
}

export function waiverSummary(w) {
  if (!w) return h('div', { class: 'banner' }, 'Waiver not signed yet.');
  return h('div', { class: 'card' },
    h('div', { class: 'spread' },
      h('strong', {}, `Signed by ${w.signedName}`),
      w.parqFlagged ? h('span', { class: 'chip warn' }, 'Screening: needs clearance') : h('span', { class: 'chip good' }, 'Screening clear')),
    h('p', { class: 'muted small' }, `${fmtDateTime(w.signedAt)} · version ${w.waiverVersion}${w.sourceIp ? ` · IP ${w.sourceIp}` : ''}`),
    h('table', {}, w.parq.map((a) => h('tr', {}, h('td', {}, a.question), h('td', {}, a.answer ? h('strong', {}, 'Yes') : 'No')))));
}

export function intakeSummary(intake, profile) {
  if (!intake) return h('div', { class: 'banner' }, 'Intake not completed yet.');
  const u = units(session.profile);
  const age = profile.birthDate ? Math.floor((Date.now() - Date.parse(profile.birthDate)) / 31557600000) : null;
  const row = (k, v) => v && h('div', { class: 'field' }, h('label', {}, k), h('div', { style: 'white-space:pre-wrap' }, v));
  return h('div', { class: 'card' },
    row('Goals', intake.goals),
    row('Training history', intake.trainingHistory),
    row('Equipment', (intake.equipment || []).join(', ')),
    row('Schedule', intake.schedule),
    row('Injuries / limitations', intake.injuries),
    row('Details', [intake.sex, age != null && `${age} y`, num(intake.heightCm) && `${round(u.toL(intake.heightCm), 1)} ${u.l}`].filter(Boolean).join(' · ')));
}
