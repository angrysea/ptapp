// The coach's review screen for a client's set video (VR-1..VR-3), at
// #/review/{clientId}/{videoId}. The coach studies the clip (slow motion,
// frame steps, scrubbing), marks the segments worth talking about, and
// records a reply: the clip exactly as they play, pause, slow and step
// through it, drawn on a canvas, with their voice over it and the clip's own
// sound lowered underneath, and whatever they draw over it (draw.js, VR-4),
// and their own webcam in a corner if they turn it on (VR-6).
// Or they write a reply instead. Sending marks the
// video reviewed (routes/replies.rs); recordings upload through uploads.js,
// so they survive a lost connection. Decisions: tasks/reply/plan.md.

import { get, post } from '../api.js';
import { go } from '../router.js';
import { session } from '../session.js';
import { $app, busy, fmtDate, fmtDateTime, h, mount, toast } from '../ui.js';
import { queueReply } from '../uploads.js';
import { askBeforeCellular } from '../videos.js';
import { snippetTools } from './snippets.js';
import { drawingTools } from './draw.js';

const MAX_REPLY_S = 300;
const FRAME = 1 / 30; // the clip's frame rate isn't exposed; 30 fps is the phones' default
const SPEEDS = [0.25, 0.5, 1];
const CLIP_UNDER_VOICE = 0.25; // the clip's own sound while recording
const TYPES = ['video/mp4;codecs=avc1.42E01E,mp4a.40.2', 'video/mp4;codecs=avc1,mp4a.40.2', 'video/mp4', 'video/webm;codecs=vp9,opus', 'video/webm'];
/** Set when the coach opens the review queue, to time the BRD's five-minute review. */
export const QUEUE_OPENED = 'pt.reviewQueueOpenedAt';

const fmt = (s) => `${Math.floor(s / 60)}:${(s % 60).toFixed(1).padStart(4, '0')}`;
const segKey = (vid) => `pt.segments.${vid}`;
const CAM_PREF = 'pt.replyCam'; // the coach's camera choice, kept for the next review
const CAM_SHARE = 0.28; // the camera's width as a share of the clip's
const CORNERS = [['br', 'Bottom right'], ['bl', 'Bottom left'], ['tr', 'Top right'], ['tl', 'Top left']];
const camPref = () => { try { return JSON.parse(localStorage.getItem(CAM_PREF)) || {}; } catch { return {}; } };
const saveCamPref = (p) => { try { localStorage.setItem(CAM_PREF, JSON.stringify(p)); } catch { /* not kept */ } };
const loadSegs = (vid) => { try { return JSON.parse(localStorage.getItem(segKey(vid))) || []; } catch { return []; } };
const saveSegs = (vid, s) => { try { localStorage.setItem(segKey(vid), JSON.stringify(s)); } catch { /* not kept */ } };

