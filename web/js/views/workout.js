// Client "Today" screen and workout logger (LG-1..LG-4, FB-2).
// Everything here works offline: the schedule comes from the read cache,
// in-progress sets are saved as a local draft, and the finished log goes
// through the outbox with a client-generated id.

import { enqueue, get } from '../api.js';
import { kv } from '../idb.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { blockTimer, restTimer } from '../timer.js';
import { $app, addDays, field, fmtDate, h, mount, num, select, sheet, toast, todayIso, uid, units } from '../ui.js';

const KIND_LABEL = { straight: '', superset: 'Superset', circuit: 'Circuit', amrap: 'AMRAP', emom: 'EMOM', interval: 'Intervals' };

/** Server logs merged with locally finished logs that may not have synced yet. */
async function loadWeek(cid) {
  const from = addDays(todayIso(), -7);
  const to = addDays(todayIso(), 21);
  const [sched, logs, local] = await Promise.all([
    get(`/clients/${cid}/schedule?from=${from}&to=${to}`, { cache: true }).catch(() => []),
    get(`/clients/${cid}/logs?from=${from}&to=${to}`, { cache: true }).catch(() => []),
    kv.get('localLogs').then((m) => m || {}),
  ]);
  // Forget local copies the server has caught up with.
  const serverIds = new Map(logs.map((l) => [l.id, l.updatedAt]));
  let pruned = false;
  for (const [id, l] of Object.entries(local)) {
    if (serverIds.has(id) && serverIds.get(id) >= l.updatedAt) { delete local[id]; pruned = true; }
  }
  if (pruned) kv.set('localLogs', local);
  const bySched = {};
  for (const l of [...logs, ...Object.values(local)]) {
    if (!l.scheduledId) continue;
    const prev = bySched[l.scheduledId];
    if (!prev || prev.updatedAt < l.updatedAt) bySched[l.scheduledId] = l;
  }
  return { sched, bySched };
}

export function statusChip(log, date) {
  if (log?.status === 'completed') return h('span', { class: 'chip good' }, 'Done');
  if (log?.status === 'modified') return h('span', { class: 'chip good' }, 'Done (modified)');
  if (log?.status === 'skipped') return h('span', { class: 'chip gray' }, 'Skipped');
  if (date < todayIso()) return h('span', { class: 'chip bad' }, 'Missed');
  if (date === todayIso()) return h('span', { class: 'chip' }, 'Today');
  return null;
}

export async function renderToday() {
  const me = session.profile;
  const [{ sched, bySched }, detail] = await Promise.all([
    loadWeek(me.userId),
    get(`/clients/${me.userId}`, { cache: true }).catch(() => null),
  ]);
  const today = todayIso();
  const todays = sched.filter((s) => s.date === today);
  const upcoming = sched.filter((s) => s.date > today).slice(0, 6);
  const recent = sched.filter((s) => s.date < today).reverse();

  const card = (s, isToday) => h('div', { class: `card link ${isToday ? 'today' : ''}`, onclick: () => go(`/workout/${s.date}/${s.id}`) },
    h('div', { class: 'spread' }, h('strong', {}, s.workout.name), statusChip(bySched[s.id], s.date)),
    h('div', { class: 'muted small' }, `${fmtDate(s.date)} · ${s.programName} · week ${s.week}`),
    h('div', { class: 'small' }, s.workout.blocks.flatMap((b) => b.items.map((i) => i.exerciseName)).join(' · ')));

  mount($app(),
    detail && (!detail.intake || !detail.waiver) && h('div', { class: 'banner' },
      'Finish setting up: ',
      !detail.intake && h('a', { href: '#/intake?onboarding=1' }, 'intake questionnaire'),
      !detail.intake && !detail.waiver && ' and ',
      !detail.waiver && h('a', { href: '#/waiver?onboarding=1' }, 'health screening & waiver')),
    h('h1', {}, `Hi ${me.name.split(' ')[0]}`),
    h('h2', {}, 'Today'),
    todays.length ? todays.map((s) => card(s, true)) : h('div', { class: 'card muted' }, 'Rest day. Nothing scheduled today.'),
    upcoming.length > 0 && [h('h2', {}, 'Coming up'), upcoming.map((s) => card(s, false))],
    recent.length > 0 && [h('h2', {}, 'Last 7 days'), recent.map((s) => card(s, false))],
    !sched.length && h('p', { class: 'muted' }, 'Your coach hasn’t assigned a program yet.'),
  );
}

