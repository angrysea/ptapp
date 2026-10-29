// Coach program builder (WB-2, WB-3, WB-4): a multi-week calendar of
// workouts; each workout is a list of blocks (straight sets, superset,
// circuit, AMRAP, EMOM, intervals) holding exercise prescriptions. Blocks
// are warm-up, main or post-workout cardio; the program's own warm-up and
// cardio blocks go into every workout when it's assigned (cardio.js).

import { del, get, post, put } from '../api.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { rirTargetInfo, rpeTargetInfo } from '../effort.js';
import { PHASES, distUnits, phaseOf, zoneInfo, zoneOptions } from '../cardio.js';
import { $app, busy, confirmSheet, field, h, mount, num, select, sheet, toast, units } from '../ui.js';
import { loadExercises } from './exercises.js';
import { ruleEditor } from './progression.js';

const KINDS = [['straight', 'Straight sets'], ['superset', 'Superset'], ['circuit', 'Circuit'], ['amrap', 'AMRAP'], ['emom', 'EMOM'], ['interval', 'Intervals']];
const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const EXTRA_PHASES = PHASES.filter(([k]) => k !== 'main');

export async function renderPrograms() {
  const list = await get('/programs', { cache: true });
  mount($app(),
    h('div', { class: 'spread page-head' }, h('h1', {}, 'Programs'), h('button', { class: 'primary', onclick: () => go('/programs/new') }, '+ New')),
    list.length === 0 && h('div', { class: 'card muted' }, 'No programs yet. Build one and assign it to clients from their Calendar tab.'),
    list.map((p) => h('div', { class: 'card' },
      h('div', { class: 'spread link', onclick: () => go(`/programs/${p.id}`), style: 'cursor:pointer' },
        h('div', {}, h('strong', {}, p.name), ' ', p.isTemplate && h('span', { class: 'chip' }, 'Template'),
          h('div', { class: 'muted small' }, `${p.weeks} weeks · ${p.workouts.length} workouts`)),
        h('span', { class: 'muted' }, '›')),
      h('div', { class: 'row', style: 'margin-top:8px' },
        h('button', { class: 'small', onclick: (e) => busy(e.currentTarget, async () => { await post(`/programs/${p.id}/copy`, {}); toast('Copied'); renderPrograms(); }) }, 'Duplicate'),
        h('button', {
          class: 'small danger', onclick: async () => {
            if (!(await confirmSheet(`Delete “${p.name}”? Clients already assigned keep their workouts.`, 'Delete'))) return;
            await del(`/programs/${p.id}`); renderPrograms();
          },
        }, 'Delete')))),
  );
}

const blankProgram = () => ({ id: '', name: '', description: '', weeks: 4, isTemplate: true, workouts: [], defaultBlocks: [] });
const blankWorkout = (week, day) => ({ week, day, workout: { id: '', name: `Week ${week} · ${DAYS[day - 1]}`, notes: '', skipProgramDefaults: false, blocks: [] } });
const blankBlock = (kind, phase = 'main') => ({ id: '', kind, phase, rounds: null, durationSec: kind === 'amrap' || kind === 'emom' ? 600 : null, workSec: kind === 'interval' ? 40 : null, restSec: kind === 'interval' ? 20 : null, items: [], notes: '' });