export async function renderReview(cid, vid) {
  const opened = Number(sessionStorage.getItem(QUEUE_OPENED)) || Date.now();
  const [videos, detail] = await Promise.all([get(`/clients/${cid}/videos`), get(`/clients/${cid}`).catch(() => null)]);
  const v = videos.find((x) => x.id === vid);
  const clientName = detail?.profile?.name || 'Client';
  const back = h('button', { class: 'ghost small', onclick: () => go('/videos') }, '‹ Videos');
  if (!v) return mount($app(), back, h('h1', {}, 'Video not found'), h('p', {}, 'It may have been deleted by the client.'));

  // Firefox records only WebM, which iPhones may not play (decision from review).
  const firefox = /Firefox\//.test(navigator.userAgent);
  const canRecord = !firefox && typeof MediaRecorder === 'function' && typeof HTMLCanvasElement.prototype.captureStream === 'function'
    && !!navigator.mediaDevices?.getUserMedia && !!window.AudioContext;

  const lo = v.trimStart ?? 0;
  let hi = v.trimEnd ?? Infinity;
  const video = h('video', { crossorigin: 'anonymous', playsinline: true, preload: 'auto', src: v.url, class: 'review-src' });
  const canvas = h('canvas', { class: 'review-canvas', 'aria-label': `${clientName}’s ${v.exerciseName}, set ${v.setIndex + 1}` });
  const ctx2d = canvas.getContext('2d');
  const marks = drawingTools(canvas, () => video.currentTime); // VR-4
  const time = h('span', { class: 'small muted review-time' }, '0:00.0');
  const scrub = h('input', { type: 'range', min: 0, max: 1, step: 0.01, value: 0, 'aria-label': 'Position' });
  const playBtn = h('button', { class: 'primary', 'aria-label': 'Play', onclick: () => toggle() }, '▶');
  const speedBtns = SPEEDS.map((r) => h('button', { class: r === 1 ? 'on' : '', onclick: () => setRate(r), 'aria-label': `${r}× speed` }, `${r}×`));

  // ---- the audio graph: the clip lowered under the coach's voice while recording
  let ac = null, toRecording = null, monitor = null, mic = null;
  function ensureAudio() {
    if (ac || !window.AudioContext) return;
    try {
      ac = new AudioContext();
      toRecording = ac.createMediaStreamDestination();
      const clip = ac.createMediaElementSource(video); // the bucket's CORS response allows this
      monitor = ac.createGain(); // what the coach hears
      clip.connect(monitor).connect(ac.destination);
      const under = ac.createGain();
      under.gain.value = CLIP_UNDER_VOICE;
      clip.connect(under).connect(toRecording);
    } catch (e) {
      console.warn('no audio graph', e);
      ac = null;
    }
  }

  // ---- playback
  const clamp = (t) => Math.min(Math.max(t, lo), Math.min(hi, video.duration || hi));
  function toggle() {
    ensureAudio();
    ac?.resume();
    playlist = null;
    if (video.paused) {
      if (video.currentTime >= Math.min(hi, video.duration) - 0.05) video.currentTime = lo;
      video.play();
    } else video.pause();
  }
  function setRate(r) {
    video.playbackRate = r;
    speedBtns.forEach((b, i) => b.classList.toggle('on', SPEEDS[i] === r));
  }
  function step(frames) {
    video.pause();
    playlist = null;
    video.currentTime = clamp(video.currentTime + frames * FRAME);
  }
  function seek(t) { playlist = null; video.currentTime = clamp(t); }
  video.addEventListener('play', () => { playBtn.textContent = '❚❚'; playBtn.setAttribute('aria-label', 'Pause'); });
  video.addEventListener('pause', () => { playBtn.textContent = '▶'; playBtn.setAttribute('aria-label', 'Play'); });
  let mainW = 0, mainH = 0; // the clip's drawn size; the canvas widens beside it to compare
  video.addEventListener('loadedmetadata', () => {
    const scale = Math.min(1, 1280 / Math.max(video.videoWidth, video.videoHeight));
    mainW = Math.round(video.videoWidth * scale);
    mainH = Math.round(video.videoHeight * scale);
    sizeCanvas();
    hi = Math.min(hi, video.duration);
    scrub.min = lo; scrub.max = hi;
    video.currentTime = lo;
  });
  video.addEventListener('timeupdate', () => {
    if (playlist) return advancePlaylist();
    if (video.currentTime >= hi) { video.pause(); video.currentTime = hi; }
  });
  scrub.addEventListener('input', () => seek(Number(scrub.value)));

  // ---- segments (VR-3): a playlist the coach can play through while recording
  let segs = loadSegs(vid);
  let markIn = null;
  let playlist = null; // { i, only } while playing segments: all from i in order, or only segment i
  const segList = h('div');
  const inBtn = h('button', { onclick: () => markInAt() }, 'In');
  const outBtn = h('button', { onclick: () => markOutAt() }, 'Out');
  function markInAt() { markIn = video.currentTime; drawSegs(); }
  function markOutAt() {
    const t = video.currentTime;
    if (markIn == null) return toast('Mark In first');
    if (t <= markIn + 0.05) return toast('Out must come after In');
    segs.push({ a: +markIn.toFixed(3), b: +t.toFixed(3) });
    markIn = null;
    saveSegs(vid, segs);
    drawSegs();
  }
  function move(i, d) {
    const j = i + d;
    if (j < 0 || j >= segs.length) return;
    [segs[i], segs[j]] = [segs[j], segs[i]];
    saveSegs(vid, segs);
    drawSegs();
  }
  function playSegs(from = 0, only = false) {
    if (!segs.length) return;
    ensureAudio();
    ac?.resume();
    playlist = { i: from, only };
    video.currentTime = segs[from].a;
    video.play();
  }
  function advancePlaylist() {
    const seg = segs[playlist.i];
    if (!seg || video.currentTime < seg.b) return;
    playlist.i += 1;
    if (playlist.only || playlist.i >= segs.length) { playlist = null; video.pause(); return; }
    video.currentTime = segs[playlist.i].a;
  }
  function drawSegs() {
    mount(segList,
      markIn != null && h('p', { class: 'small' }, `In at ${fmt(markIn)}; now mark Out.`),
      segs.length === 0 && markIn == null && h('p', { class: 'small muted' }, 'Mark In and Out to save the parts worth talking about. Play segments runs them in order.'),
      segs.map((s, i) => h('div', { class: 'row seg' },
        h('span', { class: 'grow small' }, `${i + 1}. ${fmt(s.a)} – ${fmt(s.b)}`),
        h('button', { class: 'small', 'aria-label': `Play segment ${i + 1}`, onclick: () => playSegs(i, true) }, '▶'),
        h('button', { class: 'small ghost', 'aria-label': 'Move up', onclick: () => move(i, -1) }, '↑'),
        h('button', { class: 'small ghost', 'aria-label': 'Move down', onclick: () => move(i, 1) }, '↓'),
        h('button', { class: 'small ghost', 'aria-label': `Remove segment ${i + 1}`, onclick: () => { segs.splice(i, 1); saveSegs(vid, segs); drawSegs(); } }, '✕'))),
      segs.length > 0 && h('button', { class: 'block', onclick: () => playSegs(0) }, '▶ Play segments'));
  }
  drawSegs();

  // ---- compare (VR-5): an earlier clip of the same exercise, or its demo, in step with this one
  const ref = h('video', { crossorigin: 'anonymous', playsinline: true, muted: true, preload: 'auto', class: 'review-src' });
  ref.muted = true;
  let refOn = null; // { lo, mode: 'side' | 'overlay' }
  const earlier = videos
    .filter((x) => x.exerciseId === v.exerciseId && x.id !== v.id && x.url)
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  const demoUrl = session.exercises?.[v.exerciseId]?.demoVideoUrl;
  const sources = [
    ...(demoUrl ? [{ key: 'demo', label: `${v.exerciseName} demo`, url: demoUrl, lo: 0 }] : []),
    ...earlier.map((x) => ({ key: x.id, label: `${fmtDate(x.createdAt.slice(0, 10))} · set ${x.setIndex + 1}`, url: x.url, lo: x.trimStart ?? 0 })),
  ];
  function sizeCanvas() {
    if (!mainW) return;
    const side = refOn?.mode === 'side' && ref.videoWidth;
    canvas.width = side ? mainW + Math.round((ref.videoWidth * mainH) / ref.videoHeight) : mainW;
    canvas.height = mainH;
  }
  ref.addEventListener('loadedmetadata', sizeCanvas);
  function syncRef() {
    if (!refOn || ref.readyState < 1) return;
    const want = refOn.lo + (video.currentTime - lo);
    if (want > (ref.duration || Infinity)) { if (!ref.paused) ref.pause(); return; }
    // Paused (stepping frames): match exactly. Playing: only correct real drift.
    if (Math.abs(ref.currentTime - want) > (video.paused ? 0.02 : 0.15)) ref.currentTime = Math.max(0, want);
    if (ref.playbackRate !== video.playbackRate) ref.playbackRate = video.playbackRate;
    if (video.paused !== ref.paused) { if (video.paused) ref.pause(); else ref.play().catch(() => {}); }
  }
  const compareSel = h('select', { 'aria-label': 'Compare with', onchange: () => setRef() },
    h('option', { value: '' }, 'Nothing'), sources.map((x) => h('option', { value: x.key }, x.label)));
  const modeBtns = [['side', 'Side by side'], ['overlay', 'Overlay']].map(([m, label]) => h('button', {
    class: `small ${m === 'side' ? 'primary' : ''}`, 'aria-pressed': String(m === 'side'),
    onclick: () => {
      if (refOn) refOn.mode = m;
      modeBtns.forEach((b) => { const on = b.textContent === label; b.classList.toggle('primary', on); b.setAttribute('aria-pressed', String(on)); });
      sizeCanvas();
    },
  }, label));
  function setRef() {
    if (recorder?.state === 'recording') { compareSel.value = refOn?.key || ''; return toast('Stop recording to change the comparison'); }
    const src = sources.find((x) => x.key === compareSel.value);
    const mode = modeBtns.find((b) => b.classList.contains('primary'))?.textContent === 'Overlay' ? 'overlay' : 'side';
    refOn = src ? { key: src.key, lo: src.lo, mode } : null;
    if (src) { ref.src = src.url; ref.currentTime = src.lo; } else { ref.removeAttribute('src'); ref.load(); }
    sizeCanvas();
  }
  function drawFrame() {
    ctx2d.drawImage(video, 0, 0, mainW, mainH);
    if (refOn && ref.readyState >= 2) {
      if (refOn.mode === 'side') ctx2d.drawImage(ref, mainW, 0, canvas.width - mainW, mainH);
      else { ctx2d.globalAlpha = 0.5; ctx2d.drawImage(ref, 0, 0, mainW, mainH); ctx2d.globalAlpha = 1; }
    }
  }

  // ---- the coach's camera (VR-6): drawn into a corner of the clip, so it's
  // in the preview and the recording alike; mirrored like a selfie
  const cam = h('video', { muted: true, playsinline: true, autoplay: true, class: 'review-src' });
  cam.muted = true;
  let camStream = null;
  const pref = camPref();
  let corner = CORNERS.some(([k]) => k === pref.corner) ? pref.corner : 'br';
  const camToggle = h('input', { type: 'checkbox', checked: false, 'aria-label': 'Show my camera' });
  const cornerSel = h('select', { 'aria-label': 'Camera corner', disabled: true },
    CORNERS.map(([k, label]) => h('option', { value: k, selected: k === corner }, label)));
  cornerSel.addEventListener('change', () => { corner = cornerSel.value; saveCamPref({ on: !!camStream, corner }); });
  async function setCam(on) {
    if (on && !camStream) {
      try {
        camStream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'user', width: { ideal: 640 }, height: { ideal: 480 } } });
      } catch (e) {
        camToggle.checked = false;
        return toast(e.name === 'NotAllowedError'
          ? 'The camera is blocked. Allow it for this site in your browser settings.'
          : `Couldn’t start the camera: ${e.message}`, 'error');
      }
      cam.srcObject = camStream;
      cam.play().catch(() => {});
    } else if (!on && camStream) {
      camStream.getTracks().forEach((t) => t.stop());
      camStream = null;
      cam.srcObject = null;
    }
    camToggle.checked = !!camStream;
    cornerSel.disabled = !camStream;
    saveCamPref({ on: !!camStream, corner });
  }
  camToggle.addEventListener('change', () => setCam(camToggle.checked));
  function drawCam() {
    if (!camStream || cam.readyState < 2 || !cam.videoWidth) return;
    const w = Math.round(mainW * CAM_SHARE);
    const hgt = Math.round((w * cam.videoHeight) / cam.videoWidth);
    const pad = Math.round(mainW * 0.03);
    const x = corner.endsWith('r') ? mainW - w - pad : pad;
    const y = corner.startsWith('b') ? mainH - hgt - pad : pad;
    const r = Math.round(w * 0.08);
    ctx2d.save();
    ctx2d.beginPath();
    if (ctx2d.roundRect) ctx2d.roundRect(x, y, w, hgt, r); else ctx2d.rect(x, y, w, hgt);
    ctx2d.lineWidth = Math.max(2, Math.round(w * 0.015));
    ctx2d.strokeStyle = '#fff';
    ctx2d.stroke();
    ctx2d.clip();
    ctx2d.translate(x + w, y);
    ctx2d.scale(-1, 1);
    ctx2d.drawImage(cam, 0, 0, w, hgt);
    ctx2d.restore();
  }

  // ---- draw every animation frame, playing or not, so a recording runs at a steady rate
  function frame() {
    if (!canvas.isConnected) return cleanup();
    if (video.readyState >= 2 && canvas.width) {
      syncRef();
      drawFrame();
      marks.paint(ctx2d);
      drawCam();
    }
    time.textContent = `${fmt(Math.max(0, video.currentTime - lo))} / ${fmt(Math.max(0, (Number.isFinite(hi) ? hi : video.duration || 0) - lo))}`;
    if (document.activeElement !== scrub) scrub.value = video.currentTime;
    requestAnimationFrame(frame);
  }

  // ---- recording (VR-2)
  const recBtn = h('button', { class: 'primary block', onclick: () => (recorder?.state === 'recording' ? recorder.stop() : startRecording()) }, '● Record reply');
  const recTime = h('span', { class: 'small' });
  const result = h('div');
  let recorder = null, tick = null, take = null;

  async function startRecording() {
    ensureAudio();
    if (!ac) return toast('This browser couldn’t set up sound for recording. Try Chrome, Edge or Safari, or write a reply.', 'error');
    try {
      await ac?.resume();
      mic ||= await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
    } catch (e) {
      return toast(e.name === 'NotAllowedError'
        ? 'The microphone is blocked. Allow it for this site in your browser settings, or write a reply instead.'
        : `Couldn’t start the microphone: ${e.message}`, 'error');
    }
    const voice = ac.createMediaStreamSource(mic);
    voice.connect(toRecording);
    const canvasStream = canvas.captureStream(30);
    const stream = new MediaStream([...canvasStream.getVideoTracks(), ...toRecording.stream.getAudioTracks()]);
    const mimeType = TYPES.find((t) => MediaRecorder.isTypeSupported(t)) || '';
    // ~1.6 Mbps: a five-minute reply is about 60 MB.
    recorder = new MediaRecorder(stream, { ...(mimeType && { mimeType }), videoBitsPerSecond: 1_500_000, audioBitsPerSecond: 96_000 });
    const chunks = [];
    const started = performance.now();
    recorder.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data); };
    recorder.onstop = () => {
      clearInterval(tick);
      voice.disconnect();
      canvasStream.getTracks().forEach((t) => t.stop());
      monitor.gain.value = 1;
      const type = (recorder.mimeType || mimeType || 'video/mp4').split(';')[0];
      take = { blob: new Blob(chunks, { type }), seconds: Math.min(MAX_REPLY_S, (performance.now() - started) / 1000) };
      recBtn.textContent = '● Record again';
      recTime.textContent = '';
      showTake();
    };
    monitor.gain.value = CLIP_UNDER_VOICE; // the coach hears what the client will
    take = null;
    mount(result);
    recorder.start(1000);
    recBtn.textContent = '■ Stop recording';
    tick = setInterval(() => {
      const s = (performance.now() - started) / 1000;
      recTime.textContent = `● ${fmt(s)} of ${fmt(MAX_REPLY_S)}`;
      if (s >= MAX_REPLY_S) recorder.stop();
    }, 250);
  }

  const note = h('textarea', { placeholder: 'A note to go with it (optional)', 'aria-label': 'Note', maxlength: 4000 });
  const noteSnippets = snippetTools(note, cid);
  function showTake() {
    const url = URL.createObjectURL(take.blob);
    mount(result,
      h('h3', {}, `Your reply · ${fmt(take.seconds)}`),
      h('video', { src: url, controls: true, playsinline: true, class: 'player' }),
      noteSnippets.field,
      h('div', { class: 'row', style: 'margin-top:8px' },
        noteSnippets.button,
        h('button', { class: 'ghost', onclick: () => { take = null; URL.revokeObjectURL(url); mount(result); recBtn.textContent = '● Record reply'; } }, 'Discard'),
        h('button', { class: 'primary grow', onclick: (e) => busy(e.currentTarget, sendTake) }, 'Send reply')));
  }

  async function sendTake() {
    const when = await askBeforeCellular(take.blob.size);
    if (!when) return;
    await queueReply({
      clientId: cid, replyTo: vid, file: take.blob, durationSec: take.seconds, note: note.value,
      reviewMs: Date.now() - opened, label: `${clientName} · ${v.exerciseName} set ${v.setIndex + 1}`, waitForWifi: when.waitForWifi,
    });
    done(when.waitForWifi ? 'Your reply is saved on this device and sends when you’re on Wi-Fi.' : 'Sending your reply. You can carry on; it uploads in the background.');
  }

  const written = h('textarea', { placeholder: 'Write a reply instead', 'aria-label': 'Written reply', maxlength: 4000 });
  const writtenSnippets = snippetTools(written, cid);
  async function sendWritten() {
    if (!written.value.trim()) return toast('Write something first');
    await post(`/clients/${cid}/videos/${vid}/replies`, { note: written.value, reviewMs: Date.now() - opened });
    done('Reply sent');
  }

  function done(msg) {
    console.info(`review time: ${Math.round((Date.now() - opened) / 1000)} s`); // the BRD's five-minute gate
    saveSegs(vid, []);
    sessionStorage.removeItem(QUEUE_OPENED);
    toast(msg);
    go('/videos');
  }

  // ---- keyboard: space play/pause, , . frame steps, 1 2 3 speeds, ← → one second, i o segments
  function onKey(e) {
    if (e.target.closest?.('input, textarea, select') || e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    const acts = {
      ' ': toggle, ',': () => step(-1), '.': () => step(1), 1: () => setRate(0.25), 2: () => setRate(0.5), 3: () => setRate(1),
      ArrowLeft: () => seek(video.currentTime - 1), ArrowRight: () => seek(video.currentTime + 1), i: markInAt, o: markOutAt,
      Escape: () => marks.setTool('none'),
    };
    if (acts[k]) { e.preventDefault(); acts[k](); }
  }
  document.addEventListener('keydown', onKey);

  function cleanup() {
    document.removeEventListener('keydown', onKey);
    if (recorder?.state === 'recording') recorder.stop();
    clearInterval(tick);
    video.pause();
    ref.pause();
    mic?.getTracks().forEach((t) => t.stop());
    camStream?.getTracks().forEach((t) => t.stop());
    ac?.close().catch(() => {});
  }

  mount($app(),
    back,
    h('h1', {}, clientName),
    h('p', { class: 'muted' }, `${v.exerciseName} · set ${v.setIndex + 1} · ${fmtDateTime(v.createdAt)}`),
    video,
    ref, // hidden, like the clip and the camera: all are drawn into the canvas
    cam,
    canvas,
    marks.toolbar,
    h('div', { class: 'review-controls' },
      h('div', { class: 'row' },
        h('button', { 'aria-label': 'Back one frame', onclick: () => step(-1) }, '⏮'),
        playBtn,
        h('button', { 'aria-label': 'Forward one frame', onclick: () => step(1) }, '⏭'),
        h('span', { class: 'speeds row' }, speedBtns),
        time),
      scrub,
      h('p', { class: 'small muted keys' }, 'Keys: space play/pause · , and . step a frame · 1 2 3 speed · ← → one second · i and o mark a segment · Esc stops drawing')),
    h('div', { class: 'card' },
      h('div', { class: 'spread' }, h('h3', {}, 'Segments'), h('span', { class: 'row' }, inBtn, outBtn)),
      segList),
    h('div', { class: 'card' },
      h('h3', {}, 'Compare'),
      sources.length === 0
        ? h('p', { class: 'small muted' }, `Nothing to compare with yet: no earlier ${v.exerciseName} videos from ${clientName}, and no demo video for it.`)
        : [h('p', { class: 'small muted' }, 'Plays in step with this set; shows in your recording too.'),
          h('div', { class: 'row' }, h('div', { class: 'grow' }, compareSel), modeBtns)]),
    h('div', { class: 'card' },
      h('h3', {}, 'Video reply'),
      canRecord
        ? [h('p', { class: 'small muted' }, 'Records the clip as you play, pause, slow down and step through it, with your voice over it. Up to 5 minutes.'),
          h('div', { class: 'row', style: 'margin-bottom:8px' },
            h('label', { class: 'check grow' }, camToggle, 'Show my camera in a corner'),
            h('div', { style: 'width:150px' }, cornerSel)),
          recBtn, recTime]
        : h('p', { class: 'banner' }, firefox
          ? 'Firefox can’t make video replies iPhones will play. Use Chrome, Edge or Safari to record, or write a reply below.'
          : 'This browser can’t record video replies. Use Chrome, Edge or Safari, or write a reply below.'),
      result),
    h('div', { class: 'card' },
      h('h3', {}, 'Written reply'),
      writtenSnippets.field,
      h('div', { class: 'row', style: 'margin-top:8px' },
        writtenSnippets.button,
        h('button', { class: 'grow', onclick: (e) => busy(e.currentTarget, sendWritten) }, 'Send written reply'))),
  );
  requestAnimationFrame(frame);
  if (canRecord && pref.on) setCam(true); // on last time: back on (the browser remembers the permission)
}
