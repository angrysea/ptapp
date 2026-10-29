// Client "Today" screen and workout logger (LG-1..LG-4, FB-2).
// Everything here works offline: the schedule comes from the read cache,
// in-progress sets are saved as a local draft, and the finished log goes
// through the outbox with a client-generated id.

import { del, enqueue, get, onSync, post } from '../api.js';
import { kv } from '../idb.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { blockTimer, restTimer } from '../timer.js';
import { RATINGS, rpeLogInfo, sessionRpeInfo } from '../effort.js';
import { byPhase, cardioText, distUnits, phaseOf, zoneInfo } from '../cardio.js';
import { $app, addDays, busy, confirmSheet, field, fmtDate, h, mount, num, select, sheet, toast, todayIso, uid, units } from '../ui.js';
import { CannotEncode, canEncode, encodeClip } from '../encode.js';
import { onUploads, pendingUploads, queueVideo, removeUpload, retryUpload } from '../uploads.js';
import { askBeforeCellular, filmingTip, onCellular, playReply, playVideo } from '../videos.js';
import { canRecord, recordClip } from './recorder.js';
import { trimmer } from './trim.js';
import { celebrate } from './records.js';
import { commentOn } from './comments.js';
import { checkinCard } from './checkins.js';
import { habitTiles } from './habits.js';
import { todayCard } from './meals.js';
import { nextSessionCard } from './sessions.js';

// A finished workout the server refused for good (not a 409, which means a
// newer copy is already saved) goes back to being a draft, so it is neither
// lost nor stuck showing "Done" from the local copy.
onSync(async (_, failure) => {
  const ref = failure?.item.ref || '';
  if (!ref.startsWith('log:') || failure.error.status === 409) return;
  const local = (await kv.get('localLogs')) || {};
  const log = local[ref.slice(4)];
  if (!log) return;
  delete local[log.id];
  await kv.set('localLogs', local);
  if (log.scheduledId) await kv.set(`draft:${log.scheduledId}`, log);
  toast('Your workout was kept as a draft so you can fix it and save again.', 'error');
});

const KIND_LABEL = { straight: '', superset: 'Superset', circuit: 'Circuit', amrap: 'AMRAP', emom: 'EMOM', interval: 'Intervals' };

/** Cache schedule/logs per client (not per date range), so the offline cache
 * survives midnight: items accumulate across days and are pruned to a
 * generous window. Returns {items, fresh} — fresh=false means the fetch failed. */
async function merged(path, from, to, key) {
  const map = new Map(Object.entries((await kv.get(key)) || {}));
  let fresh = false;
  try {
    const items = await get(`${path}?from=${from}&to=${to}`);
    // The server's answer is complete for this range: drop cached items in it
    // that it no longer returns (e.g. workouts removed by an unassign).
    for (const [id, it] of map) if (it.date >= from && it.date <= to) map.delete(id);
    for (const it of items) map.set(it.id, it);
    fresh = true;
  } catch { /* offline or server error: serve what we have */ }
  const lo = addDays(todayIso(), -120), hi = addDays(todayIso(), 120);
  const kept = [...map.values()].filter((it) => it.date >= lo && it.date <= hi);
  kv.set(key, Object.fromEntries(kept.map((it) => [it.id, it]))).catch(() => {});
  return { items: kept.filter((it) => it.date >= from && it.date <= to), fresh };
}

