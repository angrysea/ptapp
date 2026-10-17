import { put } from '../api.js';
import { logout } from '../auth.js';
import { kv, outbox } from '../idb.js';
import { session } from '../session.js';
import { $app, busy, field, h, mount, select, toast } from '../ui.js';

export async function renderProfile() {
  const p = session.profile;
  const name = h('input', { value: p.name });
  const unitsSel = select([['metric', 'Metric (kg, cm)'], ['imperial', 'Imperial (lb, in)']], p.units);
  const business = p.role === 'coach' && h('input', { value: p.businessName || '' });
  const pending = (await outbox.all()).length;
  mount($app(),
    h('h1', {}, 'Profile'),
    h('div', { class: 'card' },
      h('p', { class: 'muted small' }, `${p.email} · ${p.role}`),
      field('Name', name),
      business && field('Business name', business),
      field('Units', unitsSel),
      h('button', {
        class: 'primary block',
        onclick: (e) => busy(e.currentTarget, async () => {
          session.profile = await put('/me', { name: name.value, units: unitsSel.value, businessName: business ? business.value : undefined });
          toast('Saved');
        }),
      }, 'Save')),
    p.role === 'client' && h('div', { class: 'card' },
      h('a', { class: 'btn block', href: '#/intake' }, 'Intake questionnaire'),
      h('div', { style: 'height:8px' }),
      h('a', { class: 'btn block', href: '#/waiver' }, 'Health screening & waiver')),
    h('div', { class: 'card' },
      h('p', { class: 'small muted' }, pending ? `${pending} change(s) waiting to sync.` : 'Everything is synced.'),
      h('button', {
        class: 'block danger',
        onclick: async () => {
          if (pending && !confirm('Some changes have not synced yet and will be lost. Sign out anyway?')) return;
          await kv.clear(); await outbox.clear();
          logout();
        },
      }, 'Sign out')),
  );
}
