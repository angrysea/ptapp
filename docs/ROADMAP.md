# Roadmap: implementing the rest of the BRD

Every remaining requirement, in build order, with concrete steps for **this** codebase. Each feature follows the same recipe:

1. domain types and rules in `crates/core` (+ unit tests)
2. key patterns in `store.rs`; a route in `crates/api/src/routes/*`; registration in `lib.rs`; steps in `tests/e2e.rs`
3. a view in `web/js/views/*`, its route in `app.js`, and the file added to `SHELL` in `sw.js`
4. infrastructure in `template.yaml`; extend `tests/ui/journey.py`; CI (`.github/workflows/ci.yml`) then runs clippy, tests, `cfn-lint` and both journeys on the pull request and push; deploy to dev with `scripts/deploy.sh` (DEPLOY.md)

Patterns already in place to reuse rather than rebuild:

* **Uploads:** `util::presign_put` signs `Content-Length` (and any tag) into the URL, so S3 enforces the size the API approved; the response's `uploadHeaders` must be sent as-is. Uploads that need a follow-up step are tagged `state=pending` (`util::PENDING_TAG`) and confirmed by the API; a lifecycle rule expires unconfirmed ones after 2 days (see the exercise demo `demo-upload` / `demo-confirm` pair).
* **Local dates:** `Profile.timeZone` (sent by the app at startup) and `Profile::today()` / `ClientScope::today()` give a user's calendar date. Anything "per day" or "per week" (missed workouts, reminders, check-ins) uses these, never UTC.
* **Item size:** every write goes through `store::keyed`, which turns items over ~350 KB into a 400.
* **Offline writes:** the outbox in `web/js/api.js` is per user; use `enqueue(..., key)` with a stable key for edits that should replace each other.

Status legend: ✅ done in MVP · 🟡 partial · ⬜ not started

## Requirement status

| Area | Must (MVP) | Should / Could |
|---|---|---|
| Onboarding | ON-1 ✅ (emailed with SES, bound to the invited verified email) · ON-2 ✅ · ON-3 ✅ · coach profiles + public coach directory ✅ (`tasks/coaches`; tax IDs left to Stripe in BZ-1) | ON-4 ⬜ · ON-5 ⬜ |
| Builder | WB-1 ✅ · WB-2 ✅ · WB-3 ✅ · WB-4 ✅ | WB-5 ⬜ · WB-6 ⬜ · WB-7 ⬜ |
| Logging | LG-1 ✅ · LG-2 ✅ · LG-3 ✅ · LG-4 ✅ · LG-6 ✅ | LG-5 🟡 (e1RM trend; no PR events) · LG-7 🟡 (roster flags; no alerts) · LG-8 ⬜ |
| Video submission | VS-1 ✅ · VS-2 ✅ · VS-3 ✅ · VS-4 ✅ (real-device checks pending, see `tasks/todo.md`) | VS-5 ⬜ |
| Video reply | VR-1 ✅ · VR-2 ✅ · VR-3 ✅ (recorded in the browser, `tasks/reply`) | VR-4…VR-7 ⬜ |
| Feedback | FB-1 ✅ · FB-2 ✅ | FB-3 ⬜ · FB-4 ⬜ |
| AI | AI-6/AI-7 (whenever AI ships) | AI-1…AI-5 ⬜ |
| Body metrics | BM-1 ✅ · BM-2 ✅ · BM-3 ✅ · BM-4 ✅ · BM-7 ✅ · BM-8 ✅ | BM-5 ✅ · BM-6 ⬜ |
| Messaging | MS-1 ✅ (live via AppSync Events, polling fallback; coach inbox; text only) · MS-2 ✅ (Web Push; iPhone needs the Home Screen app) | MS-3 🟡 (comments anchor to log/set) · MS-4 ⬜ · MS-5 ⬜ · MS-6 ⬜ |
| Business | BZ-1 ⬜ | BZ-2 ⬜ · BZ-3 ⬜ · BZ-4 ⬜ · BZ-5 ⬜ · BZ-6 ⬜ |
| NFRs | privacy ✅ (private photos, tenant checks, per-user offline cache) · audit log ✅ · offline ✅ · upload size limits ✅ · CI ✅ (tests on every push; deploys run locally) · backups ✅ (PITR; restore tested 2026-10-28, see DEPLOY.md) · MFA 🟡 (Cognito TOTP optional; in-app enrollment needs the user.admin scope, removed for safety) · alarms ✅ (email) · export/delete ✅ (clients, self-service; coaches via admin) · video retention ✅ (lifecycle on reviewed videos and replies) | |

