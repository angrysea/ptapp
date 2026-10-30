// Coach: client roster with invites (ON-1), and the per-client page with the
// progress dashboard (LG-6), calendar/assignments (WB-4), logs + feedback
// (FB-1), metrics, intake/waiver and messages.

import { del, get, post } from '../api.js';
import { RATINGS } from '../effort.js';
import { cardioLogText, isCardioSet } from '../cardio.js';
import { go } from '../router.js';
import { session } from '../session.js';
import {
  $app, addDays, barChart, busy, confirmSheet, errorToast, field, fmtDate, fmtDateTime, h, isoDate, lineChart, mount, round, select,
  sheet, subtabs, toast, todayIso, units,
} from '../ui.js';
import { playVideo } from '../videos.js';
import { refreshVideoBadge } from './videos.js';
import { commentOn, renderThread } from './comments.js';
import { intakeSummary, waiverSummary } from './intake.js';
import { renderMetrics, stat } from './metrics.js';
import { statusChip } from './workout.js';
import { suggestionsCard } from './progression.js';
import { alertsCard } from './alerts.js';
import { checkinsTab } from './checkins.js';
import { habitsCard } from './habits.js';
import { overviewCard } from './meals.js';
import { requestsCard } from './joinrequests.js';

/** The client's 1-5 rating of a workout; bad ones stand out. */
function ratingChip(n) {
  const [, label, face] = RATINGS[n - 1] || [];
  return label && h('span', { class: `chip ${n <= 2 ? 'bad' : n >= 4 ? 'good' : 'gray'}` }, `${face} Rated ${n}/5 · ${label}`);
}

export async function renderRoster() {
  const [rows, invites, suggestions, requests] = await Promise.all([get('/clients', { cache: true }), get('/invites').catch(() => []), suggestionsCard(),
    requestsCard({ onChange: renderRoster, emailResult })]);
  const alerts = await alertsCard(); // after the roster, which files new missed-workout alerts
  rows.sort((a, b) => (b.painFlag - a.painFlag) || ((a.adherence.pct ?? 101) - (b.adherence.pct ?? 101)));
  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Clients'),
      h('div', { class: 'row' }, h('a', { class: 'btn', href: '#/schedule' }, '📅 Schedule'), h('button', { class: 'primary', onclick: inviteSheet }, '+ Invite'))),
    rows.length === 0 && h('div', { class: 'card muted' }, 'No clients yet. Invite your first client, or share your sign-up link below.'),
    alerts,
    suggestions,
    requests,
    rows.map((r) => {
      const a = r.adherence;
      const flags = [
        r.painFlag && h('span', { class: 'chip bad' }, 'Pain reported'),
        r.parqFlagged && h('span', { class: 'chip warn' }, 'Needs clearance'),
        !r.waiverSigned && h('span', { class: 'chip warn' }, 'No waiver'),
        !r.hasUpcoming && h('span', { class: 'chip gray' }, 'No program'),
        a.missed > 0 && h('span', { class: 'chip warn' }, `${a.missed} missed`),
      ].filter(Boolean);
      return h('div', { class: 'card link', onclick: () => go(`/c/${r.client.clientId}`) },
        h('div', { class: 'spread' },
          h('div', {}, h('strong', {}, r.client.name), h('div', { class: 'muted small' }, r.client.email)),
          h('div', { class: 'center' }, h('div', { class: 'stat' }, a.pct != null ? `${Math.round(a.pct)}%` : '—'), h('div', { class: 'stat-label' }, '28-day adherence'))),
        h('div', { class: 'row small', style: 'margin-top:6px' },
          h('span', { class: 'muted' }, r.lastWorkout ? `Last workout ${fmtDate(r.lastWorkout)}` : 'No workouts logged'), flags));
    }),
    invites.length > 0 && [h('h2', {}, 'Pending invites'), invites.map((i) => h('div', { class: 'card spread' },
      h('div', {}, h('strong', {}, i.clientName || i.email), h('div', { class: 'muted small' }, `${i.email} · code ${i.code}`)),
      h('div', { class: 'row' },
        h('button', { class: 'small', onclick: (e) => busy(e.currentTarget, async () => emailResult(await post(`/invites/${i.code}/resend`))) }, 'Resend email'),
        h('button', { class: 'small', onclick: () => shareInvite(i) }, 'Share'),
        h('button', { class: 'small danger', onclick: async () => { await del(`/invites/${i.code}`); renderRoster(); } }, 'Revoke'))))],
  );
}

