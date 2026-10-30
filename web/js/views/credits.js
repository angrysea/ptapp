// Service credits (routes/credits.rs, ptcore::credits). Coaches build plans
// (so many credits per service, or unlimited, for so many days) on Profile,
// grant them on a client's Overview, and turn on "Require credits" when
// ready. Clients see what they have on Profile, and the app stops them
// before a message, photo, video or booking they have no credit for (the
// API refuses it too, with a 402).

import { api, get, post, put } from '../api.js';
import { session } from '../session.js';
import { addDays, busy, confirmSheet, field, fmtDate, h, mount, num, select, sheet, toast } from '../ui.js';

export const SERVICES = [
  ['message', 'Messages', 'Each message the client sends you (text or voice note).'],
  ['photoReview', 'Photo reviews', 'Each photo the client sends you in Messages.'],
  ['videoReview', 'Video reviews', 'Each set video you review. An upload holds a credit until you review it.'],
  ['session', 'Sessions', 'Each session booked with you; given back if it’s cancelled.'],
  ['programming', 'Programs', 'Each program assigned to the client. Counted only: assigning is never stopped.'],
];
const label = (s) => SERVICES.find(([k]) => k === s)?.[1] ?? s;

/** "3 left", "Unlimited", "None". */
const amount = (b) => (b.remaining == null ? 'Unlimited' : b.remaining === 0 ? 'None left' : `${b.remaining} left`);

export const loadCredits = (cid) => get(`/clients/${cid}/credits`, { cache: true });

/** Whether the client may use `service` now (always, while their coach doesn't require credits). */
export function canUse(c, service) {
  if (!c?.enforced || service === 'programming') return true;
  return !!c.balances?.find((b) => b.service === service)?.available;
}

/** Checks before a client starts something that uses a credit; says why not. */
export async function guard(cid, service) {
  if (session.isCoach) return true;
  let c;
  try { c = await loadCredits(cid); } catch { return true; } // offline: the server decides when it syncs
  if (canUse(c, service)) return true;
  const b = c.balances.find((x) => x.service === service);
  toast(b?.held && b.remaining
    ? `Your ${label(service).toLowerCase()} are all in use by videos waiting for review. Ask your coach about your plan.`
    : `You have no ${label(service).toLowerCase()} left. Ask your coach about your plan.`, 'error');
  return false;
}

function balanceRows(c) {
  return c.balances.filter((b) => b.granted || b.remaining !== 0).map((b) => h('div', { class: 'spread credit-row' },
    h('div', {}, h('strong', {}, label(b.service)),
      h('div', { class: 'muted small' }, [
        b.held ? `${b.held} held by videos awaiting review` : '',
        b.nextExpiry ? `expires ${fmtDate(b.nextExpiry)}` : '',
      ].filter(Boolean).join(' · '))),
    h('span', { class: `chip ${b.available || b.service === 'programming' ? 'good' : 'bad'}` }, amount(b))));
}

// ---------------------------------------------------------------- client: Profile

/** The client's own credits, on their Profile. */
export function myCreditsCard(me) {
  const card = h('div', { class: 'card' }, h('h3', {}, 'Your plan'));
  loadCredits(me.userId).then((c) => {
    const any = c.balances.some((b) => b.granted);
    mount(card, h('h3', {}, 'Your plan'),
      !any ? h('p', { class: 'muted small' }, c.enforced ? 'You have no credits yet. Ask your coach about a plan.' : 'Your coach hasn’t set up a plan for you.')
        : [balanceRows(c), h('p', { class: 'muted small' }, 'Each message, photo, video review and session uses one credit.')]);
  }).catch(() => mount(card, h('h3', {}, 'Your plan'), h('p', { class: 'muted small' }, 'Needs a connection.')));
  return card;
}

// ---------------------------------------------------------------- coach: a client's credits

