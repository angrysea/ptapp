// FB-1 coach feedback and the coach-client message thread (MS-1). The thread
// refreshes itself on live events (realtime.js) and marks itself read while
// it's on screen, which clears the unread badges (views/messages.js).

import { get, post } from '../api.js';
import { onLive } from '../realtime.js';
import { session } from '../session.js';
import { busy, fmtDate, fmtDateTime, h, mount, sheet, shrinkImage, toast } from '../ui.js';
import { playReply, playVideo } from '../videos.js';
import { markRead } from './messages.js';
import { guard } from './credits.js';
import { snippetTools } from './snippets.js';
import { dismissPrompt, enable as pushOn, prefetchKey, shouldPrompt } from '../push.js';

/** Offers notifications once (until dismissed) where replies are awaited. */
export async function pushPrompt(who) {
  if (!(await shouldPrompt())) return null;
  prefetchKey();
  const card = h('div', { class: 'card push-prompt' },
    h('p', {}, `Get a notification when ${who} replies?`),
    h('div', { class: 'row' },
      h('button', {
        class: 'primary small',
        onclick: async () => {
          const s = await pushOn().catch(() => 'off');
          dismissPrompt();
          card.remove();
          toast(s === 'on' ? 'Notifications are on' : s === 'denied' ? 'Notifications are blocked; you can allow them later in your settings' : 'Notifications aren’t available here');
        },
      }, 'Turn on'),
      h('button', { class: 'ghost small', onclick: () => { dismissPrompt(); card.remove(); } }, 'Not now')));
  return card;
}

const VOICE_TYPES = ['audio/mp4', 'audio/webm;codecs=opus', 'audio/webm', 'audio/ogg;codecs=opus'];
const MAX_VOICE_S = 120;

/** Uploads a photo or voice note for the thread; returns the message's `attachment`. */
async function upload(clientId, kind, blob, durationSec) {
  const contentType = blob.type.split(';')[0];
  const up = await post(`/clients/${clientId}/attachments`, { kind, contentType, size: blob.size });
  const res = await fetch(up.uploadUrl, { method: 'PUT', headers: up.uploadHeaders, body: blob });
  if (!res.ok) throw new Error(`Upload failed (${res.status})`);
  return { kind, key: up.key, contentType, durationSec: durationSec ?? null };
}

/** A message's photo (tap for full size) or voice note. */
function attachmentView(c) {
  const a = c.attachment;
  if (!a) return null;
  if (!c.attachmentUrl) return h('div', { class: 'small muted' }, a.kind === 'photo' ? '📷 Photo (needs a connection)' : '🎤 Voice note (needs a connection)');
  if (a.kind === 'voice') return h('audio', { src: c.attachmentUrl, controls: true, preload: 'none', class: 'msg-voice' });
  return h('img', {
    src: c.attachmentUrl, alt: 'Photo', loading: 'lazy', class: 'msg-photo',
    onclick: () => { const s = sheet(h('img', { src: c.attachmentUrl, alt: 'Photo', class: 'msg-photo-full' }), h('button', { class: 'block', onclick: () => s.close() }, 'Close')); },
  });
}