function inviteLink(code) {
  return `${location.origin}${location.pathname}#/join/${code}`;
}

/** Says whether the invitation email went out; if not, offers the link to share. */
function emailResult(inv) {
  if (inv.emailed) toast(`Invitation emailed to ${inv.email}`);
  else shareInvite(inv, inv.emailError ? 'The email couldn’t be sent, so share this link with them yourself.' : 'Email isn’t set up here, so share this link with them yourself.');
}

function shareInvite(inv, note) {
  const link = inviteLink(inv.code);
  const subject = encodeURIComponent(`${inv.coachName} invited you to train together`);
  const bodyText = encodeURIComponent(`Hi ${inv.clientName || ''},\n\nJoin me on PT Coach: ${link}\n\nCreate an account with this email (${inv.email}) and you'll be connected automatically. Your invite code is ${inv.code}.`);
  const sh = sheet(
    h('h2', {}, 'Invite link'),
    note && h('p', { class: 'banner' }, note),
    h('p', { class: 'small' }, `Valid for 14 days. Code: `, h('strong', {}, inv.code)),
    h('input', { value: link, readonly: true, onfocus: (e) => e.target.select() }),
    h('div', { class: 'row', style: 'margin-top:10px' },
      h('button', { class: 'grow', onclick: async () => { await navigator.clipboard.writeText(link); toast('Copied'); } }, 'Copy link'),
      h('a', { class: 'btn primary grow', href: `mailto:${inv.email}?subject=${subject}&body=${bodyText}` }, 'Email'),
      navigator.share && h('button', { class: 'grow', onclick: () => navigator.share({ title: 'PT Coach invite', url: link }).catch(() => {}) }, 'Share…')),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Done'),
  );
}

function inviteSheet() {
  const name = h('input', { autocomplete: 'off' });
  const email = h('input', { type: 'email', autocomplete: 'off' });
  const sh = sheet(
    h('h2', {}, 'Invite a client'),
    h('p', { class: 'small muted' }, 'They get an email with a link to create their account with this address.'),
    field('Client name', name),
    field('Email', email),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const inv = await post('/invites', { clientName: name.value, email: email.value });
        sh.close();
        emailResult(inv);
        renderRoster();
      }),
    }, 'Send invitation'),
  );
}

// ------------------------------------------------------------ client page

export async function renderClient(cid, tab = 'overview', focus = null) {
  let detail;
  try { detail = await get(`/clients/${cid}`, { cache: true }); } catch (e) { errorToast(e); return go('/'); }
  const client = detail.profile;
  const body = h('div');
  const tabs = [['overview', 'Overview'], ['calendar', 'Calendar'], ['logs', 'Logs'], ['checkins', 'Check-ins'], ['metrics', 'Body'], ['intake', 'Intake'], ['messages', 'Messages']];
  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/') }, '‹ Clients'),
    h('h1', {}, client.name),
    h('p', { class: 'muted small' }, client.email),
    subtabs(tabs, tabs.some(([k]) => k === tab) ? tab : 'overview', (t) => go(`/c/${cid}/${t}`)),
    body);
  const views = { overview, calendar, logs, checkins: checkinsTab, metrics: (el) => renderMetrics(el, client), intake: (el) => mount(el, h('h3', {}, 'Intake'), intakeSummary(detail.intake, client), h('h3', {}, 'Health screening & waiver'), waiverSummary(detail.waiver)), messages: (el) => renderThread(el, cid) };
  try { await views[tab](body, client, focus); } catch (e) { mount(body, h('p', { class: 'muted' }, e.message)); }
  body.querySelector('.card.focus')?.scrollIntoView({ block: 'center' });
}

