// VS-2: trim and re-encode a set video on the device, so only the selected
// part uploads, as an H.264 MP4 every browser plays, with a poster frame.
// Decisions and device results: tasks/plan.md (Spike Results).
//
// Pipeline: mp4box demuxes the MP4/MOV; WebCodecs decodes from the keyframe
// before `start`, frames in range are drawn (rotated, scaled) to a canvas and
// re-encoded as H.264; AAC audio in range is copied without re-encoding;
// mp4-muxer writes the MP4. Anything this browser can't do throws
// `CannotEncode`, and the caller uploads the original with trim points.

const VENDOR = ['vendor/mp4box-0.5.2.all.min.js', 'vendor/mp4-muxer-5.1.3.js'];

/** 720p by default: enough to judge form, and small and quick on a phone. */
const LONG_SIDE = 1280;
const BITRATE = 2_500_000;
const CODECS = ['avc1.64001f', 'avc1.4d001f', 'avc1.42e01f']; // High, Main, Baseline at level 3.1

/** This browser can't convert this clip; upload the original instead. */
export class CannotEncode extends Error {
  constructor(message) { super(message); this.name = 'CannotEncode'; }
}

/** Tests set localStorage `pt.forceVideoFallback` to "1" to take the upload-the-original path. */
const forcedFallback = () => { try { return localStorage.getItem('pt.forceVideoFallback') === '1'; } catch { return false; } };

export const canEncode = () => !forcedFallback() && 'VideoDecoder' in window && 'VideoEncoder' in window && 'EncodedVideoChunk' in window;

let libs;
function loadLibs() {
  libs ||= Promise.all(VENDOR.map((src) => new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = src;
    s.onload = resolve;
    s.onerror = () => reject(new CannotEncode(`could not load ${src}`));
    document.head.append(s);
  })));
  return libs;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const abortError = () => new DOMException('Conversion cancelled', 'AbortError');

function demux(file) {
  return new Promise((resolve, reject) => {
    const mp4 = MP4Box.createFile();
    const samples = {};
    let info = null;
    mp4.onError = (e) => reject(new CannotEncode(`not a readable MP4/MOV: ${e}`));
    mp4.onReady = (i) => {
      info = i;
      for (const t of i.tracks) { samples[t.id] = []; mp4.setExtractionOptions(t.id, null, { nbSamples: Infinity }); }
      mp4.onSamples = (id, _u, s) => { samples[id].push(...s); };
      mp4.start();
    };
    file.arrayBuffer().then((buf) => {
      buf.fileStart = 0;
      mp4.appendBuffer(buf);
      mp4.flush();
      // Extraction runs synchronously on a fully appended buffer.
      setTimeout(() => (info ? resolve({ mp4, info, samples }) : reject(new CannotEncode('no MP4/MOV structure (e.g. a WebM file)'))), 0);
    }, reject);
  });
}

function videoDescription(mp4, track) {
  for (const entry of mp4.getTrackById(track.id).mdia.minf.stbl.stsd.entries) {
    const box = entry.avcC || entry.hvcC || entry.vpcC || entry.av1C;
    if (box) {
      const s = new DataStream(undefined, 0, DataStream.BIG_ENDIAN);
      box.write(s);
      return new Uint8Array(s.buffer, 8); // without the box header
    }
  }
  return undefined;
}

/** AAC decoder config. MP4 files keep it in mp4a › esds, which mp4box parses.
 *  iPhone .MOV files nest it as mp4a › wave › esds, which mp4box doesn't, so an
 *  AAC-LC config (what iPhones record) is derived from rate and channels. */
function aacConfig(mp4, track) {
  try {
    const d = mp4.getTrackById(track.id).mdia.minf.stbl.stsd.entries[0].esds.esd.descs[0].descs[0].data;
    if (d?.length) return d;
  } catch { /* nested or missing: derive below */ }
  const RATES = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
  const idx = RATES.indexOf(track.audio.sample_rate);
  if (idx < 0) return undefined;
  const v = (2 << 11) | (idx << 7) | (track.audio.channel_count << 3);
  return new Uint8Array([v >> 8, v & 255]);
}

/** Display rotation in degrees (phones store portrait video as rotated landscape). */
function rotationOf(track) {
  const m = track.matrix;
  if (!m) return 0;
  const deg = Math.round((Math.atan2(m[1], m[0]) * 180) / Math.PI);
  return ((deg % 360) + 360) % 360;
}

function drawRotated(ctx, frame, rotation, W, H) {
  ctx.save();
  if (rotation === 90) { ctx.translate(W, 0); ctx.rotate(Math.PI / 2); ctx.drawImage(frame, 0, 0, H, W); }
  else if (rotation === 180) { ctx.translate(W, H); ctx.rotate(Math.PI); ctx.drawImage(frame, 0, 0, W, H); }
  else if (rotation === 270) { ctx.translate(0, H); ctx.rotate(-Math.PI / 2); ctx.drawImage(frame, 0, 0, H, W); }
  else ctx.drawImage(frame, 0, 0, W, H);
  ctx.restore();
}

async function pickEncoder(width, height, framerate) {
  for (const codec of CODECS) {
    const cfg = { codec, width, height, bitrate: BITRATE, framerate, avc: { format: 'avc' }, latencyMode: 'quality' };
    try { if ((await VideoEncoder.isConfigSupported(cfg)).supported) return cfg; } catch { /* try the next profile */ }
  }
  return null;
}

/**
 * Converts `start`..`end` (seconds) of `file` to an H.264 MP4 at most 720p.
 * Resolves `{ blob, poster, durationSec, width, height }`; `poster` is a JPEG
 * of the first frame. Throws `CannotEncode` when this browser can't, or an
 * `AbortError` if `signal` aborts.
 */