---

## Phase 1b: complete the BRD MVP (≈ 5-7 weeks)

The BRD's Phase 1 also includes video, chat and payments. Build in this order: video is the differentiator and the biggest risk, so prototype it first (BRD risk #1).

### 1. Exercise video submission (VS-1 … VS-4) ✅

Built per `tasks/plan.md`. It departs from the original design here in favour of less infrastructure, after a spike on real devices (results in the plan):

* **Conversion happens on the device, not in an ffmpeg Lambda.** `web/js/encode.js` demuxes with mp4box, decodes and re-encodes the trimmed range with WebCodecs to H.264 (up to 720p, 2.5 Mbps), copies AAC audio and makes a JPEG poster; mp4-muxer writes the MP4 (libraries vendored in `web/vendor/`). An iPhone converted a 1080p clip at 2.3× real time. When a browser can't convert, the original uploads with trim points and the player keeps to them. No SQS, container image or S3 event triggers were needed. Server-side transcoding stays an option if HEVC on some devices turns out to need it.
* **Recording in the app** (`web/js/views/recorder.js`, `getUserMedia` + `MediaRecorder` at 720p, stops at 60 s), because iOS's `<input capture>` hands pages a 480×360 recording.
* **Resumable uploads** as planned: S3 multipart, 8 MiB parts each presigned for its exact size, tagged pending until completed; the device keeps the clip and part ETags in IndexedDB (`web/js/uploads.js`).
* **The review queue is a per-coach item** (`COACH#` / `REVIEW#{createdAt}#{video}`) instead of a GSI; a prefix query is oldest first.
* **Playback uses presigned S3 GETs.** CloudFront signed URLs (cheaper egress) move to the reply editor below, where heavy playback starts.
* **Retention:** reviewed videos are tagged and deleted by a lifecycle rule `VideoRetentionDays` (template parameter, default 180) after upload.

Still open: real-device checks (iPhone Safari and the home-screen app, an HEVC clip, Android), the cellular-data warning before a large original uploads, and VS-5 filming guidance.

**Exit test:** a 60 s phone clip on throttled 3G resumes after airplane mode; the coach queue shows it oldest first. The resume and queue order are covered by the browser journey; the throttled-3G run on a phone is still to do.

### 2. Coach video reply editor (VR-1 … VR-3)

Use the server-side render approach from the BRD: the browser records an **edit list**, and FFmpeg renders it.

* **Player (VR-1):** `<video>` with `playbackRate` 0.25/0.5/1, frame stepping via `requestVideoFrameCallback` (step = 1/fps; seek `currentTime ± 1/fps`), and keyboard shortcuts (`,` `.` `space`).
* **Segments (VR-3):** the coach marks in/out ranges; the list can be reordered by drag.
* **Voice-over (VR-2):** `MediaRecorder` on the microphone (`audio/webm;codecs=opus`). While recording, log a timeline of events `{t, action: play|pause|seek|rate, value}` against the voice clock. Drawing events are recorded the same way in Phase 2.
* Upload the audio (presigned PUT) and `POST /videos/{id}/replies` with `{segments, events, audioKey}` → `REPLY#{id}` in status rendering → SQS `RenderQueue`.
* **Render worker** (same ffmpeg container, separate handler): replay the event list into a sequence of source sub-clips with speed changes (`setpts`), freeze frames for pauses (`tpad`/`loop`), concatenated (`concat` filter) and muxed with the voice track (`amix`, ducking original audio). Output 720p H.264 MP4 + VTT.
* **Captions** (accessibility NFR): transcribe the voice track with Amazon Transcribe, or locally with whisper.cpp on the Phase 3 AI box, to produce a `.vtt`.
* The reply shows in the client's thread and on the workout log; the client is notified (see 3).