async function overview(el, client) {
  const u = units(session.profile);
  const d = await get(`/clients/${client.userId}/dashboard?weeks=8`);
  const a = d.adherence;
  mount(el,
    h('a', { class: 'btn block', href: `#/c/${client.userId}/report`, style: 'margin-bottom:10px' }, '📄 Progress report (print or PDF)'),
    h('div', { class: 'card grid2' },
      stat(a.pct != null ? `${Math.round(a.pct)}%` : '—', 'Adherence (8 wks)'),
      stat(d.last28.pct != null ? `${Math.round(d.last28.pct)}%` : '—', 'Last 28 days'),
      stat(`${a.completed}/${a.due}`, 'Workouts done'),
      stat(a.missed, 'Missed')),
    a.missedDates.length > 0 && h('div', { class: 'card small' }, h('strong', {}, 'Missed: '), a.missedDates.slice(-8).map((x) => fmtDate(x)).join(', ')),
    h('div', { class: 'card' }, h('h3', {}, `Weekly volume (${u.w} × reps)`),
      barChart(d.weeklyVolume.map((w) => ({ x: fmtDate(w.weekStart, { month: 'short', day: 'numeric' }), y: u.toW(w.volumeKg) })))),
    d.weeklyVolume.some((w) => w.cardioSec > 0) && h('div', { class: 'card' }, h('h3', {}, 'Weekly cardio (minutes)'),
      barChart(d.weeklyVolume.map((w) => ({ x: fmtDate(w.weekStart, { month: 'short', day: 'numeric' }), y: Math.round(w.cardioSec / 60) })))),
    h('div', { class: 'card' }, h('h3', {}, `Estimated 1RM (${u.w})`),
      d.strength.length === 0 ? h('p', { class: 'muted small' }, 'Appears once loaded sets are logged.')
        : lineChart(d.strength.slice(0, 4).map((s) => ({ label: `${s.exerciseName} (best ${u.toW(s.bestE1rmKg)})`, points: s.points.map((p) => ({ x: p.date, y: u.toW(p.e1rmKg) })) })))),
    h('div', { class: 'card' }, h('h3', {}, 'Recent check-ins'),
      d.checks.length === 0 ? h('p', { class: 'muted small' }, 'None yet.')
        : h('table', {}, h('tr', {}, h('th', {}, 'Date'), h('th', {}, 'RPE'), h('th', {}, 'Energy'), h('th', {}, 'Sleep'), h('th', {}, 'Sore'), h('th', {}, 'Pain')),
          d.checks.map((c) => h('tr', {}, h('td', {}, fmtDate(c.date)), h('td', {}, c.check.rpe ?? ''), h('td', {}, c.check.energy ?? ''),
            h('td', {}, c.check.sleep ?? ''), h('td', {}, c.check.soreness ?? ''),
            h('td', {}, c.check.pain ? h('span', { class: 'chip bad' }, c.check.painLocation || 'yes') : ''))))),
    await habitsCard(client),
    await overviewCard(client),
  );
}

