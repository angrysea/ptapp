// Cognito hosted login with the OAuth2 authorization-code + PKCE flow.
// Tokens live in localStorage so the app keeps working offline between gym
// sessions; the 30-day refresh token renews the 1-hour ID token when online.

import cfg from './config.js';

const KEY = 'pt.tokens';
const PKCE = 'pt.pkce';
const redirectUri = () => location.origin + location.pathname.replace(/index\.html$/, '');

const b64url = (buf) => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const random = (n = 32) => b64url(crypto.getRandomValues(new Uint8Array(n)));

export const devMode = () => !!cfg.devUser;
const stored = () => { try { return JSON.parse(localStorage.getItem(KEY)); } catch { return null; } };
const save = (t) => localStorage.setItem(KEY, JSON.stringify(t));

export function signedIn() {
  return devMode() ? !!localStorage.getItem('pt.devUser') : !!stored()?.refresh_token;
}

/** Opens Cognito's sign-in page; `loginHint` fills in the email there. */
export async function login({ loginHint } = {}) {
  if (devMode()) {
    const user = prompt('Dev user id (e.g. coach1, client1)', 'coach1');
    if (user) { localStorage.setItem('pt.devUser', user); location.reload(); }
    return;
  }
  const verifier = random(48);
  const state = random(16);
  const challenge = b64url(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier)));
  sessionStorage.setItem(PKCE, JSON.stringify({ verifier, state }));
  const q = new URLSearchParams({
    response_type: 'code', client_id: cfg.clientId, redirect_uri: redirectUri(),
    scope: 'openid email profile', state, code_challenge: challenge, code_challenge_method: 'S256',
    ...(loginHint && { login_hint: loginHint }),
  });
  location.assign(`${cfg.cognitoDomain}/oauth2/authorize?${q}`);
}

async function tokenRequest(params) {
  const res = await fetch(`${cfg.cognitoDomain}/oauth2/token`, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ client_id: cfg.clientId, ...params }),
  });
  if (!res.ok) throw Object.assign(new Error('token request failed'), { status: res.status });
  const t = await res.json();
  return { ...t, expires_at: Date.now() + t.expires_in * 1000 };
}

/** Completes the redirect back from Cognito, if this page load is one. */
export async function handleCallback() {
  const q = new URLSearchParams(location.search);
  if (!q.has('code')) return;
  const pkce = JSON.parse(sessionStorage.getItem(PKCE) || 'null');
  history.replaceState(null, '', redirectUri() + location.hash);
  if (!pkce || pkce.state !== q.get('state')) throw new Error('Login state mismatch; please sign in again.');
  sessionStorage.removeItem(PKCE);
  save(await tokenRequest({ grant_type: 'authorization_code', code: q.get('code'), redirect_uri: redirectUri(), code_verifier: pkce.verifier }));
}

let refreshing;
/** A valid ID token, refreshing if needed. Null when offline with an expired token. */
export async function idToken() {
  const t = stored();
  if (!t) return null;
  if (t.expires_at - 60_000 > Date.now()) return t.id_token;
  if (!navigator.onLine) return null;
  refreshing ||= tokenRequest({ grant_type: 'refresh_token', refresh_token: t.refresh_token })
    .then((n) => { save({ ...n, refresh_token: t.refresh_token }); return n.id_token; })
    .catch((e) => { if (e.status === 400) localStorage.removeItem(KEY); return null; })
    .finally(() => { refreshing = null; });
  return refreshing;
}

export function devUser() {
  return localStorage.getItem('pt.devUser');
}

export function logout() {
  localStorage.removeItem(KEY);
  localStorage.removeItem('pt.devUser');
  if (devMode() || !cfg.cognitoDomain) { location.assign(redirectUri()); return; }
  const q = new URLSearchParams({ client_id: cfg.clientId, logout_uri: redirectUri() });
  location.assign(`${cfg.cognitoDomain}/logout?${q}`);
}

// ---------------------------------------------------------------- sign-up (views/signup.js)
// The invitation sign-up page creates accounts with Cognito's user pool API
// directly (public app client, no secret), so it can fill in the invited
// email; sign-in still goes through Cognito's page.

const region = () => cfg.region || (cfg.cognitoDomain.match(/\.auth\.([a-z0-9-]+)\.amazoncognito\.com/) || [])[1];

async function userPool(action, body) {
  const res = await fetch(`https://cognito-idp.${region()}.amazonaws.com/`, {
    method: 'POST',
    headers: { 'content-type': 'application/x-amz-json-1.1', 'x-amz-target': `AWSCognitoIdentityProviderService.${action}` },
    body: JSON.stringify({ ClientId: cfg.clientId, ...body }),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw Object.assign(new Error(data.message || `Couldn’t reach the sign-in service (${res.status})`), { code: (data.__type || '').split('#').pop() });
  }
  return data;
}

export const signUp = (email, password) =>
  userPool('SignUp', { Username: email, Password: password, UserAttributes: [{ Name: 'email', Value: email }] });
export const confirmSignUp = (email, code) => userPool('ConfirmSignUp', { Username: email, ConfirmationCode: code });
export const resendSignUpCode = (email) => userPool('ResendConfirmationCode', { Username: email });

// Mirrors the user pool's password policy in template.yaml.
export const MIN_LENGTH = 10;
export const policyProblem = (pw) =>
  pw.length < MIN_LENGTH ? `Use at least ${MIN_LENGTH} characters.`
    : !/[a-z]/.test(pw) ? 'Include a lowercase letter.'
      : !/[0-9]/.test(pw) ? 'Include a number.'
        : null;