/** Server logs merged with locally finished logs that may not have synced yet. */
async function loadWeek(cid) {
  const from = addDays(todayIso(), -7);
  const to = addDays(todayIso(), 21);
  const [schedR, logsR, local] = await Promise.all([
    merged(`/clients/${cid}/schedule`, from, to, `cache:sched:${cid}`),
    merged(`/clients/${cid}/logs`, from, to, `cache:logs:${cid}`),
    kv.get('localLogs').then((m) => m || {}),
  ]);
  const sched = schedR.items, logs = logsR.items;
  // Forget local copies the server has caught up with.
  const serverIds = new Map(logs.map((l) => [l.id, l.updatedAt]));
  let pruned = false;
  for (const [id, l] of Object.entries(local)) {
    if (serverIds.has(id) && serverIds.get(id) >= l.updatedAt) { delete local[id]; pruned = true; }
  }
  if (pruned) await kv.set('localLogs', local);
  // Drafts for scheduled workouts that no longer exist (coach reassigned the
  // program) are unreachable; drop them so they don't accumulate.
  if (schedR.fresh) {
    for (const k of await kv.keys()) {
      if (k.startsWith('draft:') && !sched.some((s) => s.id === k.slice(6))) await kv.del(k);
    }
  }
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

  // A workout the client started but never finished shows as "Started" so
  // there is a path back to the draft.
  const drafts = {};
  for (const s of todays) {
    const d = await kv.get(`draft:${s.id}`);
    if (d) drafts[s.id] = d;
  }
  const chip = (s) => bySched[s.id] ? statusChip(bySched[s.id], s.date)
    : drafts[s.id] ? h('span', { class: 'chip' }, 'Started')
    : statusChip(null, s.date);

  const card = (s, isToday) => h('div', { class: `card link ${isToday ? 'today' : ''}`, onclick: () => go(`/workout/${s.date}/${s.id}`) },
    h('div', { class: 'spread' }, h('strong', {}, s.workout.name), chip(s)),
    h('div', { class: 'muted small' }, `${fmtDate(s.date)} · ${s.programName} · week ${s.week}`),
    h('div', { class: 'small' }, s.workout.blocks.flatMap((b) => b.items.map((i) => i.exerciseName)).join(' · ')));

  const checkinSlot = h('div');
  checkinCard(checkinSlot, me); // FB-3: fills in once it has loaded (online only)
  const sessionSlot = h('div');
  nextSessionCard(sessionSlot); // BZ-2 (online only)
  const habitSlot = h('div');
  habitTiles(habitSlot, me); // BZ-3: works offline from the cached list
  const mealSlot = h('div');
  todayCard(mealSlot, me); // food journal: same offline pattern
  mount($app(),
    detail && (!detail.intake || !detail.waiver) && h('div', { class: 'banner' },
      'Finish setting up: ',
      !detail.intake && h('a', { href: '#/intake?onboarding=1' }, 'intake questionnaire'),
      !detail.intake && !detail.waiver && ' and ',
      !detail.waiver && h('a', { href: '#/waiver?onboarding=1' }, 'health screening & waiver')),
    h('h1', {}, `Hi ${me.name.split(' ')[0]}`),
    checkinSlot,
    sessionSlot,
    h('h2', {}, 'Today'),
    todays.length ? todays.map((s) => card(s, true)) : h('div', { class: 'card muted' }, 'Rest day. Nothing scheduled today.'),
    habitSlot,
    mealSlot,
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
    const phase = phaseOf(b);
    for (const it of b.items) {
      // Cardio starts at what was prescribed, like reps and load do; warm-ups
      // are only ticked off, so they log no amounts.
      const c = phase !== 'warmup' && it.cardio;
      for (let i = 0; i < it.sets; i++) {
        sets.push({ itemId: it.id, exerciseId: it.exerciseId, exerciseName: it.exerciseName, setIndex: i,
          reps: it.cardio ? null : firstInt(it.reps), loadKg: it.cardio ? null : it.loadKg ?? null, rpe: null, done: false, phase,
          durationSec: c ? c.durationSec ?? null : null, distanceM: c ? c.distanceM ?? null : null, avgHr: null });
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
    const wide = (await merged(`/clients/${me.userId}/schedule`, date, date, `cache:sched:${me.userId}`)).items;
    s = wide.find((x) => x.id === schedId);
  }
  if (!s) return mount($app(), h('p', {}, 'Workout not found (it may not be cached for offline use yet).'));

  const draftKey = `draft:${schedId}`;
  let log = (await kv.get(draftKey)) || structuredClone(bySched[schedId] || freshLog(s));
  const saveDraft = () => { log.updatedAt = new Date().toISOString(); kv.set(draftKey, log); };
  const setsFor = (itemId) => log.sets.filter((x) => x.itemId === itemId).sort((a, b) => a.setIndex - b.setIndex);

  const exerciseInfo = (id) => session.exercises?.[id];

  // Videos already uploaded for this workout's sets (needs a connection).
  let videos = [];
  let replies = []; // the coach's replies to them (VR-2)
  const loadVideos = async () => {
    try {
      [videos, replies] = await Promise.all([
        get(`/clients/${me.userId}/videos?logId=${encodeURIComponent(log.id)}`),
        get(`/clients/${me.userId}/replies`).catch(() => []),
      ]);
    } catch { /* offline: none shown */ }
  };
  await loadVideos();

  // Clips saved on this device and still uploading (uploads.js), shown as
  // chips beside the uploaded ones; bars redraw as uploads progress or land.
  let pending = (await pendingUploads()).filter((u) => u.logId === log.id);
  const bars = [];
  const redrawBars = () => bars.forEach((draw) => draw());
  const stopWatching = onUploads(async (list) => {
    if (!bars.some((b) => b.el.isConnected)) return stopWatching(); // left this workout
    const mine = list.filter((u) => u.logId === log.id);
    const landed = mine.length < pending.length;
    pending = mine;
    if (landed) await loadVideos();
    redrawBars();
  });

  function deleteVideo(v, sh) {
    return h('button', {
      class: 'danger block',
      onclick: async () => {
        if (!(await confirmSheet('Delete this video? Your coach won’t see it any more.', 'Delete'))) return;
        await del(`/clients/${me.userId}/videos/${v.id}`);
        sh().close();
        await loadVideos();
        redrawBars();
        toast('Video deleted');
      },
    }, 'Delete video');
  }

  function uploadChip(u) {
    const label = u.state === 'failed' ? `⚠ Set ${u.set.setIndex + 1} · failed`
      : u.progress != null && navigator.onLine ? `⏳ Set ${u.set.setIndex + 1} · ${Math.round(u.progress * 100)}%`
        : `⏳ Set ${u.set.setIndex + 1} · ${!navigator.onLine ? 'waiting for connection' : u.waitForWifi && onCellular() ? 'waiting for Wi-Fi' : 'waiting'}`;
    return h('button', {
      class: `small ${u.state === 'failed' ? 'danger' : ''}`,
      onclick: () => {
        const s = sheet(
          h('h2', {}, `Set ${u.set.setIndex + 1} video`),
          h('p', {}, u.state === 'failed' ? `The upload failed: ${u.error}`
            : navigator.onLine ? 'It’s uploading. You can leave this page; it carries on next time the app is open.'
              : 'It’s saved on this device and uploads when you’re back online.'),
          u.state === 'failed' && h('button', { class: 'primary block', onclick: () => { s.close(); retryUpload(u.key); } }, 'Try again'),
          h('button', {
            class: 'danger block',
            onclick: async () => {
              if (!(await confirmSheet('Remove this video from the upload queue?', 'Remove'))) return;
              s.close();
              removeUpload(u.key);
            },
          }, 'Remove'),
          h('button', { class: 'ghost block', onclick: () => s.close() }, 'Close'),
        );
      },
    }, label);
  }

  /** Per exercise: ▶ for each set with a video, chips for uploads in progress, and Add video. */
  function videoBar(it) {
    const bar = h('div', { class: 'videobar' });
    const draw = () => mount(bar,
      videos.filter((v) => v.itemId === it.id).sort((a, b) => a.setIndex - b.setIndex)
        .map((v) => [h('button', {
          class: 'small',
          onclick: () => { let sh; sh = playVideo(v, deleteVideo(v, () => sh)); },
        }, `▶ Set ${v.setIndex + 1}`),
        replies.filter((r) => r.videoId === v.id).map((r) => h('button', {
          class: `small reply ${r.seenAt ? '' : 'new'}`,
          onclick: () => playReply(r, {
            title: `Coach’s reply · set ${v.setIndex + 1}`,
            onPlay: async () => {
              if (r.seenAt) return;
              r.seenAt = new Date().toISOString();
              draw();
              post(`/clients/${me.userId}/replies/${r.id}/seen`).catch(() => {});
            },
          }),
        }, r.seenAt ? '🎙 Coach reply' : '🎙 Coach reply · new'))]),
      pending.filter((u) => u.set.itemId === it.id).map(uploadChip),
      h('button', { class: 'small ghost', onclick: () => addVideo(it) }, '🎥 Add video'));
    bars.push(Object.assign(draw, { el: bar }));
    draw();
    return bar;
  }

  function addVideo(it) {
    const sets = setsFor(it.id);
    const lastDone = [...sets].reverse().find((x) => x.done) || sets[0];
    const which = select(sets.map((x) => [String(x.setIndex), `Set ${x.setIndex + 1}`]), String(lastDone.setIndex));
    // No `capture` on the picker: on iOS that records at 480×360, so
    // recording goes through the in-app recorder instead.
    const file = h('input', { type: 'file', accept: 'video/*' });
    const slot = h('div');
    const prog = h('progress', { value: 0, max: 1, hidden: true });
    const status = h('p', { class: 'small muted' }, 'Record the set or choose a clip, then pick up to 60 seconds of it.');
    let clip = null, trim = null;
    const converting = new AbortController();
    const useClip = (f, duration) => {
      trim?.destroy();
      clip = f;
      trim = f ? trimmer(f, { duration }) : null;
      mount(slot, trim?.el);
    };
    file.addEventListener('change', () => useClip(file.files?.[0] || null));
    const record = canRecord() && h('button', {
      class: 'block', type: 'button',
      onclick: async () => {
        const r = await recordClip();
        if (r) { file.value = ''; useClip(r.file, r.durationSec); }
      },
    }, '● Record with camera');
    const sh = sheet(
      h('h2', {}, `Video: ${it.exerciseName}`),
      filmingTip(session.exercises?.[it.exerciseId], me),
      field('Set', which),
      record,
      field(record ? 'Or choose a clip' : 'Clip', file),
      slot, prog, status,
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          const f = clip;
          if (!f) throw new Error('Record or choose a clip first');
          const duration = await trim.ready;
          const r = trim.range();
          saveDraft(); // the video names this log, so make sure the draft exists
          prog.hidden = false;
          try {
            // Convert the selection on this device; if it can't, upload the
            // original with trim points (sent only when the selection is
            // shorter than the clip) and the player keeps to them.
            const cut = duration != null && (r.start > 0.05 || r.end < duration - 0.05) ? { start: r.start, end: r.end } : null;
            let upload = { file: f, durationSec: duration, trim: cut, poster: null };
            if (duration != null && canEncode()) {
              try {
                const out = await encodeClip(f, {
                  start: r.start, end: r.end, signal: converting.signal,
                  onProgress: (x) => { prog.value = x; status.textContent = `Preparing video… ${Math.round(x * 100)}%`; },
                });
                upload = { file: out.blob, durationSec: out.durationSec, trim: null, poster: out.poster };
              } catch (err) {
                // Any conversion failure (not only the expected CannotEncode) falls
                // back to the original: recordings vary by browser.
                if (err?.name === 'AbortError') throw err;
                console.warn('uploading the original:', err instanceof CannotEncode ? err.message : err);
                status.textContent = 'This device can’t convert the clip, so the original will upload.';
              }
            }
            // On mobile data, a big clip can wait for Wi-Fi.
            const when = await askBeforeCellular(upload.file.size);
            if (!when) { prog.hidden = true; status.textContent = 'Not uploaded.'; return; }
            // Saved on this device first, so it survives a lost connection or a
            // closed app; uploads.js sends it in the background.
            await queueVideo({ clientId: me.userId, logId: log.id, set: sets.find((x) => String(x.setIndex) === which.value), ...upload, waitForWifi: when.waitForWifi });
            sh.close();
            toast(when.waitForWifi ? 'Saved on this device. It uploads when you’re on Wi-Fi.'
              : navigator.onLine ? 'Uploading in the background' : 'Saved on this device. It uploads when you’re back online.');
          } catch (err) { prog.hidden = true; status.textContent = err.message; throw err; }
        }),
      }, 'Upload'),
      h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'),
    );
    sh.onClose = () => { converting.abort(); trim?.destroy(); };
  }

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

  /** A set done for time or distance: minutes, distance, average heart rate, RPE. */
  function cardioRow(set, item) {
    const d = distUnits(u);
    const mins = h('input', { type: 'number', inputmode: 'decimal', step: 'any', min: 0, value: set.durationSec != null ? Math.round((set.durationSec / 60) * 100) / 100 : '', 'aria-label': 'minutes' });
    const dist = h('input', { type: 'number', inputmode: 'decimal', step: 'any', min: 0, value: d.to(set.distanceM) ?? '', 'aria-label': d.unit });
    const hr = h('input', { type: 'number', inputmode: 'numeric', min: 30, max: 250, value: set.avgHr ?? '', 'aria-label': 'average heart rate' });
    const rpe = h('input', { type: 'number', inputmode: 'decimal', step: '0.5', min: 1, max: 10, value: set.rpe ?? '', 'aria-label': 'RPE' });
    const row = h('div', { class: `setrow cardio ${set.done ? 'is-done' : ''}` });
    const sync = () => {
      set.durationSec = num(mins.value) != null ? Math.round(num(mins.value) * 60) : null;
      set.distanceM = d.from(dist.value);
      set.avgHr = num(hr.value) != null ? Math.round(num(hr.value)) : null;
      set.rpe = num(rpe.value);
      saveDraft();
    };
    for (const i of [mins, dist, hr, rpe]) i.addEventListener('change', sync);
    const done = h('button', {
      class: 'done', 'aria-label': 'done',
      onclick: () => {
        sync();
        set.done = !set.done;
        row.classList.toggle('is-done', set.done);
        saveDraft();
        if (set.done && item.restSec && set.setIndex < item.sets - 1) restTimer(item.restSec);
      },
    }, '✓');
    row.append(h('div', { class: 'n' }, set.setIndex + 1), mins, dist, hr, rpe, done);
    return row;
  }

  function prescription(it) {
    if (it.cardio) return cardioText(it, u, me.birthDate);
    const parts = [`${it.sets} × ${it.reps}`];
    if (it.loadKg != null) parts.push(`@ ${u.toW(it.loadKg)} ${u.w}${it.loadPct != null ? ` (${it.loadPct}%)` : ''}`);
    else if (it.loadPct != null) parts.push(`@ ${it.loadPct}% of your 1RM`);
    if (it.rpe != null) parts.push(`RPE ${it.rpe}`);
    if (it.rir != null) parts.push(`${it.rir} RIR`);
    if (it.tempo) parts.push(`tempo ${it.tempo}`);
    if (it.restSec) parts.push(`rest ${it.restSec}s`);
    return parts.join(' · ');
  }

  /** Warm-up blocks: what to do, ticked off as a whole. */
  function warmupCard(b) {
    const sets = log.sets.filter((x) => b.items.some((it) => it.id === x.itemId));
    const isDone = () => sets.length > 0 && sets.every((x) => x.done);
    const btn = h('button', {
      class: `block ${isDone() ? 'primary' : ''}`, 'aria-pressed': String(isDone()),
      onclick: () => {
        const to = !isDone();
        for (const x of sets) x.done = to;
        saveDraft();
        btn.classList.toggle('primary', to);
        btn.setAttribute('aria-pressed', String(to));
        btn.textContent = to ? '✓ Warm-up done' : 'Mark warm-up done';
      },
    }, isDone() ? '✓ Warm-up done' : 'Mark warm-up done');
    const label = KIND_LABEL[b.kind];
    return h('div', { class: 'card phase-warmup' },
      h('div', { class: 'spread' }, h('h3', {}, 'Warm-up', label ? ` · ${label}` : ''),
        label && h('button', { class: 'small', onclick: () => blockTimer(b) }, '⏱ Timer')),
      b.notes && h('p', { class: 'muted small' }, b.notes),
      h('ul', { class: 'warmup-list' }, b.items.map((it) => {
        const info = exerciseInfo(it.exerciseId);
        return h('li', {}, h('strong', {}, it.exerciseName), ' ', h('span', { class: 'muted small' }, prescription(it)),
          it.notes && h('div', { class: 'small' }, it.notes),
          info && (info.cues?.length || info.demoUrl || info.demoVideoUrl) && h('button', { class: 'small ghost', onclick: () => showExercise(info) }, 'How to'));
      })),
      btn);
  }

  function blockCard(b) {
    if (phaseOf(b) === 'warmup') return warmupCard(b);
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
    const post = phaseOf(b) === 'post';
    return h('div', { class: `card ${post ? 'phase-post' : ''}` }, post && h('h3', {}, 'Post-workout cardio'), head, b.notes && h('p', { class: 'muted small' }, b.notes),
      b.items.map((it) => {
        const info = exerciseInfo(it.exerciseId);
        return h('div', { class: 'exercise' },
          h('div', { class: 'spread' }, h('strong', {}, it.exerciseName),
            h('div', { class: 'row', style: 'flex-wrap:nowrap' },
              it.cardio?.durationSec && h('button', { class: 'small', onclick: () => restTimer(it.cardio.durationSec, it.exerciseName, 'work') }, '⏱ Timer'),
              info && (info.cues?.length || info.demoUrl || info.demoVideoUrl) &&
                h('button', { class: 'small ghost', onclick: () => showExercise(info) }, 'How to'))),
          h('div', { class: 'prescription' }, prescription(it), it.cardio?.zone && zoneInfo(me.birthDate)),
          it.notes && h('div', { class: 'small' }, it.notes),
          it.cardio
            ? h('div', { class: 'sethead cardio' }, h('span', {}, it.sets > 1 ? 'Rep' : ''), h('span', {}, 'Min'), h('span', {}, distUnits(u).unit), h('span', {}, 'Avg HR'), h('span', {}, 'RPE'), h('span', {}))
            : h('div', { class: 'sethead' }, h('span', {}, 'Set'), h('span', {}, 'Reps'), h('span', {}, u.w), h('span', {}, 'RPE', rpeLogInfo()), h('span', {})),
          setsFor(it.id).map((set) => (it.cardio ? cardioRow(set, it) : setRow(set, it, b))),
          !it.cardio && videoBar(it),
          exerciseNote(it));
      }));
  }

  mount($app(),
    h('button', { class: 'ghost small', onclick: () => go('/') }, '‹ Back'),
    h('h1', {}, s.workout.name),
    h('p', { class: 'muted' }, `${fmtDate(s.date)} · ${s.programName}`, ' ', statusChip(bySched[schedId], s.date)),
    s.workout.notes && h('div', { class: 'card small' }, s.workout.notes),
    byPhase(s.workout.blocks).map(blockCard),
    ratingCard(),
    h('div', { class: 'row' },
      h('button', { class: 'grow', onclick: () => finish('skipped') }, 'Skip workout'),
      h('button', { class: 'primary grow', onclick: () => finish(null) }, 'Finish workout')),
    // MS-3: a question about a finished workout links back to it for the coach.
    bySched[schedId] && bySched[schedId].status !== 'inProgress' && h('button', {
      class: 'block', style: 'margin-top:8px',
      onclick: async () => { if (await commentOn(me.userId, bySched[schedId].id)) toast('Sent to your coach'); },
    }, '💬 Ask your coach about this workout'),
  );

  /** The client's note on one exercise, for their coach; kept with the draft. */
  function exerciseNote(it) {
    const t = h('textarea', {
      class: 'exnote', rows: 2, maxlength: 1000, value: log.exerciseNotes?.[it.id] || '',
      placeholder: 'Note for your coach: how it felt, anything bothering you…', 'aria-label': `Note on ${it.exerciseName}`,
    });
    t.addEventListener('input', () => {
      const notes = (log.exerciseNotes ||= {});
      if (t.value.trim()) notes[it.id] = t.value; else delete notes[it.id];
      saveDraft();
    });
    return t;
  }

  /** How the workout went overall, 1 (bad) to 5 (great); tap again to clear. */
  function ratingCard() {
    const row = h('div', { class: 'rating', role: 'radiogroup', 'aria-label': 'How was this workout?' });
    const draw = () => mount(row, RATINGS.map(([n, label, face]) => h('button', {
      type: 'button', role: 'radio', 'aria-checked': String(log.rating === n), class: log.rating === n ? 'on' : '',
      onclick: () => { log.rating = log.rating === n ? null : n; saveDraft(); draw(); },
    }, h('span', { class: 'face', 'aria-hidden': 'true' }, face), h('span', {}, `${n} ${label}`))));
    draw();
    return h('div', { class: 'card' }, h('h3', {}, 'How was this workout?'), row);
  }

  function finish(preset) {
    // Warm-ups and post-workout cardio don't decide the status (cardio.js).
    const allDone = log.sets.filter((x) => (x.phase || 'main') === 'main').every((x) => x.done);
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
        field('Session RPE (1-10)', rpe, null, sessionRpeInfo()),
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
            if (log.status !== 'skipped') celebrate(me.userId, log.id);
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
