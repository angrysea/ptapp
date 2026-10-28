# Implementation Plan: Real-time chat and push (MS-1, MS-2)

## Overview

Coach and client message each other in real time: a message appears on the other side within a second while the app is open (BRD performance NFR), and a push notification arrives when it isn't. Unread counts show on the client's **Coach** tab and on the coach's roster. Photos and voice notes can be sent as well as text.

This is Phase 1b, section 3 of `docs/ROADMAP.md`. Today's thread (`routes/comments.rs`, `web/js/views/comments.js`) is text only and refreshes only when reopened. Deep-link cards (MS-3), automated messages (MS-4), availability (MS-5) and group chat (MS-6) are later phases, but the design leaves room for them.

The video plans (`tasks/todo.md`, `tasks/reply/`) still have open items, so this plan lives in `tasks/chat/`.

## Decisions from review (2026-10-28)

- **Build in-house** (decision 1), with the plan as proposed.
- **Lock-screen previews:** show the sender and the first words; Profile gets a setting to hide the preview (open question 2).
- **Attachments:** photos and voice notes; no video messages (open question 3).
- **No email fallback and no quiet hours** for now; revisit after real use (open questions 4 and 5).

## Architecture Decisions

1. **Build, don't buy.** A managed chat SDK (Stream, Sendbird and the like) costs roughly $300–500 a month at its first paid tier, ten times the $50/month target for the whole app. It would also hold client health conversations outside our AWS account. One coach–client thread with no group chat is small enough to build: about two weeks of work.

2. **Messages stay where comments are now.** They are `COMMENT#{createdAt}#{id}` items in the client's partition, one thread per coach–client pair, so tenancy checks, audit and offline caching keep working. They gain:
   - `kind` (`text` | `photo` | `voice`) and `mediaKey`;
   - a **client-generated id**. Sending goes through the existing IndexedDB outbox and is idempotent (`put_new`), so a message typed offline is sent once when the connection returns, exactly like workout logs.

   Old comments read as `kind: text`. Comments anchored to a log or set (MS-3 partial) keep working.

3. **Real-time delivery: AWS AppSync Events, not a hand-built WebSocket API.** *Proposed; this changes the roadmap, and Task 1 proves it.*
   - **Setup:** each signed-in user subscribes to one channel, `/inbox/{userId}`, authenticated with their Cognito ID token. A subscribe handler (APPSYNC_JS, in the template) rejects any channel but the caller's own. Only the API Lambda can publish (IAM). After saving a message, the Lambda publishes it to the other person's channel.
   - **Compared with the roadmap's API Gateway WebSocket API**, there is no `$connect` Lambda, no hand-written JWT check, no connection table, no `PostToConnection` loop and no cleanup of dead connections.
   - **Cost:** $1 per million operations plus $0.08 per million connection-minutes, which is pennies at this size.
   - **Fallback if Task 1 fails:** the roadmap's WebSocket API. Tasks 3–4 put delivery behind one function (`notify(user, event)`), so the rest of the plan doesn't change.

4. **Real time is a hint, the database is the truth.**
   - An incoming event carries the message so it appears immediately. Missing one is harmless: the thread refetches on open, when the app returns to the foreground, and after reconnecting.
   - Locally and in the journey test (no AppSync), the page polls every few seconds instead, so the same code paths are tested.

5. **Unread counts are a counter per recipient and thread.** The item is `USER#{recipient}` / `UNREAD#{clientId}`, with `count` and `lastAt`.
   - Sending a message does an atomic `ADD count 1`; opening the thread sets it to 0 and stores `readAt`.
   - `GET /me/unread` returns all of a user's counts in one query: one badge for a client, one per roster row for a coach.
   - The other side's `readAt` gives a simple "Seen" under the last message.
   - The store needs a small `add` (update) helper; it only does put, get and delete today.

6. **Push: standard Web Push with VAPID, sent from the API Lambda.**
   - **Library:** the pure-Rust `web-push-native` crate (with `reqwest`/rustls), not the `web-push` crate. That one pulls in OpenSSL, which the arm64 Lambda cross-compile with zig avoids.
   - **Keys:** the VAPID key pair is generated once by `scripts/deploy.sh`. The private key is stored as an SSM SecureString, which is free, unlike Secrets Manager. The Lambda reads it at cold start, and the public key goes into `config.js`.
   - **Subscriptions:** stored as `USER#{id}` / `PUSH#{sha256(endpoint)}`. They are pruned when the push service answers 404 or 410.
   - **When:** a push goes to every subscription of the recipient on each new message. On iPhone, Safari requires each push to show a notification. Notifications are tagged per thread, so a burst of messages replaces one notification instead of stacking.
   - **What they say:** proposed is "{Name}: {first 80 characters}" for text and "{Name} sent a photo" otherwise (open question 2).