export async function encodeClip(file, { start, end, onProgress, signal }) {
  if (!canEncode()) throw new CannotEncode('this browser has no WebCodecs');
  await loadLibs();
  const { mp4, info, samples } = await demux(file);
  const vt = info.videoTracks[0], at = info.audioTracks[0];
  if (!vt) throw new CannotEncode('no video track');
  const fps = vt.nb_samples / (vt.duration / vt.timescale) || 30;
  const description = videoDescription(mp4, vt);
  const decodeCfg = { codec: vt.codec, codedWidth: vt.video.width, codedHeight: vt.video.height, description };
  if (!(await VideoDecoder.isConfigSupported(decodeCfg).catch(() => ({}))).supported) {
    throw new CannotEncode(`can't decode ${vt.codec} here`);
  }

  const rotation = rotationOf(vt);
  const sideways = rotation % 180 !== 0;
  const srcW = sideways ? vt.video.height : vt.video.width;
  const srcH = sideways ? vt.video.width : vt.video.height;
  const scale = Math.min(1, LONG_SIDE / Math.max(srcW, srcH));
  const even = (n) => Math.max(2, Math.round((n * scale) / 2) * 2);
  const width = even(srcW), height = even(srcH);
  const cfg = await pickEncoder(width, height, Math.round(fps));
  if (!cfg) throw new CannotEncode(`no H.264 encoder for ${width}×${height}`);

  const startUs = start * 1e6, endUs = end * 1e6;
  const ts = (s, track) => (s.cts / track.timescale) * 1e6;
  const asc = at && at.codec.startsWith('mp4a') ? aacConfig(mp4, at) : undefined;
  const muxer = new Mp4Muxer.Muxer({
    target: new Mp4Muxer.ArrayBufferTarget(),
    video: { codec: 'avc', width, height },
    audio: asc ? { codec: 'aac', numberOfChannels: at.audio.channel_count, sampleRate: at.audio.sample_rate } : undefined,
    fastStart: 'in-memory',
    firstTimestampBehavior: 'offset',
  });

  const canvas = document.createElement('canvas');
  canvas.width = width; canvas.height = height;
  const ctx = canvas.getContext('2d');
  let failure = null, frames = 0, poster = null;
  const expected = Math.max(1, Math.round((end - start) * fps));
  const gop = Math.max(1, Math.round(fps * 2)); // a keyframe every 2 s, for seeking

  const encoder = new VideoEncoder({
    output: (chunk, meta) => muxer.addVideoChunk(chunk, meta),
    error: (e) => { failure ||= e; },
  });
  encoder.configure(cfg);
  const decoder = new VideoDecoder({
    output: (frame) => {
      try {
        if (failure || frame.timestamp < startUs || frame.timestamp > endUs) return;
        drawRotated(ctx, frame, rotation, width, height);
        const vf = new VideoFrame(canvas, { timestamp: frame.timestamp - startUs, duration: frame.duration || undefined });
        encoder.encode(vf, { keyFrame: frames % gop === 0 });
        vf.close();
        if (frames === 0) poster = new Promise((r) => canvas.toBlob(r, 'image/jpeg', 0.8));
        frames++;
        onProgress?.(Math.min(1, frames / expected));
      } finally { frame.close(); }
    },
    error: (e) => { failure ||= e; },
  });
  decoder.configure(decodeCfg);

  try {
    // Decode from the last keyframe at or before `start`.
    const vs = samples[vt.id];
    let first = 0;
    for (let k = 0; k < vs.length; k++) if (vs[k].is_sync && ts(vs[k], vt) <= startUs) first = k;
    for (let k = first; k < vs.length; k++) {
      if (signal?.aborted) throw abortError();
      if (failure) break;
      const s = vs[k];
      if (s.is_sync && ts(s, vt) > endUs) break;
      decoder.decode(new EncodedVideoChunk({ type: s.is_sync ? 'key' : 'delta', timestamp: ts(s, vt), duration: (s.duration / vt.timescale) * 1e6, data: s.data }));
      while ((decoder.decodeQueueSize > 8 || encoder.encodeQueueSize > 8) && !failure && !signal?.aborted) await sleep(2);
    }
    if (signal?.aborted) throw abortError();
    if (!failure) { await decoder.flush(); await encoder.flush(); }
  } finally {
    try { decoder.close(); } catch { /* already closed */ }
    try { encoder.close(); } catch { /* already closed */ }
  }
  if (failure) throw new CannotEncode(`conversion failed: ${failure.message || failure}`);
  if (frames === 0) throw new CannotEncode('no frames in the selected range');

  // AAC frames are independent, so the ones in range are copied as they are.
  if (asc) {
    let firstAudio = true;
    for (const s of samples[at.id]) {
      const t = ts(s, at);
      if (t < startUs || t > endUs) continue;
      const meta = firstAudio ? { decoderConfig: { codec: /^mp4a\.\d/.test(at.codec) ? at.codec : 'mp4a.40.2', numberOfChannels: at.audio.channel_count, sampleRate: at.audio.sample_rate, description: asc } } : undefined;
      muxer.addAudioChunkRaw(s.data, 'key', t - startUs, (s.duration / at.timescale) * 1e6, meta);
      firstAudio = false;
    }
  }
  muxer.finalize();
  return {
    blob: new Blob([muxer.target.buffer], { type: 'video/mp4' }),
    poster: poster ? await poster : null,
    durationSec: Math.round((end - start) * 10) / 10,
    width,
    height,
  };
}
