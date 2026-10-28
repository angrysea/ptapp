// In-app recording (VS-1): film a set at 720p without leaving the app. On
// iOS a page's `<input capture>` records at 480×360 (tasks/plan.md, Spike
// Results), so the camera is opened directly and recorded with MediaRecorder.

import { h, sheet } from '../ui.js';
import { MAX_CLIP_SECONDS } from './trim.js';

// MP4 first: Safari and recent Chrome record H.264/AAC, which the trim and
// convert steps handle directly. WebM is the last resort.
const TYPES = ['video/mp4;codecs=avc1.42E01E,mp4a.40.2', 'video/mp4', 'video/webm;codecs=vp9,opus', 'video/webm;codecs=vp8,opus', 'video/webm'];

export const canRecord = () => !!(navigator.mediaDevices?.getUserMedia && window.MediaRecorder);

const clock = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

function cameraError(e) {
  if (e.name === 'NotAllowedError') {
    return 'Camera access is off for this site. Allow the camera and microphone in your browser’s site settings '
      + '(on iPhone: Settings › Apps › Safari › Camera and Microphone), or choose a clip from your library.';
  }
  if (e.name === 'NotFoundError') return 'No camera was found. Choose a clip from your library instead.';
  return `The camera couldn’t start (${e.message}). Choose a clip from your library instead.`;
}

/**
 * Opens a camera sheet and records one clip of at most 60 s. Resolves
 * `{ file, durationSec }` when the person stops (or at 60 s), or null if
 * they cancel. `durationSec` is measured while recording, because some
 * browsers' recordings don't report their own length.
 */
export function recordClip() {
  return new Promise((resolve) => {
    let stream = null, recorder = null, started = 0, tick = null, done = false;
    let facing = 'environment';
    const preview = h('video', { class: 'player', playsinline: true, autoplay: true });
    preview.muted = true;
    const time = h('span', { class: 'rec-time' }, `0:00 / ${clock(MAX_CLIP_SECONDS)}`);
    const msg = h('p', { class: 'small muted', 'aria-live': 'polite' }, 'Starting the camera…');
    const rec = h('button', { class: 'primary grow', type: 'button', disabled: true }, '● Record');
    const flip = h('button', { class: 'grow', type: 'button', hidden: true }, '⟲ Switch camera');
    const sh = sheet(
      h('h2', {}, 'Record a set'),
      preview,
      time,
      msg,
      h('div', { class: 'row' }, flip, rec),
      h('button', { class: 'ghost block', type: 'button', onclick: () => sh.close() }, 'Cancel'),
    );

    const stopCamera = () => { stream?.getTracks().forEach((t) => t.stop()); stream = null; };
    const finish = (result) => {
      if (done) return;
      done = true;
      clearInterval(tick);
      stopCamera();
      resolve(result);
    };
    sh.onClose = () => {
      if (recorder?.state === 'recording') { recorder.onstop = null; recorder.stop(); }
      finish(null);
    };

    async function openCamera() {
      stopCamera();
      rec.disabled = true;
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: { ideal: facing }, width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } },
          audio: true,
        });
      } catch (e) {
        msg.textContent = cameraError(e);
        return;
      }
      if (done) return stopCamera(); // cancelled while the permission prompt was up
      preview.srcObject = stream;
      preview.play().catch(() => {});
      const s = stream.getVideoTracks()[0]?.getSettings?.() || {};
      msg.textContent = s.width ? `Camera ready · ${s.width}×${s.height}` : 'Camera ready';
      rec.disabled = false;
      const cams = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'videoinput');
      flip.hidden = cams.length < 2;
    }

    flip.addEventListener('click', () => { facing = facing === 'environment' ? 'user' : 'environment'; openCamera(); });
    rec.addEventListener('click', () => {
      if (recorder?.state === 'recording') { recorder.stop(); return; }
      const mimeType = TYPES.find((t) => MediaRecorder.isTypeSupported?.(t)) || '';
      try {
        recorder = new MediaRecorder(stream, { ...(mimeType && { mimeType }), videoBitsPerSecond: 2_500_000 });
      } catch (e) {
        msg.textContent = `This browser can’t record video (${e.message}). Choose a clip from your library instead.`;
        return;
      }
      const chunks = [];
      recorder.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data); };
      recorder.onstop = () => {
        const durationSec = Math.min(MAX_CLIP_SECONDS, (performance.now() - started) / 1000);
        const type = (recorder.mimeType || mimeType || 'video/mp4').split(';')[0];
        finish({ file: new File(chunks, `recording.${type.includes('webm') ? 'webm' : 'mp4'}`, { type }), durationSec });
        sh.close();
      };
      recorder.start(1000);
      started = performance.now();
      flip.hidden = true;
      rec.textContent = '■ Stop';
      msg.textContent = 'Recording. It stops by itself at 1:00.';
      tick = setInterval(() => {
        const s = (performance.now() - started) / 1000;
        time.textContent = `${clock(Math.min(s, MAX_CLIP_SECONDS))} / ${clock(MAX_CLIP_SECONDS)}`;
        if (s >= MAX_CLIP_SECONDS && recorder.state === 'recording') recorder.stop();
      }, 250);
    });

    if (canRecord()) openCamera();
    else msg.textContent = 'This browser can’t record video. Choose a clip from your library instead.';
  });
}
