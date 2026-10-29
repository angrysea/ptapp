// Weekly check-ins (FB-3). The client answers their coach's questions once a
// week (a card on Today opens #/checkin); the coach compares the last four
// weeks on the client's Check-ins tab and edits the questions from Profile.

import { api, get } from '../api.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { $app, busy, field, fmtDate, h, mount, sheet, toast } from '../ui.js';

const KINDS = [['scale', '1-10 scale'], ['number', 'Number'], ['yesno', 'Yes / no'], ['text', 'Text']];

const answerText = (q, v) => {
  if (v == null) return '—';
  if (q.kind === 'yesno') return v ? 'Yes' : 'No';
  if (q.kind === 'number' && q.unit) return `${v} ${q.unit}`;
  return String(v);
};

/** For the client's Today screen: this week's check-in, sent or not. Fills `slot` when online. */
export function checkinCard(slot, me) {
  get(`/clients/${me.userId}/checkins`).then(({ thisWeek, checkins }) => {
    const sent = checkins.find((c) => c.weekStart === thisWeek);
    mount(slot, sent
      ? h('p', { class: 'small muted' }, '✓ Weekly check-in sent. ', h('a', { href: '#/checkin' }, 'Edit'))
      : h('div', { class: 'card checkin-due' },
        h('strong', {}, '📝 Weekly check-in'),
        h('p', { class: 'small muted' }, 'A few questions so your coach knows how your week went.'),
        h('a', { class: 'btn primary block', href: '#/checkin' }, 'Start check-in')));
  }).catch(() => { /* offline: the card just doesn't show */ });
}

/** #/checkin: the client answers this week's questions (or edits their answers). */
export async function renderCheckin() {
  const me = session.profile;
  const { thisWeek, template, checkins } = await get(`/clients/${me.userId}/checkins`);
  const existing = checkins.find((c) => c.weekStart === thisWeek);
  const questions = existing?.questions || template.questions;
  const answers = { ...(existing?.answers || {}) };
  const control = (q) => {
    const v = answers[q.id];
    if (q.kind === 'scale') {
      const row = h('div', { class: 'scale', role: 'radiogroup', 'aria-label': q.label });
      const draw = () => mount(row, Array.from({ length: 10 }, (_, i) => i + 1).map((n) => h('button', {
        type: 'button', role: 'radio', 'aria-checked': String(answers[q.id] === n), class: answers[q.id] === n ? 'on' : '',
        onclick: () => { answers[q.id] = n; draw(); },
      }, String(n))));
      draw();
      return row;
    }
    if (q.kind === 'yesno') {
      const row = h('div', { class: 'row', role: 'radiogroup', 'aria-label': q.label });
      const draw = () => mount(row, [[true, 'Yes'], [false, 'No']].map(([val, label]) => h('button', {
        type: 'button', role: 'radio', 'aria-checked': String(answers[q.id] === val), class: answers[q.id] === val ? 'primary' : '',
        onclick: () => { answers[q.id] = val; draw(); },
      }, label)));
      draw();
      return row;
    }
    if (q.kind === 'number') {
      return h('input', { type: 'number', inputmode: 'decimal', step: 'any', value: v ?? '', 'aria-label': q.label,
        oninput: (e) => { answers[q.id] = e.target.value === '' ? null : Number(e.target.value); } });
    }
    return h('textarea', { value: v ?? '', maxlength: 2000, 'aria-label': q.label, oninput: (e) => { answers[q.id] = e.target.value; } });
  };
  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/') }, '‹ Today'),
    h('h1', {}, 'Weekly check-in'),
    h('p', { class: 'muted' }, `Week of ${fmtDate(thisWeek)}`),
    h('div', { class: 'card' },
      questions.map((q) => h('div', { class: 'checkin-q' },
        h('label', {}, q.label, q.required ? '' : h('span', { class: 'muted small' }, ' (optional)'), q.kind === 'number' && q.unit ? h('span', { class: 'muted small' }, ` · ${q.unit}`) : ''),
        control(q))),
      h('button', {
        class: 'primary block', style: 'margin-top:12px',
        onclick: (e) => busy(e.currentTarget, async () => {
          const missing = questions.find((q) => q.required && (answers[q.id] == null || answers[q.id] === ''));
          if (missing) return toast(`“${missing.label}” needs an answer`, 'error');
          await api('PUT', `/clients/${me.userId}/checkins/${thisWeek}`, { answers });
          toast(existing ? 'Check-in updated' : 'Check-in sent to your coach');
          go('/');
        }),
      }, existing ? 'Save changes' : 'Send check-in')),
  );
}

