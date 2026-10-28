# Tasks: Real-time chat and push (MS-1, MS-2)

Plan and decisions: `tasks/chat/plan.md`. Each task: build, test, verify, commit.

Commands:
- Unit tests: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- e2e: `docker compose up -d --wait`, then `E2E_ENDPOINT=http://localhost:5000 cargo test -p api --test e2e`
- Journey: `python tests/ui/journey.py`
- Template: `cfn-lint template.yaml`
- Deploy: `scripts/deploy.sh` (all variables in docs/DEPLOY.md)

## Phase 0: De-risk

### Task 1: Spike: AppSync Events channel per user, Cognito auth, Lambda publishes

**Description:** Add an `AWS::AppSync::Api` (Event API) with Cognito user pool auth for subscribers and IAM auth for publishers, and an `inbox` channel namespace. Its `onSubscribe` handler allows only `/inbox/{ctx.identity.sub}`. Grant the API Lambda `appsync:EventPublish`. Add a temporary `POST /dev/ping` route that publishes `{type: "ping"}` to the caller's channel, signed with SigV4 from Rust. A throwaway spike page signs in, subscribes over the Event API's WebSocket, and shows what arrives.

**Acceptance criteria:**
- [ ] The stack deploys in this account (no SCP denial)
- [ ] A signed-in user receives their own ping in under 1 s
- [ ] Subscribing to another user's channel is refused
- [ ] Publishing without IAM (with only a user token) is refused

**Verification:**
- [ ] `cfn-lint template.yaml`; `cargo lambda build --release --arm64` succeeds with the signing code
- [ ] Manual: spike page on dev in Chrome and on iPhone Safari; results in `tasks/chat/plan.md`

**Dependencies:** None
**Files likely touched:** `template.yaml`, `crates/api/src/realtime.rs` (new), `crates/api/src/lib.rs`, `crates/api/Cargo.toml`, `web/spikes/chat.html` (temporary)
**Estimated scope:** M

### Task 2: Spike: Web Push from the Rust Lambda to an iPhone Home Screen app and desktop Chrome

**Description:**
- Generate a VAPID key pair; for the spike, the private key goes in an SSM SecureString created by hand.
- The spike page registers the service worker's push subscription and posts it to a temporary `POST /dev/push`. That route sends a test notification with `web-push-native` over `reqwest`/rustls.
- Add a minimal `push` handler to `web/sw.js`.

**Acceptance criteria:**
- [ ] The Lambda builds for arm64 with the push crates, without OpenSSL
- [ ] A notification arrives on desktop Chrome and on an iPhone with the app on the Home Screen
- [ ] An expired or unsubscribed endpoint returns 404/410, and the code can tell

**Verification:**
- [ ] Manual: the user's iPhone (Home Screen app) and a desktop browser; results in `tasks/chat/plan.md`

**Dependencies:** None (can run alongside Task 1)
**Files likely touched:** `crates/api/src/push.rs` (new), `crates/api/Cargo.toml`, `template.yaml`, `web/sw.js`, `web/spikes/chat.html`
**Estimated scope:** M

## Checkpoint: Transport decision
- [ ] Spike results recorded; decisions 3 and 6 confirmed or revised with the human
- [ ] Temporary `/dev/*` routes are removed once Tasks 3 and 5 replace them

## Phase 1: Real-time text (MS-1)

### Task 3: Messages API: client ids, kinds, unread counters, real-time notify

**Description:** Change the thread API in `routes/comments.rs` (paths stay the same):
- **Create** takes a client-generated `id` and writes it with `put_new`, so a retried send stores the message once.
- **Fields:** `kind` (default `text`) and `mediaKey`.
- **One send path:** a shared `post_message` saves the message, bumps the recipient's `UNREAD#{cid}` counter (a new store `add` helper doing an atomic update), and calls `realtime::notify(recipient, event)`. That call does nothing when no Event API is configured.
- **New routes:** `GET /me/unread` lists a user's counters. `POST /clients/{cid}/comments/read` zeroes the caller's counter and stores `readAt`.
- **List** returns the other side's `readAt`.

**Acceptance criteria:**
- [ ] Sending twice with the same id stores one message and counts one unread
- [ ] Unread counts are per recipient and per thread; reading clears only the reader's count
- [ ] Old comments without `kind` still list as text; log- and set-anchored comments still work
- [ ] Only the client and their coach can read, send or mark read (the existing scope check)

