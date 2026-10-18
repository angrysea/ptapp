// Service worker: makes the app shell available offline (LG-3, offline NFR).
// Network-first for our own files so deploys show up immediately; falls back
// to the cache when offline. API, S3 and Cognito requests are not touched
// here (the app caches API reads in IndexedDB itself).

const VERSION = 'dev';
const CACHE = `ptapp-${VERSION}`;
const SHELL = [
  './', 'index.html', 'manifest.webmanifest', 'icon.svg', 'css/app.css',
  'js/app.js', 'js/api.js', 'js/auth.js', 'js/config.js', 'js/idb.js', 'js/router.js', 'js/session.js', 'js/timer.js', 'js/ui.js',
  'js/views/coach.js', 'js/views/comments.js', 'js/views/exercises.js', 'js/views/intake.js', 'js/views/metrics.js',
  'js/views/onboarding.js', 'js/views/profile.js', 'js/views/programs.js', 'js/views/workout.js',
];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys()
    .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
    .then(() => self.clients.claim()));
});

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET' || url.origin !== self.location.origin) return;
  e.respondWith(
    fetch(e.request)
      .then((res) => {
        if (res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(e.request, copy));
        }
        return res;
      })
      // Serving the HTML shell for a failed css/js miss would return wrong
      // content; only the navigation entry point may fall back to it.
      .catch(() => caches.match(e.request, { ignoreSearch: true }).then((hit) => hit
        || (e.request.mode === 'navigate' ? caches.match('./') : undefined))),
  );
});
