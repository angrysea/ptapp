// The coach's alert inbox (LG-7): pain reported on a workout, and runs of
// missed workouts. Shown at the top of the Clients page, after the roster
// has loaded (loading it is what finds new missed runs). The threshold for
// missed runs is set on the coach's Profile page (alertSettingsCard).

import { api, get } from '../api.js';
import { busy, field, fmtDate, h, mount, select, toast } from '../ui.js';

const ICON = { pain: '⚠️', missed: '📅' };

/** The open alerts as a card, or null when there are none. */
export async function alertsCard() {
  const { alerts } = await get('/alerts').catch(() => ({ alerts: [] }));
  if (!alerts.length) return null;
  const card = h('div', { class: 'card alerts' });
  const draw = (list) => {
    if (!list.length) return card.remove();
    mount(card,
      h('h2', {}, `Alerts (${list.length})`),
      list.map((a) => {
        const open = a.kind === 'pain' && a.logId ? `#/c/${a.clientId}/logs/${a.logId}` : `#/c/${a.clientId}/calendar`;
        return h('div', { class: `alert-row ${a.kind}` },
          h('div', {},
            `${ICON[a.kind] || '•'} `, h('strong', {}, a.clientName), ` · ${a.text}`,
            h('div', { class: 'muted small' }, fmtDate(a.date))),
          h('div', { class: 'row', style: 'margin-top:6px' },
            h('a', { class: 'btn small', href: open }, a.kind === 'pain' ? 'See workout' : 'See calendar'),
            h('a', { class: 'btn small', href: `#/c/${a.clientId}/messages` }, 'Message'),
            h('button', {
              class: 'small ghost',
              onclick: (e) => busy(e.currentTarget, async () => {
                await api('POST', `/alerts/${encodeURIComponent(a.id)}/done`);
                draw(list.filter((x) => x !== a));
              }),
            }, '✓ Done')));
      }));
  };
  draw(alerts);
  return card;
}

/** For the coach's Profile page: when a missed run becomes an alert. */
export function alertSettingsCard() {
  const card = h('div', { class: 'card' }, h('h3', {}, 'Alerts'));
  get('/alerts').then(({ settings }) => {
    const n = select([1, 2, 3, 4, 5, 6, 7].map((k) => [String(k), k === 1 ? 'Any missed workout' : `${k} in a row`]), String(settings.missedInARow), {
      onchange: (e) => busy(e.target, async () => {
        await api('PUT', '/alerts/settings', { missedInARow: Number(n.value) });
        toast('Saved');
      }),
    });
    card.append(
      field('Alert me when a client misses', n),
      h('p', { class: 'muted small' }, 'Pain reported on a workout always alerts you.'));
  }).catch(() => card.append(h('p', { class: 'muted small' }, 'Couldn’t load alert settings.')));
  return card;
}