async function calendar(el, client) {
  const from = addDays(todayIso(), -14);
  const to = addDays(todayIso(), 56);
  const [programs, starters, assignments, sched, logs] = await Promise.all([
    get('/programs', { cache: true }),
    get('/programs/starters', { cache: true }).catch(() => []),
    get(`/clients/${client.userId}/assignments`),
    get(`/clients/${client.userId}/schedule?from=${from}&to=${to}`),
    get(`/clients/${client.userId}/logs?from=${from}&to=${to}`),
  ]);
  const bySched = Object.fromEntries(logs.filter((l) => l.scheduledId).map((l) => [l.scheduledId, l]));
  // The client's own programs run as they are; one of the coach's templates
  // or a starter program is copied for the client first (routes/training.rs).
  const theirs = programs.filter((p) => p.clientId === client.userId);
  const templates = programs.filter((p) => !p.clientId);
  const opt = (value, p) => h('option', { value }, `${p.name} (${p.weeks} wk)`);
  const prog = h('select', { 'aria-label': 'Program' },
    theirs.length > 0 && h('optgroup', { label: `${client.name}’s programs` }, theirs.map((p) => opt(`p:${p.id}`, p))),
    templates.length > 0 && h('optgroup', { label: 'Your programs (makes a copy for this client)' }, templates.map((p) => opt(`p:${p.id}`, p))),
    starters.length > 0 && h('optgroup', { label: 'Starter programs (makes a copy for this client)' }, starters.map((p) => opt(`s:${p.id}`, p))));
  const copies = () => !prog.value.startsWith('p:') || !theirs.some((p) => `p:${p.id}` === prog.value);
  const start = h('input', { type: 'date', value: todayIso() });
  const weeks = {};
  for (const s of sched) {
    const d = new Date(s.date + 'T12:00:00');
    d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
    (weeks[isoDate(d)] ||= []).push(s);
  }
  mount(el,
    h('div', { class: 'card' }, h('h3', {}, 'Assign a program'),
      programs.length + starters.length === 0 ? h('p', { class: 'muted' }, 'Create a program first.', ' ', h('a', { href: '#/programs' }, 'Programs'))
        : [h('div', { class: 'grid2' }, field('Program', prog), field('Start date', start)),
          h('p', { class: 'muted small' }, 'Weights written as a % of 1RM are filled in from this client’s maxes (Progress › Maxes); without one, the program’s own weight is used.'),
          h('button', {
            class: 'primary block',
            onclick: (e) => busy(e.currentTarget, async () => {
              const [kind, id] = [prog.value.slice(0, 1), prog.value.slice(2)];
              const copied = copies();
              await post(`/clients/${client.userId}/assignments`, { ...(kind === 's' ? { starterKey: id } : { programId: id }), startDate: start.value });
              toast(copied ? `Program assigned. ${client.name} has their own copy to personalize under “${client.name}’s programs”.` : 'Program assigned');
              calendar(el, client);
            }),
          }, 'Assign')],
      theirs.length > 0 && [h('h3', { style: 'margin-top:14px' }, `${client.name}’s programs`),
        theirs.map((p) => h('div', { class: 'spread', style: 'margin:6px 0' },
          h('div', {}, h('strong', {}, p.name), h('div', { class: 'muted small' }, `${p.weeks} weeks · editing updates upcoming workouts`)),
          h('a', { class: 'btn small', href: `#/programs/${p.id}` }, 'Edit')))]),
    assignments.length > 0 && h('div', { class: 'card' }, h('h3', {}, 'Assignments'),
      assignments.map((a) => h('div', { class: 'spread', style: 'margin:6px 0' },
        h('div', {}, h('strong', {}, a.programName), h('div', { class: 'muted small' }, `${fmtDate(a.startDate)} → ${fmtDate(a.endDate)}`)),
        h('button', {
          class: 'small danger', onclick: async () => {
            if (!(await confirmSheet('Remove this program’s upcoming workouts? Logged and past workouts are kept.', 'Remove'))) return;
            const r = await del(`/clients/${client.userId}/assignments/${a.id}`);
            toast(`Removed ${r.removed} upcoming workouts`);
            calendar(el, client);
          },
        }, 'Remove')))),
    Object.entries(weeks).map(([wk, items]) => h('div', { class: 'card' }, h('h3', {}, `Week of ${fmtDate(wk)}`),
      items.map((s) => h('div', { class: 'spread', style: 'margin:6px 0' },
        h('div', {}, h('strong', {}, s.workout.name), h('div', { class: 'muted small' }, `${fmtDate(s.date)} · ${s.programName}`)),
        statusChip(bySched[s.id], s.date))))),
  );
}

