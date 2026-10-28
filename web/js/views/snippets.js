// Saved reply snippets (FB-4). `snippetTools(textarea, clientId)` gives a
// coach's message box a 💬 button (pick, add, edit, delete snippets) and a
// `/` shortcut: typing "/" then part of a snippet's name lists matches under
// the box; Enter or a tap inserts one. `{name}` becomes the client's first name.

import { api, get } from '../api.js';
import { busy, confirmSheet, field, h, mount, sheet, toast } from '../ui.js';

let cache = null; // the coach's snippets, loaded once per page load

async function load(force = false) {
  if (!cache || force) cache = await get('/snippets', { cache: true });
  return cache;
}

async function firstName(clientId) {
  try {
    const roster = await get('/clients', { cache: true });
    const name = roster.find((r) => r.client.clientId === clientId)?.client.name || '';
    return name.trim().split(/\s+/)[0] || '';
  } catch { return ''; }
}

/** Puts `text` into the box, replacing the characters from `from` to the caret. */
function insert(box, text, from = box.selectionStart) {
  const to = box.selectionEnd;
  box.setRangeText(text, from, to, 'end');
  box.dispatchEvent(new Event('input', { bubbles: true }));
  box.focus();
}

const matching = (list, q) => {
  const s = q.toLowerCase();
  return list.filter((x) => x.title.toLowerCase().includes(s) || (s.length > 2 && x.text.toLowerCase().includes(s)));
};

/** The 💬 button and `/` shortcut for one message box. Returns `field`, the
 * box wrapped with its `/` list (put it where the box would go), and `button`. */
export function snippetTools(box, clientId) {
  const fill = async (sn) => sn.text.replaceAll('{name}', (await firstName(clientId)) || 'there');

  // ---- the "/" shortcut
  const pop = h('div', { class: 'snippet-pop', role: 'listbox', hidden: true });
  let hits = [], at = 0, slash = -1;
  const close = () => { pop.hidden = true; slash = -1; };
  const pick = async (sn) => { const from = slash; close(); insert(box, await fill(sn), from); };
  const drawPop = () => {
    mount(pop, hits.slice(0, 6).map((sn, i) => h('div', {
      class: `snippet-opt ${i === at ? 'on' : ''}`, role: 'option', 'aria-selected': String(i === at),
      onmousedown: (e) => { e.preventDefault(); pick(sn); },
    }, h('strong', {}, sn.title), h('span', { class: 'muted' }, ` ${sn.text.slice(0, 60)}`))));
    pop.hidden = hits.length === 0;
  };
  box.addEventListener('input', async () => {
    const before = box.value.slice(0, box.selectionStart);
    const m = before.match(/(^|\s)\/([^\s/]*)$/);
    if (!m) return close();
    slash = before.length - m[2].length - 1;
    hits = matching(await load().catch(() => []), m[2]);
    at = 0;
    drawPop();
  });
  box.addEventListener('keydown', (e) => {
    if (pop.hidden) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      at = (at + (e.key === 'ArrowDown' ? 1 : -1) + Math.min(hits.length, 6)) % Math.min(hits.length, 6);
      drawPop();
    } else if (e.key === 'Enter' || e.key === 'Tab') {
      e.preventDefault();
      pick(hits[at]);
    } else if (e.key === 'Escape') {
      close();
    }
  });
  box.addEventListener('blur', () => setTimeout(close, 150));

  // ---- the picker
  const button = h('button', {
    type: 'button', class: 'small ghost', title: 'Snippets (or type /)', 'aria-label': 'Insert a saved snippet',
    onclick: () => picker(box, async (sn) => insert(box, await fill(sn))),
  }, '💬 Snippets');
  return { field: h('div', { class: 'snippet-wrap' }, box, pop), button };
}

/** The snippet picker: search, insert, and manage. */
async function picker(box, onPick) {
  const list = await load(true);
  const search = h('input', { type: 'search', placeholder: 'Search snippets', 'aria-label': 'Search snippets' });
  const body = h('div', { class: 'snippet-list' });
  const draw = () => mount(body,
    list.length === 0 && h('p', { class: 'muted' }, 'No snippets yet. Save feedback you give often, then insert it with a tap or by typing / in a message.'),
    matching(list, search.value).map((sn) => h('div', { class: 'snippet-row' },
      h('button', { class: 'snippet-use', onclick: () => { sh.close(); onPick(sn); } },
        h('strong', {}, sn.title), h('div', { class: 'muted small' }, sn.text)),
      h('button', { class: 'small ghost', 'aria-label': `Edit ${sn.title}`, onclick: () => { sh.close(); editor(sn, box, onPick); } }, '✎'))));
  search.addEventListener('input', draw);
  const sh = sheet(
    h('h2', {}, 'Snippets'),
    search,
    body,
    h('p', { class: 'muted small' }, 'Tip: type / in a message, then part of a name. {name} becomes the client’s first name.'),
    h('div', { class: 'row' },
      h('button', { class: 'grow', onclick: () => { sh.close(); editor(null, box, onPick); } }, '+ New snippet'),
      h('button', { class: 'ghost grow', onclick: () => sh.close() }, 'Close')),
  );
  draw();
  search.focus();
}

/** Add (sn = null) or edit a snippet. A new one starts from what's in the box. */
function editor(sn, box, onPick) {
  const title = h('input', { value: sn?.title || '', maxlength: 60 });
  const text = h('textarea', { value: sn?.text ?? box.value.trim(), maxlength: 2000, style: 'min-height:120px' });
  const sh = sheet(
    h('h2', {}, sn ? 'Edit snippet' : 'New snippet'),
    field('Name', title, 'Short, for finding it: e.g. "depth", "brace"'),
    field('Text', text, '{name} becomes the client’s first name.'),
    h('button', {
      class: 'primary block',
      onclick: (e) => busy(e.currentTarget, async () => {
        const body = { title: title.value, text: text.value };
        await (sn ? api('PUT', `/snippets/${sn.id}`, body) : api('POST', '/snippets', body));
        await load(true);
        toast('Snippet saved');
        sh.close();
        picker(box, onPick);
      }),
    }, 'Save'),
    sn && h('button', {
      class: 'danger block',
      onclick: async () => {
        if (!(await confirmSheet(`Delete the "${sn.title}" snippet?`, 'Delete'))) return;
        await api('DELETE', `/snippets/${sn.id}`);
        await load(true);
        sh.close();
        picker(box, onPick);
      },
    }, 'Delete'),
    h('button', { class: 'ghost block', onclick: () => { sh.close(); picker(box, onPick); } }, 'Cancel'),
  );
}
