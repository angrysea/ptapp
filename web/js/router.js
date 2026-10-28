// Hash router: route('/c/:id', fn) -> fn(id).

const routes = [];
let fallback = '/';

export function route(pattern, fn) {
  const re = new RegExp('^' + pattern.replace(/:(\w+)/g, '([^/]+)') + '$');
  routes.push([re, fn]);
}

export const go = (path) => { location.hash = '#' + path; };
export const current = () => location.hash.slice(1).split('?')[0] || '/';
export const query = () => new URLSearchParams(location.hash.split('?')[1] || '');

let onChange = () => {};
export function start({ home, afterRender }) {
  fallback = home;
  onChange = afterRender || onChange;
  window.addEventListener('hashchange', schedule);
  schedule();
}

// Views await data before drawing, so two quick navigations could finish out
// of order and leave the older page on screen. Renders run one at a time,
// and a navigation superseded while queued is skipped: the latest one wins.
let queue = Promise.resolve();
function schedule() {
  const path = current();
  queue = queue.then(() => (path === current() ? run() : undefined));
}

async function run() {
  const path = current();
  for (const [re, fn] of routes) {
    const m = path.match(re);
    if (m) {
      window.scrollTo(0, 0);
      onChange(path);
      try { await fn(...m.slice(1).map(decodeURIComponent)); } catch (e) { console.error(e); }
      return;
    }
  }
  go(fallback);
}
