// #/automation: a coach's reminders and automated messages (MS-4). The
// hourly sweep (api sweep.rs) sends them at each client's local time, at
// most once a day each; reminders go only to clients with notifications on.

import { api, get } from '../api.js';
import { $app, busy, field, h, mount, toast } from '../ui.js';

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const hourLabel = (n) => `${n % 12 || 12}:00 ${n < 12 ? 'am' : 'pm'}`;

function hourSelect(value, onChange, label) {
  return h('select', { 'aria-label': label, onchange: (e) => onChange(Number(e.target.value)) },
    Array.from({ length: 24 }, (_, n) => h('option', { value: n, selected: n === value }, hourLabel(n))));
}

/** Day toggles; none picked means every day. */
function dayPicker(days, onChange, label) {
  const row = h('div', { class: 'days', role: 'group', 'aria-label': label });
  const draw = () => mount(row, DAYS.map((d, i) => h('button', {
    type: 'button', class: `small ${days.includes(i) ? 'primary' : ''}`, 'aria-pressed': String(days.includes(i)),
    onclick: () => { const k = days.indexOf(i); if (k >= 0) days.splice(k, 1); else days.push(i); onChange(days); draw(); },
  }, d)));
  draw();
  return row;
}

export async function renderAutomation() {
  const a = await get('/automation');
  const redraw = () => draw();
  const body = h('div');

  function reminder(key, title, help, dayChoice) {
    const r = a[key];
    return h('div', { class: 'card' },
      h('label', { class: 'check' },
        h('input', { type: 'checkbox', checked: !!r, onchange: (e) => { a[key] = e.target.checked ? { days: dayChoice ? [6] : [], hour: dayChoice ? 17 : 7 } : null; redraw(); } }),
        h('strong', {}, title)),
      h('p', { class: 'muted small' }, help),
      r && [
        dayChoice && field('On', dayPicker(r.days, (d) => { r.days = d; }, `${title} days`)),
        field('At (client’s local time)', hourSelect(r.hour, (n) => { r.hour = n; }, `${title} time`)),
      ]);
  }

  function messageCard(m, i) {
    return h('div', { class: 'card' },
      field('Message', h('textarea', { value: m.text, maxlength: 1000, placeholder: 'New week, {name}! Here’s what we’re focusing on…', oninput: (e) => { m.text = e.target.value; } }), '{name} becomes the client’s first name.'),
      field('Days (none picked = every day)', dayPicker(m.when.days, (d) => { m.when.days = d; }, 'Message days')),
      field('At (client’s local time)', hourSelect(m.when.hour, (n) => { m.when.hour = n; }, 'Message time')),
      h('div', { class: 'spread' },
        h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: m.active, onchange: (e) => { m.active = e.target.checked; } }), ' On'),
        h('button', { class: 'small danger', onclick: () => { a.messages.splice(i, 1); redraw(); } }, 'Delete')));
  }

  function draw() {
    mount(body,
      h('h2', {}, 'Reminders'),
      h('p', { class: 'muted small' }, 'Sent as notifications to clients who have them turned on.'),
      reminder('checkinReminder', 'Weekly check-in reminder', 'Reminds clients who haven’t sent this week’s check-in yet.', true),
      reminder('workoutReminder', 'Workout reminder', 'On days a client has a workout scheduled, reminds them if they haven’t logged it yet.', false),
      h('h2', {}, 'Automated messages'),
      h('p', { class: 'muted small' }, 'Sent to every client in their conversation with you, as if you typed them.'),
      a.messages.length === 0 && h('div', { class: 'card muted' }, 'None yet.'),
      a.messages.map(messageCard),
      h('button', {
        class: 'block',
        onclick: () => { a.messages.push({ id: `m${Date.now().toString(36)}`, text: '', when: { days: [0], hour: 8 }, active: true }); redraw(); },
      }, '+ Add an automated message'),
      h('button', {
        class: 'primary block', style: 'margin-top:12px',
        onclick: (e) => busy(e.currentTarget, async () => {
          Object.assign(a, await api('PUT', '/automation', a));
          toast('Saved');
          redraw();
        }),
      }, 'Save'));
  }

  mount($app(),
    h('a', { class: 'btn ghost small', href: '#/me' }, '‹ Profile'),
    h('h1', {}, 'Reminders & automated messages'),
    h('p', { class: 'muted' }, 'Checked every hour; each one goes out at most once a day, at the client’s local time.'),
    body);
  draw();
}

/** For the coach's Profile page. */
export function automationCard() {
  return h('div', { class: 'card' },
    h('h3', {}, 'Reminders & automated messages'),
    h('p', { class: 'muted small' }, 'Check-in and workout reminders, and messages that go out on a schedule.'),
    h('a', { class: 'btn block', href: '#/automation' }, 'Set up reminders & messages'));
}