**Verification:**
- [ ] e2e: idempotent send, unread up and down for both sides, `readAt`, permissions (a second coach gets 403)
- [ ] Unit tests for the store `add` helper's request shape; clippy clean

**Dependencies:** Task 1 checkpoint
**Files likely touched:** `crates/api/src/routes/comments.rs`, `crates/api/src/realtime.rs`, `crates/api/src/store.rs`, `crates/core/src/model.rs`, `crates/api/src/lib.rs`, `crates/api/tests/e2e.rs`
**Estimated scope:** M

### Task 4: Live thread in the app: incoming messages, offline send, unread badges, Seen

**Description:**
- **`web/js/realtime.js` (new):** connects to the Event API with the ID token, subscribes to the user's inbox, reconnects with backoff and refreshes the token. Without an Event API (local, journey), it polls `/me/unread` every few seconds.
- **Thread:** shows incoming messages without a reload. Sending goes through the outbox with a client id, shows the message at once marked "Sending…", and marks it sent on success. "Seen" appears under the last own message once the other side has read it.
- **Badges:** the client's **Coach** tab and each roster row on the coach side show unread counts. Opening a thread marks it read.
- **Refetch:** on foreground (`visibilitychange`) and after a reconnect.

**Acceptance criteria:**
- [ ] With two signed-in devices on dev, a message shows on the other in under 1 s
- [ ] A message sent offline appears as "Sending…" and is delivered once when back online
- [ ] Badges are right after send, read, reload and switching devices

**Verification:**
- [ ] Journey test, two browser contexts (coach and client): a send shows on the other side without a reload (polling locally), the badge rises and clears, an offline send is delivered on reconnect
- [ ] Manual: phone and laptop on dev

**Dependencies:** Task 3
**Files likely touched:** `web/js/realtime.js` (new), `web/js/views/comments.js`, `web/js/app.js`, `web/js/views/coach.js`, `web/css/app.css`, `web/sw.js`, `scripts/deploy.sh` (Event API endpoint in `config.js`), `tests/ui/journey.py`
**Estimated scope:** M

## Checkpoint: Chat works
- [ ] All tests and CI green; deployed to dev
- [ ] Two devices chat on dev in real time; badges and Seen correct
- [ ] Review with the human

## Phase 2: Push (MS-2)

### Task 5: Push subscriptions and sending

**Description:**
- **Keys:** `scripts/deploy.sh` creates the VAPID key pair in SSM once, if missing, and writes the public key into `config.js`. The Lambda gets `ssm:GetParameter` on that parameter only.
- **Routes:** `POST /me/push` stores a subscription (`PUSH#{sha256(endpoint)}`) and `DELETE /me/push` removes one.
- **Sending:** `post_message` sends a push to each of the recipient's subscriptions, in parallel and best-effort: a failed push never fails the send. 404/410 deletes the subscription.
- **Payload:** sender name, preview text (decision 6, open question 2), thread id and a per-thread tag.

**Acceptance criteria:**
- [ ] Subscribing twice from one browser stores one subscription; signing out removes it
- [ ] A gone endpoint is pruned on the next send
- [ ] Message sends still succeed when push fails or isn't configured

**Verification:**
- [ ] e2e: subscribe, resubscribe and unsubscribe; send with push not configured
- [ ] Unit tests: payload building and the prune decision

**Dependencies:** Task 2 checkpoint, Task 3
**Files likely touched:** `crates/api/src/push.rs`, `crates/api/src/routes/me.rs`, `crates/api/src/routes/comments.rs`, `crates/api/src/lib.rs`, `template.yaml`, `scripts/deploy.sh`, `crates/api/tests/e2e.rs`
**Estimated scope:** M

### Task 6: Push in the PWA: turn on, show, open the thread