/** The coach's Check-ins tab: the last four weeks side by side. */
export async function checkinsTab(el, client) {
  const { checkins } = await get(`/clients/${client.userId}/checkins`);
  if (!checkins.length) return mount(el, h('div', { class: 'card muted' }, 'No check-ins yet. Clients see a check-in card on their Today screen each week.'));
  const weeks = checkins.slice(0, 4);
  // Rows follow the newest check-in's questions, then any older ones it lacks.
  const rows = [];
  for (const c of weeks) for (const q of c.questions) if (!rows.some((r) => r.id === q.id)) rows.push(q);
  mount(el,
    h('div', { class: 'table-wrap' },
      h('table', { class: 'checkins' },
        h('thead', {}, h('tr', {}, h('th', {}, ''), weeks.map((c) => h('th', {}, fmtDate(c.weekStart))))),
        h('tbody', {}, rows.map((q) => h('tr', {},
          h('th', { scope: 'row' }, q.label),
          weeks.map((c) => h('td', {}, answerText(q, c.answers[q.id])))))))),
    h('p', { class: 'muted small' }, `Newest first. ${checkins.length > 4 ? 'Older weeks are kept too.' : ''}`),
  );
}

/** For the coach's Profile page: edit the check-in questions. */
export function checkinSettingsCard() {
  return h('div', { class: 'card' },
    h('h3', {}, 'Weekly check-in'),
    h('p', { class: 'muted small' }, 'Your clients answer these each week from their Today screen.'),
    h('button', { class: 'block', onclick: () => editQuestions() }, 'Edit check-in questions'));
}

async function editQuestions() {
  const t = await get('/checkin-template');
  const qs = t.questions.map((q) => ({ ...q }));
  const list = h('div');
  const draw = () => mount(list, qs.map((q, i) => h('div', { class: 'item-card' },
    field('Question', h('input', { value: q.label, maxlength: 120, oninput: (e) => { q.label = e.target.value; } })),
    h('div', { class: 'grid2' },
      field('Answer', h('select', { onchange: (e) => { q.kind = e.target.value; draw(); } },
        KINDS.map(([k, label]) => h('option', { value: k, selected: q.kind === k }, label)))),
      q.kind === 'number' && field('Unit', h('input', { value: q.unit || '', maxlength: 20, placeholder: 'e.g. lb', oninput: (e) => { q.unit = e.target.value; } }))),
    h('div', { class: 'spread' },
      h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: !!q.required, onchange: (e) => { q.required = e.target.checked; } }), ' Required'),
      h('div', { class: 'row' },
        h('button', { class: 'small ghost', 'aria-label': 'Move up', onclick: () => { if (i) { [qs[i - 1], qs[i]] = [qs[i], qs[i - 1]]; draw(); } } }, '↑'),
        h('button', { class: 'small ghost', 'aria-label': `Remove “${q.label}”`, onclick: () => { qs.splice(i, 1); draw(); } }, '✕'))))));
  draw();
  const sh = sheet(
    h('h2', {}, 'Check-in questions'),
    h('p', { class: 'muted small' }, 'Changes apply from the next check-in; past weeks keep the questions they answered.'),
    list,
    h('button', { class: 'block', onclick: () => { qs.push({ id: `q${Date.now().toString(36)}`, kind: 'scale', label: '', required: false }); draw(); } }, '+ Add a question'),
    h('div', { class: 'row', style: 'margin-top:8px' },
      h('button', { class: 'ghost grow', onclick: () => sh.close() }, 'Cancel'),
      h('button', {
        class: 'primary grow',
        onclick: (e) => busy(e.currentTarget, async () => {
          await api('PUT', '/checkin-template', { questions: qs.map((q) => ({ ...q, unit: q.kind === 'number' ? q.unit || null : null })) });
          toast('Check-in questions saved');
          sh.close();
        }),
      }, 'Save')),
  );
}