7. **iPhone push needs the app on the Home Screen** (iOS 16.4+), and permission must be asked from a tap.
   - **Where to turn it on:** Profile gets a "Notifications" switch, and after a user's first message a one-time card offers it.
   - **In Safari:** it explains Add to Home Screen instead, since push can't work there.
   - **Store apps:** Capacitor with APNs/FCM stays Phase 4.

8. **Attachments reuse existing upload paths.**
   - **Photos:** the progress-photo path (shrunk to 1600 px JPEG in the browser, presigned PUT).
   - **Voice notes:** `MediaRecorder`, capped at 2 minutes, uploaded with a single presigned PUT (they're small).
   - **Storage:** keys are `media/{coach}/{client}/messages/{id}.{ext}`, read through the same presigned GETs as photos.
   - **Video messages:** none; set videos and video replies already cover video (open question 3).

9. **Video replies notify through this.**
   - The reply editor's "add a comment to the thread" step (reply plan, decision 5) becomes a call to the same `post_message` function. A reply then arrives live, raises the unread count and sends a push without extra work.
   - Whichever plan lands second wires it up.

## Dependency Graph

```
Task 1 spike: AppSync Events channel ──┐
                                       ├── Task 3 messages API (ids, kinds, unread, notify)
Task 2 spike: Web Push from Lambda ──┐ │        │
                                     │ │        └── Task 4 live thread UI + badges
                                     │ │                 │
                                     └─┴── Task 5 push sending ── Task 6 push in the PWA
                                                         │
                                Task 7 photo messages ───┤ (need Task 4)
                                Task 8 voice notes ──────┤
                                                         └── Task 9 docs, costs, cleanup
```

## Task List

### Phase 0: De-risk
- [ ] Task 1: Spike: AppSync Events channel per user, Cognito auth, Lambda publishes
- [ ] Task 2: Spike: Web Push from the Rust Lambda to an iPhone Home Screen app and desktop Chrome

### Checkpoint: Transport decision
- [ ] Decision 3 (AppSync Events) and decision 6 (web-push-native) confirmed, or revised with the human

### Phase 1: Real-time text (MS-1)
- [ ] Task 3: Messages API: client ids, kinds, unread counters, real-time notify
- [ ] Task 4: Live thread in the app: incoming messages, offline send, unread badges, Seen

### Checkpoint: Chat works
- [ ] Two devices chat on dev with messages arriving in under 1 s; badges are correct

### Phase 2: Push (MS-2)
- [ ] Task 5: Push subscriptions and sending
- [ ] Task 6: Push in the PWA: turn on, show, open the thread

### Checkpoint: Push works
- [ ] A message to a closed app notifies on iPhone (Home Screen) and desktop; tapping opens the thread

### Phase 3: Attachments and finish
- [ ] Task 7: Photo messages
- [ ] Task 8: Voice notes
- [ ] Task 9: Docs, roadmap, cost note, cleanup

### Checkpoint: Complete
- [ ] All acceptance criteria met; CI green; deployed to dev; review with the human

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| AppSync Events blocked by the AWS Organization's SCPs (as GitHub OIDC was), or its subscribe handler can't check the Cognito user | High | Task 1 tests exactly this first, on the real account; fallback is the API Gateway WebSocket API (decision 3) |
| Web Push crypto or HTTP client doesn't cross-compile for arm64 Lambda | Med | Pure-Rust crate and rustls; Task 2 builds it with `cargo lambda` before anything depends on it |
| iPhone users never add the app to the Home Screen, so get no push | Med | Clear in-app hint; unread badge and email fallback (open question 4); Capacitor in Phase 4 |
| Safari revokes a subscription when a push shows no notification | Med | Every push shows a notification (decision 6) |
| Notifications show health details on a lock screen | Med | Short preview, setting to hide it (open question 2) |
| Real-time events missed (sleep, network change) | Low | Refetch on foreground and reconnect (decision 4) |
| moto can't emulate AppSync or push services | Low | Both behind small functions that are no-ops when not configured; e2e checks the stored state; manual checks on dev |

## Open Questions

1. **Build vs buy:** confirm building in-house (decision 1). A managed SDK would be faster to ship but costs far more each month.
2. **Lock-screen previews:** show the first words of a message, or only "New message from {name}"? Proposed: show a preview by default, with a Profile setting to hide it.
3. **Attachments in this phase:** photos and voice notes as proposed? Short video messages are left out, since set videos cover form checks.
4. **Email fallback:** should an unread message also be emailed after some time (for example, 12 hours unread and no push subscription)? SES is already set up. Proposed: not now; revisit after real use.
5. **Quiet hours:** should pushes be held overnight in the recipient's time zone (`Profile.timeZone`)? Proposed: not now; the phone's own Focus modes cover it.