**Description:**
- **Service worker:** `push` shows the notification (always: Safari's rule). `notificationclick` focuses an open window or opens the app on the thread: `#/coach` for a client, the client's Messages tab for a coach. When a window is open, it tells the page to refetch.
- **Turning it on:** Profile gets a **Notifications** switch, and a one-time card after the user's first message offers it. In iPhone Safari outside the Home Screen app, it shows how to add the app to the Home Screen instead. A blocked permission shows how to allow it in settings.
- **Sign-out** unsubscribes.

**Acceptance criteria:**
- [ ] Turning on asks for permission only from a tap; turning off unsubscribes
- [ ] Tapping a notification opens the right thread, whether the app was closed or open
- [ ] iPhone Safari (not installed) shows the Home Screen hint, not a broken switch

**Verification:**
- [ ] Journey test: the switch with a granted permission stub posts a subscription; the SW `notificationclick` route mapping is unit-checked in the page
- [ ] Manual: iPhone Home Screen app and desktop Chrome, app closed and open

**Dependencies:** Task 5
**Files likely touched:** `web/sw.js`, `web/js/push.js` (new), `web/js/views/profile.js`, `web/js/views/comments.js`, `web/js/auth.js`, `tests/ui/journey.py`
**Estimated scope:** M

## Checkpoint: Push works
- [ ] All tests and CI green; deployed to dev
- [ ] A message to a closed app notifies on iPhone (Home Screen) and desktop, and opens the thread
- [ ] Review with the human

## Phase 3: Attachments and finish

### Task 7: Photo messages

**Description:**
- **Sending:** a photo button in the thread composer picks or takes a photo, shrinks it to 1600 px JPEG (as progress photos do), uploads it with a presigned PUT, then sends a `kind: photo` message with its `mediaKey`.
- **Showing:** the thread shows a thumbnail that opens full size, using presigned GETs as photos do.
- **Offline:** the upload waits in the outbox like any send.

**Acceptance criteria:**
- [ ] A photo sent from a phone shows on the other side live and in the notification text
- [ ] Only the two people in the thread can fetch it (presigned through the scope check)

**Verification:**
- [ ] e2e: create a photo message (presign returned, key under the client's prefix), another coach gets 403
- [ ] Journey test: send a small image; it renders in both threads

**Dependencies:** Task 4
**Files likely touched:** `crates/api/src/routes/comments.rs`, `web/js/views/comments.js`, `web/css/app.css`, `crates/api/tests/e2e.rs`, `tests/ui/journey.py`
**Estimated scope:** M

### Task 8: Voice notes

**Description:**
- **Recording:** hold, or tap to start and stop, to record from the microphone with `MediaRecorder`: MP4/AAC where supported (Safari, Chrome) and WebM/Opus otherwise. It is capped at 2 minutes and the length is shown.
- **Sending:** uploads like a photo, as `kind: voice`.
- **Playing:** messages play inline with their duration. A denied microphone shows a clear message.

**Acceptance criteria:**
- [ ] A voice note recorded on a laptop plays on iPhone, and the other way round
- [ ] Recording stops at 2 minutes; cancel discards it

**Verification:**
- [ ] Journey test with a fake microphone: record about 2 s, send, it plays on the other side
- [ ] Manual: laptop ↔ iPhone

**Dependencies:** Task 4 (Task 7 for the shared upload code)
**Files likely touched:** `web/js/views/comments.js`, `web/js/voice.js` (new), `crates/api/src/routes/comments.rs`, `web/css/app.css`, `tests/ui/journey.py`
**Estimated scope:** M

### Task 9: Docs, roadmap, cost note, cleanup

**Description:**
- **Docs:** update `docs/ARCHITECTURE.md` (real-time and push, data model items, costs), `docs/DEPLOY.md` (VAPID setup, Event API outputs) and `docs/ROADMAP.md` (MS-1 and MS-2 done, the decision 3 change).
- **Retention:** message media follows the media bucket's lifecycle.
- **Cleanup:** delete the spike page and `/dev/*` routes. If the video reply editor has shipped, route its thread comment through `post_message` (decision 9).

**Acceptance criteria:**
- [ ] Docs describe what was built; no spike code remains
- [ ] `cfn-lint`, clippy, e2e and journey all green

**Verification:**
- [ ] CI green; deployed to dev; smoke check passes

**Dependencies:** Tasks 5–8
**Files likely touched:** `docs/*.md`, `template.yaml`, `web/spikes/chat.html` (delete), `crates/api/src/lib.rs`
**Estimated scope:** S

## Checkpoint: Complete
- [ ] All acceptance criteria met; CI green; deployed to dev
- [ ] Review with the human