**Exit gate from BRD:** coach reviews a video in under 5 minutes end to end. Measure from queue open to reply sent.

### 3. Real-time chat + push (MS-1, MS-2)

* **Buy vs build:** the BRD notes a managed chat SDK saves weeks. If budget allows, use one. The in-house path on this stack:
* `AWS::ApiGatewayV2::Api` with `ProtocolType: WEBSOCKET` and a Rust Lambda (`$connect` validates the Cognito JWT from a query param, storing `CONN#{connectionId}` and `USERCONN#{user}` items with TTL; `$disconnect` removes them).
* Comments become messages: add `kind: text|photo|video|voice` and `mediaKey`, plus a `READ#{user}` marker item for read receipts and unread counts (`lastReadAt` per thread).
* `POST /clients/{cid}/comments` writes the item, then fans out with `PostToConnection` to the other party's connections (under 1 s, per the BRD performance NFR).
* **Push (MS-2):** Web Push with VAPID keys in Secrets Manager. The PWA subscribes via `PushManager` and stores the subscription in `PUSH#{endpointHash}`. Send with the `web-push` protocol from Rust (`web-push` crate) when the recipient has no live connection. iOS needs the PWA installed to the Home Screen (iOS 16.4+); for full parity, wrap the PWA in **Capacitor** (Phase 4) and use APNs/FCM through SNS.
* Voice notes: `MediaRecorder` → presigned PUT → `kind: voice`.

### 4. Stripe subscriptions (BZ-1)

* Coach config: `COACH#{coach}/PLAN#{id}` with name, price and trial days, mirrored to a Stripe Product/Price. Use **Stripe Connect (Express)** so client payments go to the coach's account, with an optional application fee as your SaaS revenue.
* `POST /billing/checkout` (client) → Stripe Checkout Session (mode `subscription`, `trial_period_days`) → redirect.
* `POST /billing/portal` → Billing Portal session (cancel, update card).
* `POST /stripe/webhook` (no JWT; verify the `Stripe-Signature` HMAC with the secret from Secrets Manager; dedupe with an idempotency `EVT#{id}` item) handles `checkout.session.completed`, `customer.subscription.updated|deleted` and `invoice.payment_failed` → `CLIENT#{id}/SUB` `{status, currentPeriodEnd}`.
* Gating: a `past_due`/`canceled` status shows a banner and stops new program assignment. **Never** block offline logging.
* SaaS billing for coaches themselves (if you sell to many coaches): a separate Stripe subscription on the coach profile, checked when an admin approves or invites a coach, and in `Ctx`.

### 5. MVP hardening (NFRs)

