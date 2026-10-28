// Coach: the set-video review queue (VS-4). Every client's unreviewed videos,
// oldest first, to watch, comment on and mark reviewed.

import { get, post } from '../api.js';
import { go } from '../router.js';
import { $app, busy, fmtDateTime, h, mount, toast } from '../ui.js';
import { playVideo } from '../videos.js';
import { commentOn } from './comments.js';

const DAY = 24 * 3600 * 1000;

function waited(createdAt) {
  const ms = Date.now() - Date.parse(createdAt);
  if (ms < 3600 * 1000) return 'just now';
  if (ms < DAY) return `${Math.round(ms / 3600000)} h ago`;
  return `${Math.round(ms / DAY)} d ago`;
}

/** Shows the number of unreviewed videos on the coach's Videos tab. */
export async function refreshVideoBadge(count) {
  const tab = document.querySelector('#tabs a[data-path="/videos"]');
  if (!tab) return;
  if (count == null) {
    try { count = (await get('/videos/queue')).length; } catch { return; }
  }
  let badge = tab.querySelector('.badge');
  if (!count) { badge?.remove(); return; }
  if (!badge) { badge = h('span', { class: 'badge' }); tab.append(badge); }
  badge.textContent = String(count);
  badge.setAttribute('aria-label', `${count} to review`);
}

export async function renderVideoQueue() {
  const queue = await get('/videos/queue');
  refreshVideoBadge(queue.length);
  const markReviewed = async (v) => {
    await post(`/clients/${v.clientId}/videos/${v.videoId}/reviewed`);
    toast('Marked reviewed');
    renderVideoQueue();
  };
  mount($app(),
    h('h1', {}, 'Videos'),
    h('p', { class: 'muted' }, queue.length
      ? `${queue.length} to review, oldest first. Mark each one reviewed once you’ve given feedback.`
      : 'Nothing to review. Set videos from your clients show up here.'),
    queue.map((v) => {
      const old = Date.now() - Date.parse(v.createdAt) > 2 * DAY;
      const watch = () => {
        const sh = playVideo(v,
          h('button', { class: 'primary block', onclick: (e) => busy(e.currentTarget, async () => { sh.close(); await markReviewed(v); }) }, '✓ Mark reviewed'));
      };
      return h('div', { class: 'card queue-item' },
        h('button', { class: 'thumb', onclick: watch, 'aria-label': `Watch ${v.clientName}’s ${v.exerciseName}, set ${v.setIndex + 1}` },
          v.posterUrl ? h('img', { src: v.posterUrl, alt: '' }) : h('span', { 'aria-hidden': 'true' }, '▶')),
        h('div', { class: 'queue-body' },
          h('div', { class: 'spread' },
            h('strong', {}, v.clientName),
            h('span', { class: `chip ${old ? 'warn' : ''}`, title: fmtDateTime(v.createdAt) }, waited(v.createdAt))),
          h('div', { class: 'small' }, `${v.exerciseName} · set ${v.setIndex + 1}${v.durationSec ? ` · ${Math.round(v.trimEnd != null ? v.trimEnd - v.trimStart : v.durationSec)} s` : ''}`),
          h('div', { class: 'row', style: 'margin-top:8px' },
            h('button', { class: 'small', onclick: watch }, '▶ Watch'),
            h('button', { class: 'small', onclick: async () => { if (await commentOn(v.clientId, v.logId, v.setIndex, v.itemId)) toast('Comment sent'); } }, '💬 Comment'),
            h('button', { class: 'small ghost', onclick: () => go(`/c/${v.clientId}/logs`) }, 'Open workout'),
            h('button', { class: 'small primary', onclick: (e) => busy(e.currentTarget, () => markReviewed(v)) }, '✓ Reviewed'))));
    }),
  );
}
