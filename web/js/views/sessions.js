// Scheduling (BZ-2). Coaches: #/schedule lists upcoming sessions with every
// client, books one at any time, and sets weekly availability. Clients:
// #/sessions books an open slot (shown in their own time zone) and cancels.
// Both get a private calendar feed link for their phone's calendar.

import { api, get } from '../api.js';
import cfg from '../config.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { $app, busy, confirmSheet, field, h, mount, select, sheet, toast } from '../ui.js';

const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
const dayLabel = (iso) => new Date(iso).toLocaleDateString(undefined, { weekday: 'long', month: 'short', day: 'numeric' });
const timeLabel = (iso) => new Date(iso).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
const byDay = (items, at) => {
  const out = new Map();
  for (const x of items) {
    const d = dayLabel(at(x));
    if (!out.has(d)) out.set(d, []);
    out.get(d).push(x);
  }
  return out;
};

function sessionRow(s, onCancel) {
  const cancelled = s.status === 'cancelled';
  const with_ = session.isCoach ? s.clientName : s.coachName;
  return h('div', { class: `session-row ${cancelled ? 'cancelled' : ''}` },
    h('div', {},
      h('strong', {}, `${timeLabel(s.start)} · ${with_}`), h('span', { class: 'muted small' }, ` · ${s.minutes} min`),
      cancelled && h('span', { class: 'chip gray' }, 'Cancelled'),
      (s.location || s.videoLink || s.notes) && h('div', { class: 'small muted' },
        [s.location, s.notes].filter(Boolean).join(' · '),
        s.videoLink && h('a', { href: s.videoLink, target: '_blank', rel: 'noopener' }, ' Join video'))),
    !cancelled && new Date(s.start) > new Date() && h('button', {
      class: 'small ghost',
      onclick: async () => {
        if (!(await confirmSheet(`Cancel the ${dayLabel(s.start)} ${timeLabel(s.start)} session? ${session.isCoach ? s.clientName : s.coachName} will be told.`, 'Cancel session'))) return;
        await api('POST', `/clients/${s.clientId}/sessions/${s.id}/cancel`);
        toast('Session cancelled');
        onCancel();
      },
    }, 'Cancel'));
}

function sessionList(list, onCancel) {
  const upcoming = list.filter((s) => new Date(s.start) > new Date(Date.now() - 3600e3));
  if (!upcoming.length) return h('div', { class: 'card muted' }, 'No sessions booked.');
  return [...byDay(upcoming, (s) => s.start)].map(([day, items]) => h('div', { class: 'card' },
    h('h3', {}, day), items.map((s) => sessionRow(s, onCancel))));
}

/** The private calendar feed, for Profile. */
export function calendarCard() {
  const box = h('div');
  const show = ({ path }) => {
    const url = cfg.apiUrl + path;
    mount(box,
      h('input', { value: url, readonly: true, onfocus: (e) => e.target.select(), 'aria-label': 'Calendar feed link' }),
      h('div', { class: 'row', style: 'margin-top:8px' },
        h('button', { class: 'grow', onclick: async () => { await navigator.clipboard.writeText(url); toast('Copied'); } }, 'Copy link'),
        h('a', { class: 'btn grow', href: url.replace(/^https?:/, 'webcal:') }, 'Add to calendar'),
        h('button', {
          class: 'ghost',
          onclick: async () => {
            if (!(await confirmSheet('Make a new link? The old one stops working, so calendars using it stop updating.', 'New link'))) return;
            show(await api('POST', '/me/calendar/reset'));
          },
        }, 'New link')));
  };
  get('/me/calendar').then(show).catch(() => mount(box, h('p', { class: 'muted small' }, 'Couldn’t load your calendar link.')));
  return h('div', { class: 'card' },
    h('h3', {}, 'Calendar'),
    h('p', { class: 'muted small' }, session.isCoach
      ? 'Your sessions in Google, Apple or Outlook Calendar. The link is private: anyone with it can see your schedule.'
      : 'Your sessions and workouts in Google, Apple or Outlook Calendar. The link is private: anyone with it can see your schedule.'),
    box);
}

// ---------------------------------------------------------------- coach

export async function renderSchedule() {
  const [list, { availability, timeZone }, roster] = await Promise.all([get('/sessions'), get('/availability'), get('/clients', { cache: true })]);
  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Schedule'), h('button', { class: 'primary', onclick: () => bookSheet(roster) }, '+ Book')),
    sessionList(list, renderSchedule),
    availabilityCard(availability, timeZone),
  );
}

function bookSheet(roster) {
  const client = select(roster.map((r) => [r.client.clientId, r.client.name]), roster[0]?.client.clientId);
  const date = h('input', { type: 'date', value: new Date(Date.now() + 864e5).toISOString().slice(0, 10) });
  const time = h('input', { type: 'time', value: '09:00', step: 900 });
  const minutes = h('input', { type: 'number', min: 15, max: 240, step: 15, value: 60 });
  const notes = h('input', { maxlength: 500, placeholder: 'Optional, e.g. deadlift technique' });
  const sh = sheet(
    h('h2', {}, 'Book a session'),
    roster.length === 0 ? h('p', { class: 'muted' }, 'Invite a client first.') : [
      field('Client', client),
      h('div', { class: 'grid2' }, field('Date', date), field('Time (your time)', time)),
      field('Length (minutes)', minutes),
      field('Notes', notes),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          const start = new Date(`${date.value}T${time.value}`).toISOString();
          await api('POST', `/clients/${client.value}/sessions`, { start, minutes: Number(minutes.value), notes: notes.value });
          toast('Session booked; your client has been told');
          sh.close();
          renderSchedule();
        }),
      }, 'Book')],
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
  );
}