export async function renderThread(el, clientId, logsById = {}) {
  const listEl = h('div', { class: 'thread' });
  const text = h('textarea', { placeholder: 'Write a message…', style: 'min-height:70px', 'aria-label': 'Message' });
  const anchor = (c) => {
    if (!c.logId) return null;
    const set = c.setIndex != null ? ` set ${c.setIndex + 1}` : '';
    const a = c.about;
    if (!a) {
      const log = logsById[c.logId];
      return h('div', { class: 'meta' }, `on ${log ? log.workoutName : 'a workout'}${set && `,${set}`}`);
    }
    // MS-3: a card that opens what the message is about.
    const label = `${c.videoId ? '🎥' : '🏋️'} ${a.workoutName} · ${fmtDate(a.date)}${a.exerciseName ? ` · ${a.exerciseName}${set}` : set}`;
    const href = session.isCoach
      ? (c.videoId ? `#/review/${clientId}/${c.videoId}` : `#/c/${clientId}/logs/${c.logId}`)
      : (!c.videoId && a.scheduledId ? `#/workout/${a.date}/${a.scheduledId}` : null);
    if (href) return h('a', { class: 'about', href }, label);
    if (c.videoId) {
      return h('button', {
        class: 'about', type: 'button',
        onclick: async () => {
          const v = (await get(`/clients/${clientId}/videos`)).find((x) => x.id === c.videoId);
          if (v?.url) playVideo(v); else toast('That video is no longer available');
        },
      }, label);
    }
    return h('div', { class: 'meta' }, label);
  };
  let shown = null; // id of the newest message drawn, to redraw and mark read only on change
  async function draw() {
    const list = await get(`/clients/${clientId}/comments`, { cache: true });
    const newest = list.at(-1)?.id ?? '';
    if (newest === shown) return;
    const first = shown === null;
    shown = newest;
    // Messages that carry a coach's video reply (VR-2) get a play button.
    let replies = null; // null: couldn't load them (offline)
    if (list.some((c) => c.replyId)) {
      try { replies = Object.fromEntries((await get(`/clients/${clientId}/replies`)).map((r) => [r.id, r])); } catch { /* offline */ }
    }
    const play = (c) => {
      if (!c.replyId) return null;
      const r = replies?.[c.replyId];
      if (!r?.url) return h('div', { class: 'small muted' }, replies ? 'Video reply (no longer available)' : 'Video reply (needs a connection)');
      return h('button', {
        class: 'small reply', style: 'margin-top:6px',
        onclick: () => playReply(r, {
          onPlay: () => { if (!session.isCoach && !r.seenAt) post(`/clients/${clientId}/replies/${r.id}/seen`).catch(() => {}); },
        }),
      }, '▶ Play video reply');
    };
    mount(listEl,
      list.length === 0 && h('p', { class: 'muted' }, 'No messages yet.'),
      list.map((c) => h('div', { class: `msg ${c.authorId === session.userId ? 'mine' : ''}` },
        h('div', { class: 'meta' }, `${c.authorName} · ${fmtDateTime(c.createdAt)}`),
        anchor(c),
        attachmentView(c),
        c.text && h('div', { style: 'white-space:pre-wrap' }, c.text),
        play(c))));
    listEl.lastElementChild?.scrollIntoView({ block: 'nearest', behavior: first ? 'auto' : 'smooth' });
    if (list.some((c) => c.authorId !== session.userId) && document.visibilityState === 'visible') markRead(clientId);
  }
  // ---- photos and voice notes (they need a connection: they upload first)
  const pending = h('div'); // a recorded voice note waiting to be sent
  const sendMessage = async (attachment) => {
    if (!(await guard(clientId, attachment?.kind === 'photo' ? 'photoReview' : 'message'))) return;
    await post(`/clients/${clientId}/comments`, { text: text.value, ...(attachment && { attachment }) });
    text.value = '';
    await draw();
  };
  const photoPick = h('input', { type: 'file', accept: 'image/*', hidden: true, 'aria-label': 'Photo to send' });
  const photoBtn = h('button', { class: 'small', 'aria-label': 'Send a photo', onclick: () => photoPick.click() }, '📷 Photo');
  photoPick.addEventListener('change', () => busy(photoBtn, async () => {
    const f = photoPick.files?.[0];
    photoPick.value = '';
    if (!f || !(await guard(clientId, 'photoReview'))) return; // before the upload, not after
    await sendMessage(await upload(clientId, 'photo', await shrinkImage(f, 1600)));
  }).catch((e) => toast(e.message, 'error')));

  let rec = null;
  const voiceBtn = h('button', { class: 'small', 'aria-label': 'Record a voice note', onclick: () => (rec ? rec.stop() : startVoice()) }, '🎤 Voice');
  async function startVoice() {
    if (!(await guard(clientId, 'message'))) return;
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
    } catch (e) {
      return toast(e.name === 'NotAllowedError' ? 'The microphone is blocked; allow it in your browser settings.' : `Couldn’t start the microphone: ${e.message}`, 'error');
    }
    const mimeType = VOICE_TYPES.find((t) => MediaRecorder.isTypeSupported(t)) || '';
    rec = new MediaRecorder(stream, { ...(mimeType && { mimeType }), audioBitsPerSecond: 64_000 });
    const chunks = [];
    const started = performance.now();
    const tick = setInterval(() => {
      const s = (performance.now() - started) / 1000;
      voiceBtn.textContent = `■ Stop ${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
      if (s >= MAX_VOICE_S) rec?.stop();
    }, 250);
    rec.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data); };
    rec.onstop = () => {
      clearInterval(tick);
      stream.getTracks().forEach((t) => t.stop());
      const blob = new Blob(chunks, { type: (rec.mimeType || mimeType || 'audio/webm').split(';')[0] });
      const seconds = Math.min(MAX_VOICE_S, (performance.now() - started) / 1000);
      rec = null;
      voiceBtn.textContent = '🎤 Voice';
      const url = URL.createObjectURL(blob);
      const clear = () => { URL.revokeObjectURL(url); mount(pending); };
      mount(pending, h('div', { class: 'voice-take' },
        h('audio', { src: url, controls: true }),
        h('div', { class: 'row' },
          h('button', { class: 'ghost small', onclick: clear }, 'Discard'),
          h('button', {
            class: 'primary small',
            onclick: (e) => busy(e.currentTarget, async () => { await sendMessage(await upload(clientId, 'voice', blob, seconds)); clear(); }),
          }, 'Send voice note'))));
    };
    rec.start(1000);
    voiceBtn.textContent = '■ Stop 0:00';
  }
  const canVoice = typeof MediaRecorder === 'function' && !!navigator.mediaDevices?.getUserMedia;
  const snippets = session.isCoach ? snippetTools(text, clientId) : { field: text, button: null };

  mount(el,
    await pushPrompt(session.isCoach ? 'your client' : 'your coach'),
    listEl,
    h('div', { class: 'card' }, snippets.field,
      pending,
      h('div', { class: 'row', style: 'margin-top:8px' },
        photoBtn, photoPick,
        canVoice && voiceBtn,
        snippets.button,
        h('button', {
          class: 'primary grow',
          onclick: (e) => busy(e.currentTarget, async () => {
            if (!text.value.trim()) return;
            await sendMessage();
          }),
        }, 'Send'))),
  );
  await draw();
  // New messages appear while the thread is open; stops once it's gone.
  const off = onLive(() => {
    if (!el.isConnected) return off();
    draw().catch(() => {});
  });
}

/** Prompt for a comment anchored to a workout log or one set in it. */
/** A message about a workout, one of its sets, or a set video (MS-3), from
 * the coach or the client. Resolves true once sent. */
export function commentOn(clientId, logId, setIndex, itemId, videoId) {
  return new Promise((resolve) => {
    const text = h('textarea', { placeholder: session.isCoach ? 'Your comment' : 'Your question or note for your coach', 'aria-label': 'Comment', style: 'min-height:90px', maxlength: 4000 });
    const snippets = session.isCoach ? snippetTools(text, clientId) : { field: text, button: null };
    let sent = false;
    const sh = sheet(
      h('h2', {}, videoId ? `Comment on the set ${setIndex + 1} video` : setIndex != null ? `Comment on set ${setIndex + 1}` : session.isCoach ? 'Comment on this workout' : 'Ask your coach about this workout'),
      snippets.field,
      h('div', { class: 'row', style: 'margin-top:8px' },
        snippets.button,
        h('button', { class: 'ghost grow', onclick: () => sh.close() }, 'Cancel'),
        h('button', {
          class: 'primary grow',
          onclick: (e) => busy(e.currentTarget, async () => {
            if (!text.value.trim() || !(await guard(clientId, 'message'))) return;
            await post(`/clients/${clientId}/comments`, videoId ? { text: text.value, videoId } : { text: text.value, logId, setIndex: setIndex ?? null, itemId: itemId ?? null });
            sent = true;
            sh.close();
          }),
        }, 'Send')),
    );
    sh.onClose = () => resolve(sent);
    text.focus();
  });
}