/** For the coach's Overview of a client: balances, grants, history, and grant/add buttons. */
export async function clientCreditsCard(client) {
  const card = h('div', { class: 'card' });
  const draw = async () => {
    const [c, plans] = await Promise.all([get(`/clients/${client.userId}/credits`), get('/plans').catch(() => [])]);
    mount(card,
      h('div', { class: 'spread' }, h('h3', {}, 'Credits'),
        h('div', { class: 'row' },
          h('button', { class: 'small', onclick: () => grantPlanSheet(client, plans, c.today, draw) }, 'Grant a plan'),
          h('button', { class: 'small ghost', onclick: () => addCreditsSheet(client, c.today, draw) }, 'Add credits'))),
      !c.enforced && h('p', { class: 'small muted' }, 'You don’t require credits yet, so nothing is stopped; uses are still counted. Turn it on in Profile › Plans & credits.'),
      c.grants.length === 0 ? h('p', { class: 'muted small' }, 'No credits granted yet.') : balanceRows(c),
      c.grants.length > 0 && h('details', {},
        h('summary', { class: 'small' }, 'Grants'),
        c.grants.map((g) => h('div', { class: 'spread small credit-row' },
          h('div', {}, `${label(g.service)}: ${g.total == null ? 'unlimited' : `${g.used} of ${g.total} used`}`,
            h('div', { class: 'muted' }, [g.planName || g.note || 'Added by hand', g.expiresOn ? `${g.expiresOn < c.today ? 'expired' : 'expires'} ${fmtDate(g.expiresOn)}` : 'no expiry'].join(' · '))),
          h('div', { class: 'row', style: 'flex-wrap:nowrap' },
            h('button', { class: 'small ghost', 'aria-label': `Change when ${label(g.service)} expire`, onclick: () => expirySheet(client, g, draw) }, 'Expiry'),
            h('button', {
              class: 'small ghost', 'aria-label': `Remove ${label(g.service)} grant`,
              onclick: async () => {
                if (!(await confirmSheet(`Remove these ${label(g.service).toLowerCase()}? Any left are taken away.`, 'Remove'))) return;
                await api('DELETE', `/clients/${client.userId}/credits/${g.id}`);
                draw();
              },
            }, '✕'))))),
      c.history.length > 0 && h('details', {},
        h('summary', { class: 'small' }, 'History'),
        c.history.map((e) => h('div', { class: 'small spread' },
          h('span', {}, `${fmtDate(e.at.slice(0, 10))} · ${e.reason}`),
          h('span', { class: 'muted' }, e.unlimited ? `${label(e.service)}: unlimited` : `${e.change > 0 ? '+' : ''}${e.change} ${label(e.service).toLowerCase()}`)))));
  };
  draw().catch((e) => mount(card, h('p', { class: 'muted small' }, e.message)));
  return card;
}

const expiryInput = (value) => h('input', { type: 'date', value: value ?? '' });

function grantPlanSheet(client, plans, today, onDone) {
  if (!plans.length) { toast('Create a plan first in Profile › Plans & credits'); return; }
  const pick = select(plans.map((p) => [p.id, p.name]), plans[0].id);
  const expires = expiryInput('');
  const summary = h('p', { class: 'small muted' });
  const show = () => {
    const p = plans.find((x) => x.id === pick.value);
    expires.value = p.validDays ? addDays(today, p.validDays - 1) : '';
    mount(summary, planSummary(p));
  };
  pick.addEventListener('change', show);
  show();
  const sh = sheet(
    h('h2', {}, `Grant a plan to ${client.name}`),
    field('Plan', pick), summary,
    field('Credits expire after', expires, 'Leave empty for no expiry.'),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await post(`/clients/${client.userId}/credits/plan`, { planId: pick.value, expiresOn: expires.value || null });
        toast('Plan granted');
        sh.close();
        onDone();
      }),
    }, 'Grant'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