function availabilityCard(av, timeZone) {
  const windows = av.windows.map((w) => ({ ...w }));
  const list = h('div');
  const t5 = (s) => s.slice(0, 5); // "08:00:00" → "08:00"
  const draw = () => mount(list,
    windows.length === 0 && h('p', { class: 'muted small' }, 'No hours yet: clients can’t book until you add some.'),
    windows.map((w, i) => h('div', { class: 'row avail-row' },
      h('select', { 'aria-label': 'Day', onchange: (e) => { w.day = Number(e.target.value); } }, DAYS.map((d, k) => h('option', { value: k, selected: k === w.day }, d))),
      h('input', { type: 'time', value: t5(w.start), 'aria-label': 'From', step: 900, onchange: (e) => { w.start = `${e.target.value}:00`; } }),
      h('span', {}, '–'),
      h('input', { type: 'time', value: t5(w.end), 'aria-label': 'To', step: 900, onchange: (e) => { w.end = `${e.target.value}:00`; } }),
      h('button', { class: 'small ghost', 'aria-label': 'Remove these hours', onclick: () => { windows.splice(i, 1); draw(); } }, '✕'))));
  draw();
  const minutes = h('input', { type: 'number', min: 15, max: 240, step: 15, value: av.minutes });
  const notice = h('input', { type: 'number', min: 0, max: 168, value: av.noticeHours });
  const location = h('input', { value: av.location || '', maxlength: 200, placeholder: 'e.g. Iron Gym, 12 Main St' });
  const link = h('input', { type: 'url', value: av.videoLink || '', placeholder: 'https://… (for video sessions)' });
  return h('div', { class: 'card' },
    h('h2', {}, 'When clients can book'),
    h('p', { class: 'muted small' }, `Weekly hours in your time zone (${timeZone}). Clients see open times in theirs.`),
    list,
    h('button', { class: 'small', onclick: () => { windows.push({ day: 0, start: '08:00:00', end: '12:00:00' }); draw(); } }, '+ Add hours'),
    h('div', { class: 'grid2', style: 'margin-top:10px' }, field('Session length (min)', minutes), field('Booking notice (hours)', notice)),
    field('Location', location),
    field('Video link', link),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await api('PUT', '/availability', { windows, minutes: Number(minutes.value), noticeHours: Number(notice.value), location: location.value || null, videoLink: link.value || null });
        toast('Saved');
      }),
    }, 'Save hours'));
}

// ---------------------------------------------------------------- client

/** For the client's Today screen: the next session, and a way to book. Fills `slot`. */
export function nextSessionCard(slot) {
  get('/sessions').then((list) => {
    const next = list.find((s) => s.status === 'booked' && new Date(s.start) > new Date());
    mount(slot, h('a', { class: 'card link next-session', href: '#/sessions' },
      h('span', {}, '📅 ', next ? h('strong', {}, `Next session: ${dayLabel(next.start)}, ${timeLabel(next.start)}`) : 'Book a session with your coach'),
      h('span', { class: 'muted' }, '›')));
  }).catch(() => { /* offline */ });
}

export async function renderSessions() {
  const me = session.profile;
  const [list, open] = await Promise.all([get('/sessions'), get(`/clients/${me.userId}/slots?days=14`)]);
  const slots = open.slots;
  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/') }, '‹ Today'),
    h('h1', {}, 'Sessions'),
    sessionList(list, renderSessions),
    h('h2', {}, 'Book a session'),
    h('p', { class: 'muted small' }, `${open.minutes}-minute sessions${open.location ? ` · ${open.location}` : ''}. Times are in your time zone.`),
    slots.length === 0
      ? h('div', { class: 'card muted' }, 'No open times in the next two weeks. Message your coach to find one.')
      : [...byDay(slots, (s) => s)].map(([day, times]) => h('div', { class: 'card' },
        h('h3', {}, day),
        h('div', { class: 'slot-grid' }, times.map((t) => h('button', {
          class: 'small',
          onclick: async (e) => {
            const btn = e.currentTarget; // gone from the event once we await
            if (!(await confirmSheet(`Book ${day}, ${timeLabel(t)}?`, 'Book'))) return;
            await busy(btn, async () => {
              try {
                await api('POST', `/clients/${me.userId}/sessions`, { start: t });
                toast('Booked; your coach has been told');
              } catch (err) {
                if (err.status !== 409) throw err;
                toast(err.body?.current?.message || 'That time was just taken', 'error');
              }
              renderSessions();
            });
          },
        }, timeLabel(t)))))),
  );
}
