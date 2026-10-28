// A client's 1RMs (WB-5, ON-4 test lifts). Programs can prescribe loads as
// a % of these; saving one re-weighs the client's upcoming workouts. Shown
// as the Maxes tab of the Progress view (views/metrics.js) for the client
// and their coach.

import { api, get } from '../api.js';
import { session } from '../session.js';
import { busy, confirmSheet, field, fmtDate, h, mount, num, select, toast, todayIso, units } from '../ui.js';
import { loadExercises } from './exercises.js';

const updated = (n) => (n ? ` ${n} upcoming workout${n === 1 ? '' : 's'} updated.` : '');

export async function maxes(el, client) {
  const u = units(session.profile);
  const base = `/clients/${client.userId}/maxes`;
  const [{ maxes: list, estimates }, library] = await Promise.all([get(base), loadExercises()]);
  const recorded = new Set(list.map((m) => m.exerciseId));
  const redraw = () => maxes(el, client);

  const save = async (exerciseId, exerciseName, kg, source, date) => {
    const r = await api('PUT', `${base}/${encodeURIComponent(exerciseId)}`, { exerciseName, kg, source, date });
    toast(`Saved.${updated(r.updatedWorkouts)}`);
    redraw();
  };

  const exercise = select(library.map((e) => [e.id, e.name]), list[0]?.exerciseId ?? library.find((e) => /squat/i.test(e.name))?.id);
  const weight = h('input', { type: 'number', inputmode: 'decimal', step: 'any', min: 0 });
  const source = select([['tested', 'Tested (lifted for 1 rep)'], ['estimated', 'Estimated']], 'tested');
  const date = h('input', { type: 'date', value: todayIso() });

  mount(el,
    h('div', { class: 'card' },
      h('h3', {}, 'One-rep maxes'),
      h('p', { class: 'muted small' }, 'Programs can set loads as a % of these. Changing one updates the weights in upcoming workouts.'),
      list.length === 0 && h('p', { class: 'muted' }, 'None recorded yet.'),
      list.map((m) => h('div', { class: 'spread max-row' },
        h('div', {},
          h('strong', {}, m.exerciseName), ' ',
          h('span', {}, `${u.toW(m.kg)} ${u.w}`),
          h('div', { class: 'muted small' }, `${m.source === 'tested' ? 'Tested' : 'Estimated'} ${fmtDate(m.date)} · set by ${m.setBy === 'coach' ? 'coach' : 'client'}`)),
        h('button', {
          class: 'small ghost', 'aria-label': `Delete ${m.exerciseName} max`,
          onclick: async () => {
            if (!(await confirmSheet(`Delete the ${m.exerciseName} max? Upcoming % loads for it go back to "pick a weight".`, 'Delete'))) return;
            const r = await api('DELETE', `${base}/${encodeURIComponent(m.exerciseId)}`);
            toast(`Deleted.${updated(r.updatedWorkouts)}`);
            redraw();
          },
        }, '✕')))),

    h('div', { class: 'card' },
      h('h3', {}, 'Record a max'),
      field('Exercise', exercise),
      h('div', { class: 'grid2' }, field(`1RM (${u.w})`, weight), field('Date', date)),
      field('How it was found', source),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          const kg = u.fromW(weight.value);
          if (!num(weight.value) || kg <= 0) throw new Error('Enter the weight');
          const ex = library.find((x) => x.id === exercise.value);
          await save(ex.id, ex.name, kg, source.value, date.value);
        }),
      }, 'Save max')),

    estimates.length > 0 && h('div', { class: 'card' },
      h('h3', {}, 'From logged sets'),
      h('p', { class: 'muted small' }, 'Best estimated 1RM per exercise in the last six months, from weight × reps.'),
      estimates.map((e) => h('div', { class: 'spread max-row' },
        h('div', {}, h('strong', {}, e.exerciseName), ' ', `${u.toW(e.kg)} ${u.w}`, h('div', { class: 'muted small' }, fmtDate(e.date))),
        h('button', {
          class: 'small',
          onclick: (ev) => busy(ev.currentTarget, () => save(e.exerciseId, e.exerciseName, e.kg, 'estimated', e.date)),
        }, recorded.has(e.exerciseId) ? 'Use instead' : 'Use')))),
  );
}