function firstInt(reps) {
  const m = String(reps || '').match(/\d+/);
  return m ? Number(m[0]) : null;
}

function freshLog(s) {
  const sets = [];
  for (const b of s.workout.blocks) {
    for (const it of b.items) {
      for (let i = 0; i < it.sets; i++) {
        sets.push({ itemId: it.id, exerciseId: it.exerciseId, exerciseName: it.exerciseName, setIndex: i,
          reps: firstInt(it.reps), loadKg: it.loadKg ?? null, rpe: null, done: false });
      }
    }
  }
  return { id: uid(), scheduledId: s.id, date: s.date, workoutName: s.workout.name, status: 'inProgress', sets, comment: '', check: null, updatedAt: new Date().toISOString() };
}

export async function renderWorkout(date, schedId) {
  const me = session.profile;
  const u = units(me);
  const { sched, bySched } = await loadWeek(me.userId);
  let s = sched.find((x) => x.id === schedId);
  if (!s) {
    const wide = await get(`/clients/${me.userId}/schedule?from=${date}&to=${date}`, { cache: true }).catch(() => []);
    s = wide.find((x) => x.id === schedId);
  }
  if (!s) return mount($app(), h('p', {}, 'Workout not found (it may not be cached for offline use yet).'));

  const draftKey = `draft:${schedId}`;
  let log = (await kv.get(draftKey)) || structuredClone(bySched[schedId] || freshLog(s));
  const saveDraft = () => { log.updatedAt = new Date().toISOString(); kv.set(draftKey, log); };
  const setsFor = (itemId) => log.sets.filter((x) => x.itemId === itemId).sort((a, b) => a.setIndex - b.setIndex);

  const exerciseInfo = (id) => session.exercises?.[id];

  function setRow(set, item, block) {
    const reps = h('input', { type: 'number', inputmode: 'numeric', value: set.reps ?? '', 'aria-label': 'reps' });
    const load = h('input', { type: 'number', inputmode: 'decimal', step: 'any', value: set.loadKg != null ? u.toW(set.loadKg) : '', 'aria-label': u.w });
    const rpe = h('input', { type: 'number', inputmode: 'decimal', step: '0.5', min: 1, max: 10, value: set.rpe ?? '', 'aria-label': 'RPE' });
    const row = h('div', { class: `setrow ${set.done ? 'is-done' : ''}` });
    const sync = () => { set.reps = num(reps.value); set.loadKg = u.fromW(load.value); set.rpe = num(rpe.value); saveDraft(); };
    for (const i of [reps, load, rpe]) i.addEventListener('change', sync);
    const done = h('button', {
      class: 'done', 'aria-label': 'set done',
      onclick: () => {
        sync();
        set.done = !set.done;
        row.classList.toggle('is-done', set.done);
        saveDraft();
        const rest = item.restSec ?? block.restSec;
        if (set.done && block.kind === 'straight' && rest) restTimer(rest);
      },
    }, '✓');
    row.append(h('div', { class: 'n' }, set.setIndex + 1), reps, load, rpe, done);
    return row;
  }

  function prescription(it) {
    const parts = [`${it.sets} × ${it.reps}`];
    if (it.loadKg != null) parts.push(`@ ${u.toW(it.loadKg)} ${u.w}`);
    if (it.rpe != null) parts.push(`RPE ${it.rpe}`);
    if (it.rir != null) parts.push(`${it.rir} RIR`);
    if (it.tempo) parts.push(`tempo ${it.tempo}`);
    if (it.restSec) parts.push(`rest ${it.restSec}s`);
    return parts.join(' · ');
  }

  function blockCard(b) {
    const head = [];
    const label = KIND_LABEL[b.kind];
    if (label) {
      const detail = b.kind === 'amrap' || b.kind === 'emom'
        ? `${Math.round((b.durationSec || 0) / 60)} min`
        : [b.rounds && `${b.rounds} rounds`, b.workSec && `${b.workSec}s on`, b.restSec && `${b.restSec}s off`].filter(Boolean).join(' · ');
      head.push(h('div', { class: 'spread' },
        h('h3', {}, `${label} ${detail ? '· ' + detail : ''}`),
        h('button', { class: 'small primary', onclick: () => blockTimer(b) }, '⏱ Timer')));
    }
    return h('div', { class: 'card' }, head, b.notes && h('p', { class: 'muted small' }, b.notes),
      b.items.map((it) => {
        const info = exerciseInfo(it.exerciseId);
        return h('div', { class: 'exercise' },
          h('div', { class: 'spread' }, h('strong', {}, it.exerciseName),
            info && (info.cues?.length || info.demoUrl || info.demoVideoUrl) &&
              h('button', { class: 'small ghost', onclick: () => showExercise(info) }, 'How to')),
          h('div', { class: 'prescription' }, prescription(it)),
          it.notes && h('div', { class: 'small' }, it.notes),
          h('div', { class: 'sethead' }, h('span', {}, 'Set'), h('span', {}, 'Reps'), h('span', {}, u.w), h('span', {}, 'RPE'), h('span', {})),
          setsFor(it.id).map((set) => setRow(set, it, b)));
      }));
  }

  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/') }, '‹ Back'),
    h('h1', {}, s.workout.name),
    h('p', { class: 'muted' }, `${fmtDate(s.date)} · ${s.programName}`, ' ', statusChip(bySched[schedId], s.date)),
    s.workout.notes && h('div', { class: 'card small' }, s.workout.notes),
    s.workout.blocks.map(blockCard),
    h('div', { class: 'row' },
      h('button', { class: 'grow', onclick: () => finish('skipped') }, 'Skip workout'),
      h('button', { class: 'primary grow', onclick: () => finish(null) }, 'Finish workout')),
  );

  function finish(preset) {
    const allDone = log.sets.every((x) => x.done);
    const status = select([['completed', 'Completed as written'], ['modified', 'Completed with changes'], ['skipped', 'Skipped']],
      preset || (allDone ? 'completed' : 'modified'));
    const comment = h('textarea', { value: log.comment || '', placeholder: 'How did it go? Anything your coach should know?' });
    const c = log.check || {};
    const scale = (v) => select([['', '—'], ...[1, 2, 3, 4, 5].map((n) => [String(n), String(n)])], v != null ? String(v) : '');
    const rpe = h('input', { type: 'number', inputmode: 'decimal', min: 1, max: 10, step: '0.5', value: c.rpe ?? '' });
    const energy = scale(c.energy), sleep = scale(c.sleep), soreness = scale(c.soreness);
    const pain = h('input', { type: 'checkbox', checked: !!c.pain });
    const painWhere = h('input', { value: c.painLocation || '', placeholder: 'Where? e.g. left knee' });
    const sh = sheet(
      h('h2', {}, 'Finish workout'),
      field('Status', status),
      field('Comment', comment),
      h('h3', {}, 'Post-workout check'),
      h('div', { class: 'grid2' },
        field('Session RPE (1-10)', rpe),
        field('Energy (1-5)', energy),
        field('Sleep last night (1-5)', sleep),
        field('Soreness (1-5)', soreness)),
      h('label', { class: 'check' }, pain, 'I felt pain'),
      painWhere,
      h('div', { class: 'row', style: 'margin-top:12px' },
        h('button', { class: 'grow', onclick: () => sh.close() }, 'Cancel'),
        h('button', {
          class: 'primary grow',
          onclick: async () => {
            log.status = status.value;
            log.comment = comment.value;
            log.check = { rpe: num(rpe.value), energy: num(energy.value), sleep: num(sleep.value), soreness: num(soreness.value),
              pain: pain.checked, painLocation: pain.checked ? painWhere.value || null : null };
            log.updatedAt = new Date().toISOString();
            const local = (await kv.get('localLogs')) || {};
            local[log.id] = log;
            await kv.set('localLogs', local);
            await kv.del(draftKey);
            await enqueue('PUT', `/clients/${me.userId}/logs/${log.id}`, log, `log:${log.id}`);
            sh.close();
            toast(navigator.onLine ? 'Workout saved' : 'Saved offline. It will sync when you’re back online.');
            go('/');
          },
        }, 'Save')),
    );
  }
}

function showExercise(info) {
  const sh = sheet(
    h('h2', {}, info.name),
    info.demoVideoUrl && h('video', { src: info.demoVideoUrl, controls: true, playsinline: true, style: 'width:100%;border-radius:10px' }),
    info.demoUrl && h('p', {}, h('a', { href: info.demoUrl, target: '_blank', rel: 'noopener' }, 'Watch demo')),
    info.cues?.length > 0 && h('ul', {}, info.cues.map((c) => h('li', {}, c))),
    h('p', { class: 'muted small' }, [...(info.muscleGroups || []), ...(info.equipment || [])].join(' · ')),
    h('button', { class: 'block', onclick: () => sh.close() }, 'Close'),
  );
}
