// WB-1 exercise library: built-in exercises plus the coach's custom ones
// with cues, tags and an optional uploaded demo video.

import { del, get, post, put } from '../api.js';
import { session } from '../session.js';
import { $app, busy, confirmSheet, field, h, mount, sheet, toast } from '../ui.js';

export async function loadExercises(force = false) {
  if (!session.exercises || force) {
    const list = await get('/exercises', { cache: true });
    session.exercises = Object.fromEntries(list.map((e) => [e.id, e]));
  }
  return Object.values(session.exercises).sort((a, b) => a.name.localeCompare(b.name));
}

export async function renderExercises() {
  const list = await loadExercises(true);
  const search = h('input', { type: 'search', placeholder: 'Search by name, muscle or equipment' });
  const box = h('div');
  const draw = () => {
    const q = search.value.toLowerCase();
    mount(box, list
      .filter((e) => !q || [e.name, ...(e.muscleGroups || []), ...(e.equipment || [])].join(' ').toLowerCase().includes(q))
      .map((e) => h('div', { class: 'card link', onclick: () => editSheet(e) },
        h('div', { class: 'spread' }, h('strong', {}, e.name), e.custom ? h('span', { class: 'chip' }, 'Custom') : h('span', { class: 'chip gray' }, 'Built-in')),
        h('div', { class: 'muted small' }, [...(e.muscleGroups || []), ...(e.equipment || [])].join(' · ')))));
  };
  search.addEventListener('input', draw);
  mount($app(),
    h('div', { class: 'spread' }, h('h1', {}, 'Exercises'), h('button', { class: 'primary', onclick: () => editSheet(null) }, '+ New')),
    search, h('div', { style: 'height:12px' }), box);
  draw();
}

const csv = (a) => (a || []).join(', ');
const splitCsv = (s) => s.split(',').map((x) => x.trim()).filter(Boolean);

function editSheet(ex) {
  const readOnly = ex && !ex.custom;
  const name = h('input', { value: ex?.name || '', disabled: readOnly });
  const muscles = h('input', { value: csv(ex?.muscleGroups), placeholder: 'quads, glutes', disabled: readOnly });
  const equipment = h('input', { value: csv(ex?.equipment), placeholder: 'barbell, rack', disabled: readOnly });
  const cues = h('textarea', { value: (ex?.cues || []).join('\n'), placeholder: 'One cue per line', disabled: readOnly });
  const demoUrl = h('input', { type: 'url', value: ex?.demoUrl || '', placeholder: 'https://… (optional link)', disabled: readOnly });
  const video = h('input', { type: 'file', accept: 'video/mp4,video/quicktime,video/webm' });
  const body = () => ({ name: name.value, muscleGroups: splitCsv(muscles.value), equipment: splitCsv(equipment.value), cues: cues.value.split('\n'), demoUrl: demoUrl.value || null });
  const sh = sheet(
    h('h2', {}, ex ? ex.name : 'New exercise'),
    readOnly && h('p', { class: 'muted small' }, 'Built-in exercises are read-only. Create a custom exercise to change cues or add your own demo.'),
    ex?.demoVideoUrl && h('video', { src: ex.demoVideoUrl, controls: true, playsinline: true, style: 'width:100%;border-radius:10px' }),
    field('Name', name), field('Muscle groups', muscles), field('Equipment', equipment), field('Coaching cues', cues), field('Demo link', demoUrl),
    !readOnly && ex && field('Upload your own demo video', video, 'Short clip, ideally under 30 s.'),
    !readOnly && h('div', { class: 'row' },
      ex && h('button', {
        class: 'danger', onclick: async () => {
          if (!(await confirmSheet(`Delete ${ex.name}? Existing programs keep their copy of the name.`, 'Delete'))) return;
          await del(`/exercises/${ex.id}`); sh.close(); renderExercises();
        },
      }, 'Delete'),
      h('button', {
        class: 'primary grow',
        onclick: (e) => busy(e.currentTarget, async () => {
          const saved = ex ? await put(`/exercises/${ex.id}`, body()) : await post('/exercises', body());
          const f = video.files?.[0];
          if (f) {
            const { uploadUrl, uploadHeaders } = await post(`/exercises/${saved.id}/demo-upload`, { contentType: f.type, size: f.size });
            const res = await fetch(uploadUrl, { method: 'PUT', headers: uploadHeaders, body: f });
            if (!res.ok) throw new Error(`Video upload failed (${res.status})`);
            await post(`/exercises/${saved.id}/demo-confirm`);
          }
          toast('Saved');
          sh.close();
          renderExercises();
        }),
      }, ex ? 'Save' : 'Create')),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Close'),
  );
}