async function logs(el, client, focus = null) {
  const u = units(session.profile);
  const [list, comments, videos] = await Promise.all([
    get(`/clients/${client.userId}/logs?from=${addDays(todayIso(), -60)}&to=${todayIso()}`),
    get(`/clients/${client.userId}/comments`),
    get(`/clients/${client.userId}/videos`).catch(() => []),
  ]);
  const byLog = {};
  for (const c of comments) if (c.logId) (byLog[c.logId] ||= []).push(c);
  const videosFor = (logId, itemId) => videos.filter((v) => v.logId === logId && v.itemId === itemId).sort((a, b) => a.setIndex - b.setIndex);
  const refresh = () => logs(el, client, focus);
  mount(el,
    list.length === 0 && h('p', { class: 'muted' }, 'No workouts logged in the last 60 days.'),
    [...list].reverse().map((l) => {
      const byEx = {};
      for (const s of l.sets) (byEx[`${s.itemId}`] ||= { name: `${s.exerciseName}${s.phase === 'warmup' ? ' (warm-up)' : ''}`, sets: [] }).sets.push(s);
      const c = l.check;
      return h('div', { class: `card ${l.id === focus ? 'focus' : ''}`, 'data-log': l.id },
        h('div', { class: 'spread' }, h('strong', {}, l.workoutName), statusChip(l, l.date)),
        h('div', { class: 'muted small' }, `${fmtDate(l.date)} · updated ${fmtDateTime(l.updatedAt)}`),
        l.rating && ratingChip(l.rating),
        Object.values(l.blockTimes ?? {}).map((t) => h('div', { class: 'small' }, `⏱ For time: ${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`)),
        l.comment && h('p', { style: 'white-space:pre-wrap' }, `“${l.comment}”`),
        c && h('div', { class: 'small row' },
          c.rpe != null && h('span', {}, `sRPE ${c.rpe}`), c.energy != null && h('span', {}, `energy ${c.energy}/5`),
          c.sleep != null && h('span', {}, `sleep ${c.sleep}/5`), c.soreness != null && h('span', {}, `soreness ${c.soreness}/5`),
          c.pain && h('span', { class: 'chip bad' }, `Pain: ${c.painLocation || 'unspecified'}`)),
        Object.entries(byEx).map(([itemId, ex]) => h('div', { class: 'exercise' }, h('strong', { class: 'small' }, ex.name),
          l.exerciseNotes?.[itemId] && h('div', { class: 'exnote-view small' }, `📝 “${l.exerciseNotes[itemId]}”`),
          videosFor(l.id, itemId).length > 0 && h('div', { class: 'videobar' },
            videosFor(l.id, itemId).map((v) => h('button', {
              class: 'small', title: v.reviewedAt ? 'Reviewed' : 'Not reviewed yet',
              onclick: () => {
                const sh = playVideo(v, h('button', { class: 'block', onclick: () => { sh.close(); go(`/review/${client.userId}/${v.id}`); } }, '🎙 Reply with video'), !v.reviewedAt && h('button', {
                  class: 'primary block',
                  onclick: (e) => busy(e.currentTarget, async () => {
                    await post(`/clients/${client.userId}/videos/${v.id}/reviewed`);
                    sh.close();
                    toast('Marked reviewed');
                    refreshVideoBadge();
                    refresh();
                  }),
                }, '✓ Mark reviewed'));
              },
            }, `▶ Set ${v.setIndex + 1}${v.reviewedAt ? ' ✓' : ''}`))),
          // A warm-up is ticked off as a whole, so one chip says it all.
          h('div', { class: 'row small' }, ex.sets.sort((a, b) => a.setIndex - b.setIndex).slice(0, ex.sets[0]?.phase === 'warmup' ? 1 : undefined).map((s) => h('button', {
            class: `small ${s.done ? '' : 'ghost'}`, title: 'Comment on this set',
            onclick: async () => { if (await commentOn(client.userId, l.id, s.setIndex, s.itemId)) refresh(); },
          }, `${s.phase === 'warmup' ? (s.done ? 'Done' : 'Not done')
            : isCardioSet(s) ? `${cardioLogText(s, u)}${s.done ? '' : ' (not done)'}`
              : `${s.loadKg != null ? `${round(u.toW(s.loadKg), 1)}${u.w}` : 'BW'} × ${s.reps ?? '–'}${s.rpe ? ` @${s.rpe}` : ''}${s.done ? '' : ' (not done)'}`}`))))),
        (byLog[l.id] || []).map((cm) => h('div', { class: 'small', style: 'margin-top:6px' },
          h('strong', {}, `${cm.authorName}${cm.setIndex != null ? ` (set ${cm.setIndex + 1})` : ''}: `), cm.text)),
        h('button', { class: 'small', style: 'margin-top:8px', onclick: async () => { if (await commentOn(client.userId, l.id)) refresh(); } }, '💬 Comment on workout'));
    }),
  );
}