function addCreditsSheet(client, today, onDone) {
  const service = select(SERVICES.map(([k, l]) => [k, l]), 'message');
  const qty = h('input', { type: 'number', inputmode: 'numeric', min: 1, value: 1 });
  const unlimited = h('input', { type: 'checkbox' });
  unlimited.addEventListener('change', () => { qty.disabled = unlimited.checked; });
  const expires = expiryInput(addDays(today, 29));
  const note = h('input', { maxlength: 200, placeholder: 'e.g. Bonus review' });
  const sh = sheet(
    h('h2', {}, `Add credits for ${client.name}`),
    field('Service', service),
    h('div', { class: 'row' }, h('div', { style: 'width:120px' }, field('Credits', qty)), h('label', { class: 'check' }, unlimited, 'Unlimited')),
    field('Expire after', expires, 'Leave empty for no expiry.'),
    field('Note', note),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const n = num(qty.value);
        if (!unlimited.checked && !(n >= 1)) throw new Error('Enter how many credits, or tick Unlimited');
        await post(`/clients/${client.userId}/credits`, { service: service.value, quantity: unlimited.checked ? null : Math.round(n), expiresOn: expires.value || null, note: note.value || null });
        toast('Credits added');
        sh.close();
        onDone();
      }),
    }, 'Add'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

function expirySheet(client, g, onDone) {
  const expires = expiryInput(g.expiresOn);
  const sh = sheet(
    h('h2', {}, `${label(g.service)}: expiry`),
    field('Credits expire after', expires, 'Leave empty for no expiry.'),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        await put(`/clients/${client.userId}/credits/${g.id}`, { expiresOn: expires.value || null });
        toast('Saved');
        sh.close();
        onDone();
      }),
    }, 'Save'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

// ---------------------------------------------------------------- coach: plans (Profile)

const CURRENCIES = ['usd', 'cad', 'eur', 'gbp', 'aud'];
/** "$49.00", "Free". */
export const fmtPrice = (p) => (!p.priceCents ? 'Free'
  : new Intl.NumberFormat(undefined, { style: 'currency', currency: (p.currency || 'usd').toUpperCase() }).format(p.priceCents / 100));

const planSummary = (p) => [
  fmtPrice(p),
  p.lines.map((l) => `${l.quantity == null ? 'Unlimited' : l.quantity} ${label(l.service).toLowerCase()}`).join(', '),
  p.validDays ? `for ${p.validDays} days` : 'no expiry',
].join(' · ');

/** Profile card: require credits, and the coach's plans. */
export function plansCard() {
  const card = h('div', { class: 'card' });
  const draw = async () => {
    const [settings, plans] = await Promise.all([get('/credits/settings'), get('/plans')]);
    const enforce = h('input', { type: 'checkbox', checked: settings.enforce, 'aria-label': 'Require credits' });
    enforce.addEventListener('change', async () => {
      if (enforce.checked && !(await confirmSheet('Require credits? Clients without credits for a service can’t use it (messages, photos, video reviews, bookings) until you grant some.', 'Require credits'))) {
        enforce.checked = false;
        return;
      }
      await put('/credits/settings', { enforce: enforce.checked });
      toast(enforce.checked ? 'Credits are now required' : 'Credits are no longer required');
    });
    mount(card,
      h('h3', {}, 'Plans & credits'),
      h('label', { class: 'check' }, enforce, 'Require credits'),
      h('p', { class: 'muted small' }, 'A plan is a bundle of credits for your services. Grant one from a client’s Overview; each message, photo, video review, session and program uses one credit.'),
      plans.map((p) => h('div', { class: 'spread credit-row' },
        h('div', {}, h('strong', {}, p.name), h('div', { class: 'small' }, p.description), h('div', { class: 'muted small' }, planSummary(p))),
        h('div', { class: 'row', style: 'flex-wrap:nowrap' },
          h('button', { class: 'small ghost', onclick: () => planSheet(p, draw) }, 'Edit'),
          h('button', {
            class: 'small ghost', 'aria-label': `Delete ${p.name}`,
            onclick: async () => {
              if (!(await confirmSheet(`Delete the “${p.name}” plan? Clients keep the credits they were granted.`, 'Delete'))) return;
              await api('DELETE', `/plans/${p.id}`);
              draw();
            },
          }, '✕')))),
      h('button', { class: 'block', onclick: () => planSheet(null, draw) }, '+ New plan'));
  };
  draw().catch((e) => mount(card, h('h3', {}, 'Plans & credits'), h('p', { class: 'muted small' }, e.message)));
  return card;
}