* **ON-1 email:** SES with a verified domain; `POST /invites` sends the templated email, and the mailto fallback stays.
* **Coach MFA:** add a Cognito **pre-token-generation** Lambda that adds `role`. The API then rejects coach routes unless the token's `amr` claim includes `mfa`, and the PWA links to the hosted UI's TOTP setup. Alternatively, use separate coach and client user pools, with `MfaConfiguration: ON` for the coach pool.
* **Privacy export & delete (privacy NFR):** `POST /me/export` → an async job (SQS) that queries `CLIENT#{id}`, zips JSON + CSV + media into S3 and emails a presigned link; `DELETE /me` → deletes partition items in batches, the S3 prefix `media/{coach}/{client}/` and the Cognito user, and writes a tombstone audit record. Add an explicit consent checkbox at signup (privacy NFR), stored on the profile with a timestamp.
* ✅ **Upload size limits:** done. Presigned PUTs are signed for an exact, capped size (15 MB photos, 250 MB demo videos), so S3 refuses anything else; no post-upload check is needed. Video parts (section 1) should do the same per part.
* **Observability:** CloudWatch alarms on Lambda errors/throttles, HTTP API 5xx, and DLQ depth → SNS email. Use structured `tracing` JSON logs (already in place). Worth doing before the first real users, ahead of the video work.
* **Backups:** an AWS Backup plan (daily, 35 days) and a scripted quarterly restore test into a scratch table.
* ✅ **CI/CD:** done. `.github/workflows/ci.yml` runs clippy (`-D warnings`), unit tests, `cfn-lint`, a web syntax check, the e2e test with moto as a service container and the Playwright journey on every push and pull request. Deploys run locally with `scripts/deploy.sh`: the AWS Organization blocks the OIDC provider a GitHub deploy would need. Revisit if its admin allows one (the removed deploy job and role template are in git history).
* ✅ **Coach signups:** done. Invitation only: an admin (bootstrapped from `ADMIN_EMAILS`) emails a coach an invitation tied to their address (SES, from `INVITE_FROM`); the link opens the app's sign-up page with the email filled in and stops working once used. The open coach request was removed. Still open: SES production access and the domain's DKIM records (DEPLOY.md), emailing client invitations the same way, and suspending a coach.
* **Photo list:** `GET /photos` still checks S3 once per photo on every load. Move photos to the upload + confirm pattern the exercise demos use, which also lets the pending-tag lifecycle rule clean up abandoned photo uploads.

**Phase 1 exit gate (BRD):** clients log at least 80% of assigned workouts for 4 weeks, which the dashboard's adherence % measures directly; coach video review takes under 5 minutes.

---

## Phase 2: coach productivity (≈ 6-8 weeks)

