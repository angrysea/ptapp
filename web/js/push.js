// Push notifications for this device (MS-2). Turning them on asks the
// browser for permission (only ever from a tap) and subscribes with the
// API's VAPID key; the server stores the subscription and notifies it when a
// message or video reply arrives (routes/notify.rs). The service worker shows
// the notification and opens the conversation when it's tapped (sw.js).
// iPhones and iPads only allow push in the app added to the Home Screen.

import { api, get, post } from './api.js';

const PREVIEW = 'pt.push.preview';
const DISMISSED = 'pt.push.promptDismissed';

export const supported = () => 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window;
const ios = () => /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const standalone = () => matchMedia('(display-mode: standalone)').matches || navigator.standalone === true;
/** On an iPhone or iPad outside the Home Screen app, where push can't work. */
export const needsHomeScreen = () => ios() && !standalone();

const stored = (k, fallback) => { try { return localStorage.getItem(k) ?? fallback; } catch { return fallback; } };
const store = (k, v) => { try { localStorage.setItem(k, v); } catch { /* not remembered */ } };
/** Whether notifications show the message text (else just "New message"). */
export const previewOn = () => stored(PREVIEW, '1') !== '0';

function keyBytes(b64) {
  const s = atob(b64.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (b64.length % 4)) % 4));
  return Uint8Array.from(s, (c) => c.charCodeAt(0));
}

// Fetched ahead of time: Safari wants subscribe() close to the tap that asked.
let keyPromise = null;
export function prefetchKey() {
  keyPromise ||= get('/push/key').then((r) => r.publicKey).catch(() => { keyPromise = null; return null; });
  return keyPromise;
}

/** The service worker, or null if it isn't running within 3 s (`ready`
 * never settles where service workers are off, e.g. some private windows). */
const worker = () => Promise.race([navigator.serviceWorker.ready, new Promise((r) => setTimeout(() => r(null), 3000))]);

export async function currentSubscription() {
  if (!supported()) return null;
  const reg = await worker();
  return reg ? reg.pushManager.getSubscription() : null;
}

/** 'on', 'off', 'denied' (blocked in the browser) or 'unavailable'. */
export async function status() {
  if (!supported()) return 'unavailable';
  if (Notification.permission === 'denied') return 'denied';
  return Notification.permission === 'granted' && (await currentSubscription()) ? 'on' : 'off';
}

const send = (sub, preview) => post('/me/push', { ...sub.toJSON(), preview });

/** Turns notifications on; call it from a tap. Returns what `status` would. */
export async function enable() {
  if (!supported()) return 'unavailable';
  const permission = await Notification.requestPermission();
  if (permission !== 'granted') return permission === 'denied' ? 'denied' : 'off';
  const publicKey = await prefetchKey();
  if (!publicKey) return 'unavailable';
  const reg = await worker();
  if (!reg) return 'unavailable';
  const sub = (await reg.pushManager.getSubscription()) || await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(publicKey) });
  await send(sub, previewOn());
  return 'on';
}

/** Turns notifications off on this device (also on sign-out). */
export async function disable() {
  const sub = await currentSubscription().catch(() => null);
  if (!sub) return;
  await api('DELETE', '/me/push', { endpoint: sub.endpoint }).catch(() => {}); // offline: the server prunes it later
  await sub.unsubscribe().catch(() => {});
}

export async function setPreview(on) {
  store(PREVIEW, on ? '1' : '0');
  const sub = await currentSubscription();
  if (sub && Notification.permission === 'granted') await send(sub, on);
}

/** At start-up: make sure the server has this device's subscription. */
export async function resync() {
  if (!supported() || Notification.permission !== 'granted') return;
  const sub = await currentSubscription().catch(() => null);
  if (sub) send(sub, previewOn()).catch(() => {});
}

/** Whether to offer notifications in a conversation (once, until dismissed). */
export async function shouldPrompt() {
  return supported() && Notification.permission === 'default' && stored(DISMISSED, '0') !== '1' && (await status()) === 'off';
}
export const dismissPrompt = () => store(DISMISSED, '1');