export async function renderProgramEditor(id) {
  const [lib, loaded] = await Promise.all([loadExercises(), id === 'new' ? blankProgram() : get(`/programs/${id}`)]);
  const p = structuredClone(loaded);
  p.defaultBlocks ||= [];
  const u = units(session.profile);
  let sel = null; // index into p.workouts
  let dirty = false;
  const touch = () => { dirty = true; };

  const name = h('input', { value: p.name, placeholder: 'e.g. 12-week strength block', oninput: (e) => { p.name = e.target.value; touch(); } });
  const desc = h('textarea', { value: p.description, oninput: (e) => { p.description = e.target.value; touch(); } });
  const weeks = h('input', { type: 'number', min: 1, max: 52, value: p.weeks, onchange: (e) => { p.weeks = Math.max(1, Math.min(52, Number(e.target.value) || 1)); touch(); draw(); } });
  const tmpl = h('input', { type: 'checkbox', checked: p.isTemplate, onchange: (e) => { p.isTemplate = e.target.checked; touch(); } });
  const calBox = h('div');
  const editorBox = h('div');
  const defaultsBox = h('div');

  const save = (btn) => busy(btn, async () => {
    if (!p.name.trim()) throw new Error('Give the program a name');
    const outside = p.workouts.filter((w) => w.week > p.weeks);
    if (outside.length) throw new Error(`${outside.length} workout(s) are beyond week ${p.weeks}; increase weeks or delete them`);
    const saved = p.id ? await put(`/programs/${p.id}`, p) : await post('/programs', p);
    Object.assign(p, saved);
    dirty = false;
    toast('Program saved');
    if (id === 'new') go(`/programs/${saved.id}`);
    else draw();
  });

  mount($app(),
    h('div', { class: 'spread' },
      h('button', { class: 'ghost small', onclick: async () => { if (!dirty || (await confirmSheet('Discard unsaved changes?', 'Discard'))) go('/programs'); } }, '‹ Programs'),
      h('button', { class: 'primary', onclick: (e) => save(e.currentTarget) }, 'Save')),
    h('div', { class: 'card' },
      field('Program name', name),
      field('Description', desc),
      h('div', { class: 'row' }, h('div', { style: 'width:120px' }, field('Weeks', weeks)), h('label', { class: 'check' }, tmpl, 'Reusable template'))),
    h('div', { class: 'card' }, h('h3', {}, 'Calendar'), h('p', { class: 'muted small' }, 'Tap a day to add or edit its workout.'), calBox),
    editorBox,
    defaultsCard(),
  );

  /** The warm-up and post-workout cardio every workout gets on assignment. */
  function defaultsCard() {
    const blocks = p.defaultBlocks;
    const drawDefaults = () => mount(defaultsBox, blocks.map((b, bi) => blockEditor(blocks, b, bi, drawDefaults, EXTRA_PHASES)));
    drawDefaults();
    return h('div', { class: 'card' },
      h('h3', {}, 'Warm-up and cardio for every workout'),
      h('p', { class: 'muted small' }, 'Added to each workout when you assign the program. A workout with its own warm-up or cardio keeps its own, and a workout can leave these out. Changes apply to future assignments.'),
      defaultsBox,
      addBlockRow(blocks, drawDefaults, EXTRA_PHASES));
  }

  /** The block type picker and one add button per part of the workout. */
  function addBlockRow(blocks, redraw, phases) {
    const kind = select(KINDS, 'straight', { 'aria-label': 'Block type' });
    const ADD = { main: '+ Add block', warmup: '+ Warm-up', post: '+ Post-workout cardio' };
    return h('div', { class: 'row' }, h('div', { class: 'grow' }, kind),
      phases.map(([phase]) => h('button', { onclick: () => { blocks.push(blankBlock(kind.value, phase)); touch(); redraw(); } }, ADD[phase])));
  }

  function draw() {
    const cal = h('div', { class: 'cal' }, h('div', {}), DAYS.map((d) => h('div', { class: 'hd' }, d)));
    for (let w = 1; w <= p.weeks; w++) {
      cal.append(h('div', { class: 'wk' }, `Wk ${w}`,
        h('button', { class: 'small ghost', title: 'Copy week', onclick: () => copyWeek(w) }, '⧉')));
      for (let d = 1; d <= 7; d++) {
        const idxs = p.workouts.map((x, i) => [x, i]).filter(([x]) => x.week === w && x.day === d);
        cal.append(h('div', {
          class: `day ${idxs.length ? 'has' : ''} ${idxs.some(([, i]) => i === sel) ? 'sel' : ''}`,
          onclick: () => pickDay(w, d, idxs),
        }, idxs.length ? idxs.map(([x]) => h('span', { class: 'w' }, x.workout.name)) : h('span', { class: 'muted' }, '+')));
      }
    }
    mount(calBox, cal);
    mount(editorBox, sel != null && p.workouts[sel] ? workoutEditor(p.workouts[sel]) : null);
  }

  function pickDay(w, d, idxs) {
    if (idxs.length === 0) {
      p.workouts.push(blankWorkout(w, d));
      sel = p.workouts.length - 1;
      touch();
    } else if (idxs.length === 1 || !idxs.some(([, i]) => i === sel)) {
      sel = idxs[0][1];
    } else {
      // cycle through multiple workouts on one day
      const pos = idxs.findIndex(([, i]) => i === sel);
      sel = idxs[(pos + 1) % idxs.length][1];
    }
    draw();
    editorBox.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  function copyWeek(w) {
    const target = h('input', { type: 'number', min: 1, max: 52, value: Math.min(w + 1, 52) });
    const sh = sheet(
      h('h2', {}, `Copy week ${w}`),
      field('To week', target),
      h('button', {
        class: 'primary block',
        onclick: () => {
          const t = Number(target.value);
          if (!t || t < 1 || t > 52) return;
          for (const x of p.workouts.filter((x) => x.week === w)) {
            const c = structuredClone(x);
            c.week = t;
            c.workout.id = '';
            c.workout.name = c.workout.name.replace(/^Week \d+/, `Week ${t}`);
            c.workout.blocks.forEach((b) => { b.id = ''; b.items.forEach((i) => { i.id = ''; }); });
            p.workouts.push(c);
          }
          if (t > p.weeks) { p.weeks = t; weeks.value = t; }
          touch(); sh.close(); draw();
        },
      }, 'Copy'),
    );
  }

  function workoutEditor(pw) {
    const wname = h('input', { value: pw.workout.name, oninput: (e) => { pw.workout.name = e.target.value; touch(); } });
    wname.addEventListener('change', draw);
    const wnotes = h('textarea', { value: pw.workout.notes, placeholder: 'Focus for the day…', oninput: (e) => { pw.workout.notes = e.target.value; touch(); } });
    const blocksBox = h('div');
    const drawBlocks = () => mount(blocksBox, pw.workout.blocks.map((b, bi) => blockEditor(pw.workout.blocks, b, bi, drawBlocks)));
    drawBlocks();
    const skip = h('input', { type: 'checkbox', checked: !!pw.workout.skipProgramDefaults, onchange: (e) => { pw.workout.skipProgramDefaults = e.target.checked; touch(); } });
    const moveW = h('input', { type: 'number', min: 1, max: p.weeks, value: pw.week, style: 'width:80px' });
    const moveD = select(DAYS.map((d, i) => [String(i + 1), d]), String(pw.day));
    return h('div', { class: 'card today' },
      h('div', { class: 'spread' }, h('h2', {}, 'Workout'), h('button', { class: 'ghost small', onclick: () => { sel = null; draw(); } }, 'Close')),
      field('Name', wname),
      field('Notes', wnotes),
      blocksBox,
      addBlockRow(pw.workout.blocks, drawBlocks, PHASES),
      p.defaultBlocks.length > 0 && h('label', { class: 'check' }, skip, 'Leave out the program’s warm-up and cardio'),
      h('hr'),
      h('div', { class: 'row small' }, 'Week', moveW, 'Day', h('div', { style: 'width:100px' }, moveD),
        h('button', { class: 'small', onclick: () => { pw.week = Number(moveW.value); pw.day = Number(moveD.value); touch(); draw(); } }, 'Move'),
        h('button', {
          class: 'small', onclick: () => {
            const c = structuredClone(pw);
            c.week = Number(moveW.value); c.day = Number(moveD.value); c.workout.id = '';
            c.workout.blocks.forEach((b) => { b.id = ''; b.items.forEach((i) => { i.id = ''; }); });
            p.workouts.push(c); sel = p.workouts.length - 1; touch(); draw();
          },
        }, 'Copy to'),
        h('button', { class: 'small danger', onclick: () => { p.workouts.splice(p.workouts.indexOf(pw), 1); sel = null; touch(); draw(); } }, 'Delete workout')),
    );
  }

  function blockEditor(blocks, b, bi, redraw, phases = PHASES) {
    const dist = distUnits(u);
    const numIn = (obj, key, attrs = {}) => h('input', {
      type: 'number', inputmode: 'decimal', value: obj[key] ?? '', ...attrs,
      onchange: (e) => { obj[key] = num(e.target.value) ?? (key === 'sets' ? 1 : null); touch(); },
    });
    const minutes = h('input', { type: 'number', inputmode: 'decimal', value: b.durationSec ? b.durationSec / 60 : '', onchange: (e) => { b.durationSec = num(e.target.value) ? Math.round(num(e.target.value) * 60) : null; touch(); } });
    const kindSel = select(KINDS, b.kind, { 'aria-label': 'Block type', onchange: (e) => { b.kind = e.target.value; touch(); redraw(); } });
    // Buttons rather than a select, so a block's selects stay type, then exercise.
    const phaseBtns = h('div', { class: 'row phase-pick', role: 'group', 'aria-label': 'Part of the workout' },
      phases.map(([k, label]) => h('button', {
        type: 'button', class: `small ${phaseOf(b) === k ? 'primary' : 'ghost'}`, 'aria-pressed': String(phaseOf(b) === k),
        onclick: () => { b.phase = k; touch(); redraw(); },
      }, label)));
    const exSel = select([['', 'Add exercise…'], ...lib.map((e) => [e.id, e.name])], '');
    exSel.addEventListener('change', () => {
      const ex = lib.find((e) => e.id === exSel.value);
      if (!ex) return;
      const last = b.items[b.items.length - 1];
      const cardio = ex.modality === 'cardio';
      b.items.push({ id: '', exerciseId: ex.id, exerciseName: ex.name, sets: cardio ? 1 : last?.sets ?? 3, reps: cardio ? '1' : last?.reps ?? '10', loadKg: null, tempo: null,
        restSec: !cardio && b.kind === 'straight' ? 90 : null, rpe: null, rir: null, cardio: cardio ? { durationSec: (phaseOf(b) === 'warmup' ? 5 : 20) * 60 } : null, notes: '' });
      touch(); redraw();
    });
    const move = (arr, i, d) => { const j = i + d; if (j < 0 || j >= arr.length) return; [arr[i], arr[j]] = [arr[j], arr[i]]; touch(); redraw(); };
    return h('div', { class: `block-card phase-${phaseOf(b)}` },
      phaseBtns,
      h('div', { class: 'spread' },
        h('div', { style: 'flex:1' }, kindSel),
        h('div', { class: 'row', style: 'flex-wrap:nowrap' },
          h('button', { class: 'small ghost', onclick: () => move(blocks, bi, -1) }, '↑'),
          h('button', { class: 'small ghost', onclick: () => move(blocks, bi, 1) }, '↓'),
          h('button', { class: 'small ghost', onclick: () => { blocks.splice(bi, 1); touch(); redraw(); } }, '✕'))),
      h('div', { class: 'item-grid', style: 'margin-top:6px' },
        ['superset', 'circuit', 'interval'].includes(b.kind) && field('Rounds', numIn(b, 'rounds')),
        ['amrap', 'emom'].includes(b.kind) && field('Minutes', minutes),
        ['interval', 'circuit'].includes(b.kind) && field('Work (s)', numIn(b, 'workSec')),
        ['interval', 'circuit', 'superset'].includes(b.kind) && field('Rest (s)', numIn(b, 'restSec'))),
      b.items.map((it, ii) => h('div', { class: 'item-card' },
        h('div', { class: 'spread' }, h('strong', {}, it.exerciseName),
          h('div', { class: 'row', style: 'flex-wrap:nowrap' },
            h('button', { class: 'small ghost', onclick: () => move(b.items, ii, -1) }, '↑'),
            h('button', { class: 'small ghost', onclick: () => move(b.items, ii, 1) }, '↓'),
            h('button', { class: 'small ghost', onclick: () => { b.items.splice(ii, 1); touch(); redraw(); } }, '✕'))),
        it.cardio ? h('div', { class: 'item-grid' },
          field('Repeats', numIn(it, 'sets', { min: 1 })),
          field('Minutes', h('input', {
            type: 'number', inputmode: 'decimal', step: 'any', min: 0, value: it.cardio.durationSec ? it.cardio.durationSec / 60 : '',
            onchange: (e) => { it.cardio.durationSec = num(e.target.value) ? Math.round(num(e.target.value) * 60) : null; touch(); },
          })),
          field(`Distance (${dist.unit})`, h('input', {
            type: 'number', inputmode: 'decimal', step: 'any', min: 0, value: dist.to(it.cardio.distanceM) ?? '',
            onchange: (e) => { it.cardio.distanceM = dist.from(e.target.value) || null; touch(); },
          })),
          field('Zone', select(zoneOptions, it.cardio.zone ? String(it.cardio.zone) : '', { onchange: (e) => { it.cardio.zone = num(e.target.value); touch(); } }), null, zoneInfo()),
          field('RPE', numIn(it, 'rpe', { step: '0.5', min: 1, max: 10 }), null, rpeTargetInfo()),
          field('Rest (s)', numIn(it, 'restSec')))
          : h('div', { class: 'item-grid' },
            field('Sets', numIn(it, 'sets', { min: 1 })),
            field('Reps', h('input', { value: it.reps, placeholder: '8-10', onchange: (e) => { it.reps = e.target.value || '1'; touch(); } })),
            field('Load', loadInput(it, u, touch)),
            field('Tempo', h('input', { value: it.tempo || '', placeholder: '3-1-1-0', onchange: (e) => { it.tempo = e.target.value || null; touch(); } })),
            field('Rest (s)', numIn(it, 'restSec')),
            field('RPE', numIn(it, 'rpe', { step: '0.5', min: 1, max: 10 }), null, rpeTargetInfo()),
            field('RIR', numIn(it, 'rir', { min: 0, max: 10 }), null, rirTargetInfo())),
        h('input', { value: it.notes, placeholder: 'Notes / cues for this exercise', onchange: (e) => { it.notes = e.target.value; touch(); } }),
        h('button', {
          class: 'small ghost', type: 'button',
          onclick: () => { it.cardio = it.cardio ? null : { durationSec: 600 }; if (it.cardio) it.loadPct = it.loadKg = it.progression = null; touch(); redraw(); },
        }, it.cardio ? 'Do as sets × reps instead' : 'Do for time or distance instead'),
        !it.cardio && ruleEditor(it, u, touch))),
      h('div', { style: 'margin-top:8px' }, exSel),
    );
  }

  draw();
}

/** A load as a weight or as a % of the client's 1RM (WB-5), which assigning
 * turns into a weight from their maxes (Progress → Maxes). */
function loadInput(it, u, touch) {
  const asPct = () => mode.value === 'pct';
  const value = h('input', {
    type: 'number', inputmode: 'decimal', step: 'any', min: 0, 'aria-label': 'Load',
    value: it.loadPct != null ? it.loadPct : it.loadKg != null ? u.toW(it.loadKg) : '',
    onchange: () => store(),
  });
  const mode = select([['w', u.w], ['pct', '% 1RM']], it.loadPct != null ? 'pct' : 'w', { 'aria-label': 'Load unit', onchange: () => store() });
  function store() {
    const v = value.value === '' ? null : Number(value.value);
    it.loadPct = asPct() ? v : null;
    it.loadKg = asPct() ? null : u.fromW(value.value);
    touch();
  }
  return h('div', { class: 'load-input' }, value, mode);
}