/** Create or edit a plan: per service, included or not, a number or unlimited. */
function planSheet(plan, onDone) {
  const name = h('input', { value: plan?.name ?? '', maxlength: 80, placeholder: 'e.g. Monthly coaching' });
  const desc = h('textarea', { maxlength: 500, value: plan?.description ?? '', placeholder: 'What the client gets, e.g. a month of programming, weekly video feedback and messaging' });
  const price = h('input', { type: 'number', inputmode: 'decimal', min: 0, step: '0.01', value: plan ? (plan.priceCents / 100).toFixed(2) : '', placeholder: '0.00' });
  const currency = select(CURRENCIES.map((c) => [c, c.toUpperCase()]), plan?.currency || 'usd', { 'aria-label': 'Currency' });
  const days = h('input', { type: 'number', inputmode: 'numeric', min: 1, value: plan ? plan.validDays ?? '' : 30, placeholder: 'no expiry' });
  const rows = SERVICES.map(([k, l, help]) => {
    const line = plan?.lines.find((x) => x.service === k);
    const on = h('input', { type: 'checkbox', checked: !!line, 'aria-label': `Include ${l}` });
    const qty = h('input', { type: 'number', inputmode: 'numeric', min: 1, value: line?.quantity ?? (line ? '' : 4), 'aria-label': `${l} credits`, style: 'width:80px' });
    const unl = h('input', { type: 'checkbox', checked: !!line && line.quantity == null, 'aria-label': `Unlimited ${l}` });
    const sync = () => { qty.disabled = !on.checked || unl.checked; unl.disabled = !on.checked; };
    on.addEventListener('change', sync);
    unl.addEventListener('change', sync);
    sync();
    return {
      el: h('div', { class: 'field' },
        h('div', { class: 'row' }, h('label', { class: 'check grow' }, on, h('strong', {}, l)), qty, h('label', { class: 'check' }, unl, 'Unlimited')),
        h('div', { class: 'muted small' }, help)),
      get: () => (on.checked ? { service: k, quantity: unl.checked ? null : Math.round(num(qty.value) ?? 0) } : null),
    };
  });
  const sh = sheet(
    h('h2', {}, plan ? 'Edit plan' : 'New plan'),
    field('Name', name),
    field('Description', desc),
    h('div', { class: 'row' }, h('div', { class: 'grow' }, field('Price', price, 'What the client pays for the plan; 0 for free.')), h('div', { style: 'width:110px' }, field('Currency', currency))),
    rows.map((r) => r.el),
    field('Credits last (days)', days, 'Leave empty for no expiry. You can change the date when granting.'),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        if (!name.value.trim()) throw new Error('Give the plan a name');
        if (!desc.value.trim()) throw new Error('Describe what the plan includes');
        const cost = num(price.value);
        if (cost == null || cost < 0) throw new Error('Enter the plan’s price (0 if it’s free)');
        const body = { name: name.value, description: desc.value, priceCents: Math.round(cost * 100), currency: currency.value, lines: rows.map((r) => r.get()).filter(Boolean), validDays: num(days.value) ? Math.round(num(days.value)) : null };
        if (plan) await put(`/plans/${plan.id}`, body); else await post('/plans', body);
        toast('Plan saved');
        sh.close();
        onDone();
      }),
    }, 'Save plan'),
    h('button', { class: 'ghost block', onclick: () => sh.close() }, 'Cancel'));
}

