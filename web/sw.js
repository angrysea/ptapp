// Service worker: makes the app shell available offline (LG-3, offline NFR).
// Network-first for our own files so deploys show up immediately; falls back
// to the cache when offline. API, S3 and Cognito requests are not touched
// here (the app caches API reads in IndexedDB itself).

const VERSION = 'dev';
const CACHE = `ptapp-${VERSION}`;
const SHELL = [
  './', 'index.html', 'manifest.webmanifest', 'icon.svg', 'css/app.css',
  'js/app.js', 'js/api.js', 'js/auth.js', 'js/config.js', 'js/push.js', 'js/realtime.js', 'js/effort.js', 'js/encode.js', 'js/idb.js', 'js/router.js', 'js/session.js', 'js/timer.js', 'js/ui.js', 'js/uploads.js', 'js/videos.js',
  'js/views/admin.js', 'js/views/coach.js', 'js/views/coachprofile.js', 'js/views/consent.js', 'js/views/maxes.js', 'js/views/records.js', 'js/views/progression.js', 'js/views/snippets.js', 'js/views/alerts.js', 'js/views/checkins.js', 'js/views/automation.js', 'js/views/directory.js', 'js/views/comments.js', 'js/views/exercises.js', 'js/views/intake.js', 'js/views/messages.js', 'js/views/metrics.js',
  'js/views/onboarding.js', 'js/views/profile.js', 'js/views/programs.js', 'js/views/review.js', 'js/views/recorder.js', 'js/views/signup.js', 'js/views/trim.js', 'js/views/videos.js', 'js/views/workout.js',
  // set-video conversion (encode.js); precached so it works offline
  'vendor/mp4box-0.5.2.all.min.js', 'vendor/mp4-muxer-5.1.3.js',
];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys()
    .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
    .then(() => self.clients.claim()));
});

// Web Push (MS-2). Every push shows a notification: Safari revokes the
// subscription of a site whose pushes show nothing. One notification per
// tag, so a burst of messages in one thread replaces rather than stacks.
self.addEventListener('push', (e) => {
  let d = {};
  try { d = e.data?.json() ?? {}; } catch { d = { body: e.data?.text() }; }
  e.waitUntil(self.registration.showNotification(d.title || 'Personal Training Coach', {
    body: d.body || '', tag: d.tag, renotify: !!d.tag, icon: 'icon.svg', badge: 'icon.svg', data: { url: d.url || './' },
  }));
});

// Opening a notification focuses the app if it's open, else opens it, on the notification's page.
self.addEventListener('notificationclick', (e) => {
  e.notification.close();
  const url = new URL(e.notification.data?.url || './', self.registration.scope).href;
  e.waitUntil(self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((wins) => {
    const win = wins.find((w) => w.url.startsWith(self.registration.scope));
    return win ? win.focus().then((w) => (w || win).navigate(url)) : self.clients.openWindow(url);
  }));
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