| Feature | Implementation |
|---|---|
| **VR-4 Drawing tools** | A `<canvas>` over the video; tools for line, angle (3-point, live degrees), circle, arrow, freehand and an on-screen timer. Strokes are recorded as timeline events `{t, tool, points, color}` in the same edit list; the render worker draws them with ffmpeg `drawbox`/`drawtext`, or by rasterizing the strokes to PNG overlays per segment (`overlay=enable='between(t,a,b)'`). |
| **VR-5 Compare** | Pick a reference clip (exercise demo or the client's earlier `VIDEO#` for the same `exerciseId`, found via GSI `CLIENT#{id}#EX#{exerciseId}`). Side-by-side with two `<video>` elements and synced `currentTime`; overlay with CSS opacity. The renderer uses `hstack` or `blend`. |
| **FB-3 Weekly check-in** | `COACH#/CHECKIN_TEMPLATE#{id}` holds the questions (JSON schema: scale/text/number/photo). `CLIENT#/CHECKIN#{weekStart}` stores the answers. An EventBridge Scheduler weekly rule in each client's `Profile.timeZone` sends the reminder (MS-4). The comparison view shows the last 4 weeks as columns. |
| **FB-4 Saved snippets** | `COACH#/SNIPPET#{id}`; a picker in the comment box and video reply; `/` shortcut. |
| **WB-5 %1RM loads** | Add `CLIENT#/MAX#{exerciseId}` (tested or estimated). `Item.load` becomes an enum `{kg} \| {pct1rm}`. At **assign time** `materialize()` resolves % to kg rounded to the plate increment (core function, unit-tested); the snapshot keeps both. |
| **WB-6 Progression rules** | A core `ProgressionRule { exerciseId, when: AllRepsHitAtOrBelowRpe(x), then: AddKg(2.5) \| AddReps(1), cap }`. Evaluated in Rust when a log is saved (or nightly): it proposes next-week load changes as **pending suggestions** for the coach (the same coach-in-the-loop UI as AI-6). Pain flags block suggestions (BRD risk). |
| **LG-5 PR detection** | On `put_log`, compare each done set's e1RM and rep-max with `CLIENT#/PR#{exerciseId}#{reps}`; write new PRs and a `PR` event; celebrate in the UI. e1RM history already exists in `stats::strength_trends`. |
| **LG-7 Alerts** | A DynamoDB Stream on the table → Lambda: on LOG with `check.pain`, or daily (EventBridge) when consecutive missed ≥ N (coach setting), write `COACH#/ALERT#{ts}` and push to the coach. The roster flags exist already; add an alert inbox. |
| **BZ-2 Scheduling** | `COACH#/AVAIL` weekly hours (doubles as MS-5); `CLIENT#/SESSION#{start}`; `.ics` feed per user (`GET /cal/{token}.ics`, a secret token) for Google/Apple calendar sync; Google Calendar API two-way sync later. Video calls via a meeting-link field, or Amazon Chime SDK. |
| **BZ-3 Habits** | `CLIENT#/HABIT#{id}` (target, unit) + `HABITLOG#{date}#{habitId}`; streaks computed in core; quick-tap tiles on Today; offline via the outbox like logs. |
| **BZ-6 Export** | Reuse the Phase 1b export job; add a PDF progress report (charts rendered server-side with `plotters` → PNG, laid out with `printpdf`/`typst`). |
| **ON-4 Baseline** | A "baseline" flag on metrics/photos plus a test-lift form that writes `MAX#` items. |
| **ON-5 Intake templates** | Move intake questions to `COACH#/INTAKE_TEMPLATE` (same schema engine as FB-3); `Intake` becomes `{templateVersion, answers: map}` with the current fixed fields kept for compatibility. |
| **BM-6 Other estimates** | US Navy formula in `bodycomp.rs` (needs height + neck/waist/hip tape); a manual entry kind `external {source: dexa\|bia\|scale, bodyFatPct}`. |
| **MS-3 Deep links** | Messages carry `ref: {type: log\|set\|video, id}`; render as cards linking to `#/workout/...` or the video. |
| **MS-4 Automated messages** | EventBridge Scheduler schedules per coach rule (`SCHEDULE#{id}`) → Lambda sends the message and push. |
| **VS-5 Filming guidance** | Add `filming {angle: side\|front\|45, distanceM, notes}` to `Exercise`; shown before the camera opens. |

**Exit gate:** a coach handles 30+ clients without the app being the bottleneck. Instrument time-in-app per client per week.

---

## Phase 3: local AI (≈ 8-10 weeks; needs Phase 1-2 data)

**Topology (BRD option 3, hybrid):** a GPU box you own runs an **AI worker** that *pulls* jobs, so no inbound port is opened.

* Queue: SQS `AiJobs` + DLQ. The worker uses an IAM user/role restricted to `sqs:ReceiveMessage/DeleteMessage` on that queue, `s3:GetObject` on `media/*/videos/mp4/*`, and `execute-api:Invoke` on one `POST /ai/results` route (IAM auth instead of JWT).
* Worker stack: Python, **RTMPose / YOLO-pose** for keypoints, and **Ollama or vLLM** with an 8-14B quantized open-weight model on a 16 GB GPU.

| Feature | Implementation |
|---|---|
| **AI-1 Pose metrics** | Job on video ready: keypoints per frame → rep segmentation (hip/bar vertical-velocity zero crossings) → per rep: depth (hip-knee angle at the bottom), ROM, tempo (eccentric/concentric durations), bar path (wrist-midpoint trajectory) and joint angles. Store `VIDEO#/ANALYSIS` with per-metric **confidence**; hide below a threshold (BRD risk). Optional on-device instant rep count with MediaPipe Pose (WASM) in the PWA. |
| **AI-2 Fault flags** | Start with 3-5 lifts (open question in the BRD; suggested: squat, deadlift, bench, overhead press, hip thrust). Rules on the keypoint series (knee valgus angle, hip-rise ratio vs chest, lumbar flexion proxy) → `{fault, tStart, tEnd, confidence, evidence}`. Build a **labeled clip set** from Phase 1-2 coach replies to tune thresholds. |
| **VR-7 Skeleton overlay** | The editor draws keypoints from `ANALYSIS` on the canvas (toggle). |
| **AI-3 Progression suggestions** | **Deterministic** in `crates/core` (the BRD says LLMs must not do the arithmetic): an RPE/RIR autoregulation table, stall detection (e1RM flat for 3 exposures with RPE rising → deload 10%), and hard caps on weekly load increase. The LLM only writes the explanation. |
| **AI-4 Weekly summary** | Weekly EventBridge → job with the dashboard JSON (adherence, volume, e1RM, check-ins, pain flags, open issues) → LLM summary with cited data points. |
| **AI-5 Draft replies** | A prompt with the coach's last 50 replies as a style guide plus thread context → a draft in the composer, never sent automatically. |
| **AI-6 Coach-in-the-loop** | All AI output becomes `SUGGESTION#{id}` items with status pending, shown in a coach inbox with approve/edit/reject. Only an approve action creates client-visible items. No API path lets AI write to client-visible keys; enforce this in code review and with an e2e test. |
| **AI-7 Reasons** | Every suggestion stores `evidence: [{metric, value, date, source}]`, rendered as "Because: …". |

**Exit gate:** coaches accept most suggestions with minor edits on the labeled test set. Log acceptance/edit-distance per suggestion type.

---

## Phase 4: growth

| Feature | Implementation |
|---|---|
| **Native apps** | Wrap the PWA with **Capacitor** for App Store / Play: same web code, plus native camera, background upload, APNs/FCM push and HealthKit / Health Connect plugins. Prototype the video editor in the wrapper early; go native only if it can't perform (BRD stack note). |
| **LG-8 Wearables** | Capacitor Health plugins read steps, HR and sleep daily → `CLIENT#/HEALTH#{date}`; feeds habits and AI-4. |
| **BZ-4 Nutrition** | Macro targets `CLIENT#/NUTRITION_TARGET`, daily log `FOOD#{date}`; CSV import from Cronometer/MyFitnessPal exports (their APIs are restricted). |
| **WB-7 Group programs** | `COACH#/GROUP#{id}` with members; assign = fan-out materialize per member (SQS for large groups). |
| **MS-6 Group chat** | Threads become first-class: `THREAD#{id}` with members; WebSocket fan-out to members. |
| **BZ-5 Branded app** | `COACH#/BRAND` (logo, colors) → CSS variables at boot. A custom domain per coach via CloudFront + ACM; a per-coach Capacitor build for true white-label. |
| **Assistant coach** | Role `assistant` with `ASSIGNMENT#{assistant}#{client}` items; `client_scope` checks it with a permissions bitmask (view, comment, program). |
| **VR-6 PiP coach camera** | Record the webcam with the voice-over (`getUserMedia` video) and composite it in the render (`overlay` bottom-right). |
| **MS-5 Availability** | Display `COACH#/AVAIL` in the client thread header (built in BZ-2). |

## Open questions from the BRD (answers needed before the phase they affect)

1. **Single coach vs. SaaS.** Built multi-tenant (one coach = one tenant). Decide whether coaches pay you (Phase 1b, BZ-1). Coach onboarding is admin-gated already (1b hardening); a SaaS plan would add a subscription check to it.
2. **Where local AI runs.** The design assumes a GPU box pulling from SQS (Phase 3).
3. **Lifts for form review.** Pick 3-5 before Phase 3; start labeling clips in Phase 1b.
4. **Nutrition in scope?** Phase 4 by default.
5. **Native vs cross-platform.** Currently a PWA, with Capacitor in Phase 4 if store distribution or native APIs are needed sooner.
