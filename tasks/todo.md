# Tasks: Exercise video submission (VS-1 … VS-4)

Plan and decisions: `tasks/plan.md`. Standard verification for every task:

- `cargo clippy --all-targets -- -D warnings` and `cargo test`
- `E2E_ENDPOINT=http://127.0.0.1:5055 cargo test -p api --test e2e` (moto running)
- `python tests/ui/journey.py` (local API and web server running) for UI tasks
- CI green on push

---

## Phase 0: De-risk

### Task 1: Spike: trim and re-encode a phone clip in the browser on real devices

**Status:** done. Results and the proposed decision are in `tasks/plan.md` › Spike Results. Spike page: https://claude.ai/artifact/QuVA7gQ56VFiK7kwARXuwR. An iPhone encodes a 1080p clip at 2.3× real time with audio; Windows Edge and Chrome play iPhone H.264 originals; iOS `<input capture>` records at 480×360 (so a new Task 4b adds an in-app recorder). HEVC is untested (moved into Task 5).

**Description:** Build a throwaway page, outside the app (for example a scratch HTML file served locally and opened on phones over LAN or a tunnel). It demuxes a picked video with mp4box.js, decodes the in/out range with WebCodecs, re-encodes to H.264 (1080p and 720p) with audio, muxes with mp4-muxer, and draws a poster frame. Run it on the devices clients and coaches actually use, then record the results in `tasks/plan.md`.

**Acceptance criteria:**
- [x] Results table (done for iPhone H.264 and desktop Edge/Chrome; no Android device available; HEVC moved to Task 5) for at least: iPhone Safari (HEVC and "Most Compatible" H.264 recordings), Android Chrome, desktop Chrome/Edge. For each: did decode work, did encode work, time for a 60 s clip at 1080p and at 720p, output size
- [x] Output MP4s play in Chrome, Edge and Safari (desktop and phone)
- [x] Recommendation recorded: confirm "browser encode + fallback", or switch to server transcode (Task 11 moves up)

**Verification:**
- [x] Manual: the human reviews the results table and approves the processing decision (approved 2026-10-27)

**Dependencies:** None
**Files likely touched:** scratch only; `tasks/plan.md` (results)
**Estimated scope:** M

---

## Checkpoint: Processing decision
- [x] Decision 1 in `tasks/plan.md` confirmed or revised with the human (confirmed: browser encoding at 720p, plus fallback)

---

## Phase 1: Thin slice

### Task 2: Video API: create, presigned part URLs, complete, list

**Status:** done. `crates/api/src/routes/videos.rs`; e2e covers create (with validation and permissions), part URL re-issue, early and malformed completes, upload, complete (idempotent), tag removal, list with playback/poster URLs and `logId` filter, tenant isolation. The media bucket's CORS now exposes `ETag` (deploy before Task 3's browser upload).

**Description:** Add the `Video` model (`CLIENT#{cid}` / `VIDEO#{id}`: logId, itemId, setIndex, exerciseId, status uploading|ready, key, posterKey, sizeBytes, durationSec, trimStart/trimEnd, createdAt, reviewedAt). Routes:
- `POST /clients/{cid}/videos` (client only): validates content type and size (cap), starts a multipart upload tagged `state=pending`, returns the video plus part URLs, each signed for its exact length.
- `POST …/videos/{id}/parts`: re-issues URLs for given part numbers.
- `POST …/videos/{id}/complete` with `{parts:[{n, etag}], poster?}`: completes the upload, checks the object exists and its size, removes the tag, sets status ready.
- `GET …/videos` (client or coach): lists ready videos with presigned GET URLs.
- Poster: a separate small presigned PUT returned at create.

**Acceptance criteria:**
- [x] A client can create, upload parts, complete and list; a coach can list and play their client's videos; other coaches and clients get 403
- [x] Part URLs are signed for their exact length; a size over the cap is a 400; completing twice is idempotent
- [x] Unconfirmed uploads keep `state=pending`, so the existing lifecycle rules abort and expire them

**Verification:**
- [x] e2e: create → PUT parts (via the SDK) → complete → list shows a playable URL; tenant isolation; size cap; tag removed after complete
- [x] Unit tests for part splitting (sizes, last part, counts)

**Dependencies:** Task 1 (only for the fields the fallback needs: trimStart/trimEnd)
**Files likely touched:** `crates/core/src/model.rs`, `crates/api/src/routes/videos.rs` (new), `crates/api/src/lib.rs`, `crates/api/src/util.rs`, `crates/api/tests/e2e.rs`
**Estimated scope:** M

### Task 3: Client uploads a clip from the set row; coach plays it in the Logs tab

**Status:** done except the real-phone check on dev. Each exercise has a video bar under its sets ("▶ Set N" and "🎥 Add video") instead of a fifth column in each set row, which would squeeze the inputs at phone width. `web/js/videos.js` uploads parts with XHR (progress, ETag) and plays videos in a sheet. The coach's Logs tab shows ▶ per exercise. Also fixed a router race this exposed (two quick navigations could leave the older page on screen). The local server now gives its bucket the same CORS rule as AWS.

**Description:** A 🎥 button on each set row in the logger opens a library file picker (`accept="video/*"`, no `capture`: on iOS that records at 480×360; in-app recording is Task 4b). While online, it uploads the clip as is (no trim yet) through Task 2's API with a progress bar, then shows a thumbnail on the set. In the coach's Logs tab, sets with a video get a ▶ that opens a player sheet.

**Acceptance criteria:**
- [x] A client attaches a clip to a set and sees upload progress, then a ▶ for that set (poster thumbnails come with Task 5, which makes the poster frame)
- [x] The coach sees ▶ on that set in Logs and the clip plays (with seeking)
- [x] Offline, the button explains that video upload needs a connection (resumable offline comes in Task 7)

**Verification:**
- [x] Journey test: client uploads a small fixture clip; coach opens it in Logs
- [ ] Manual: real phone upload on dev

**Dependencies:** Task 2
**Files likely touched:** `web/js/views/workout.js`, `web/js/views/coach.js`, `web/js/videos.js` (new: upload + player helpers), `web/sw.js`, `tests/ui/journey.py`, `tests/ui/fixtures/clip.mp4`
**Estimated scope:** M

## Checkpoint: Thin slice
- [ ] All tests green; deployed to dev; a real phone clip plays for the coach
- [ ] Review with the human before Phase 2

---

## Phase 2: Trim and cap (VS-2)

### Task 4: Trim screen with in/out handles and a 60 s cap

**Status:** done except the manual phone check. `web/js/views/trim.js` is a reusable trim control (preview, start and end handles on one track, readout, Play loops the selection), shown in the Add video sheet once a clip is picked. Until Task 5 re-encodes on the device, the original uploads with `trimStart`/`trimEnd` and the player keeps to them (part of Task 6, done early). `ui.sheet()` gained an `onClose` hook so a playing preview stops when the sheet is dismissed. The journey's fixture is now a 65 s clip (173 KB) to exercise the cap.

**Description:** After picking a clip, show a preview `<video>` with in/out range handles and the current length. The selection can't exceed 60 s, and longer clips start with the first 60 s selected. Confirm passes `{file, start, end}` on to processing.

**Acceptance criteria:**
- [x] The handles move with touch and mouse; the selected length is shown; more than 60 s can't be selected
- [ ] Preview playback loops within the selection (implemented; not covered by the journey test, check on a phone)
- [x] Usable at phone width (390 px) with no horizontal scroll

**Verification:**
- [x] Journey test drives the handles on the fixture clip
- [ ] Manual: iPhone and Android

**Dependencies:** Task 3
**Files likely touched:** `web/js/views/trim.js` (new), `web/js/views/workout.js`, `web/css/app.css`, `web/sw.js`
**Estimated scope:** S

### Task 4b: In-app recorder at 720p with the 60 s cap

**Status:** done except the real-phone checks. `web/js/views/recorder.js`: "● Record with camera" in the Add video sheet opens a camera sheet (rear camera, 1280×720 requested, MP4 preferred, 2.5 Mbps) with a timer, Switch camera when there is more than one, and an automatic stop at 1:00. The recording goes into the same trim and convert steps as a library clip. The trim control takes the recorder's measured length when a recording doesn't report its own. Denied or missing cameras show how to allow access, and choosing from the library still works. Any conversion error now falls back to uploading the original, not only `CannotEncode`. The journey records about 3 s from a fake camera (Chromium `--use-fake-device-for-media-stream`); in Edge it was converted (MP4, 3.0 s, with a poster) and the coach played it.

**Description:** "Record" opens an in-page camera view (`getUserMedia`, rear camera, 1280×720 requested) and records with `MediaRecorder`, preferring `video/mp4` (H.264/AAC on Safari, and Chrome 126+); otherwise WebM, which Task 5 re-encodes. It stops automatically at 60 s and hands the clip to the trim screen. Camera permission is asked on first use; if it's denied, the page explains how to allow it and offers library pick instead.

**Acceptance criteria:**
- [ ] On iPhone Safari, a recorded clip is at least 720p (not 480×360), with audio
- [ ] Recording stops at 60 s; switching cameras works where a device has more than one
- [ ] Denied permission degrades to library pick with a clear message

**Verification:**
- [ ] Manual: iPhone Safari and the home-screen app; Android Chrome if available
- [x] Journey test with a fake camera (`--use-fake-device-for-media-stream`) records a few seconds and reaches the trim screen

**Dependencies:** Task 4
**Files likely touched:** `web/js/views/recorder.js` (new), `web/js/views/workout.js`, `web/css/app.css`, `web/sw.js`, `tests/ui/journey.py`
**Estimated scope:** M

### Task 5: Browser re-encode of the trimmed range to H.264 MP4 plus a poster

**Status:** done except the real-iPhone checks (speed against the spike, and an HEVC clip). `web/js/encode.js` converts the selection to H.264 at up to 720p, copies the AAC audio and makes a JPEG poster from the first frame. The vendored mp4box 0.5.2 and mp4-muxer 5.1.3 (`web/vendor/`, hashes in its README) load on first use and are precached. The converted clip uploads with its poster and no trim points. If the device can't convert (`CannotEncode`), the original uploads with trim points (Task 6). Verified: the app's encode.js on the real iPhone clip gave 720×1280, 6.0 s, stereo AAC, 2.1 MB and a 94 KB poster in 0.85 s (desktop Edge). The journey covers both paths; CI now uses Google Chrome, which has an H.264 encoder. The "poster appears as the thumbnail" criterion is met as the video's poster image in the player; the ▶ buttons are still text.

**Description:** Using the spike's approach, demux (vendored mp4box.js), decode the selected range, re-encode to H.264 at the spike's chosen resolution and bitrate with AAC audio, mux (vendored mp4-muxer), and capture a poster frame. Show progress with cancel. Upload the result through the existing flow.

**Acceptance criteria:**
- [x] The output is the selected range only, at most 60 s, plays in all target browsers, and is roughly the expected size (about 4 Mbps × duration)
- [ ] HEVC ("High Efficiency") iPhone clip tested on a real device: decodes and re-encodes on the phone, or falls back (Task 6). Record the result
- [x] The poster appears as the thumbnail for client and coach
- [x] The vendored libraries are pinned, load only when the trim screen opens, and are cached for offline use

**Verification:**
- [x] Journey test: trimmed fixture → uploaded duration about equal to the selection
- [ ] Manual: real iPhone and Android clips, checked against the spike timings

**Dependencies:** Task 4, Task 1 decision
**Files likely touched:** `web/js/encode.js` (new), `web/vendor/*` (new, pinned), `web/js/views/trim.js`, `web/sw.js`
**Estimated scope:** M

### Task 6: Fallback: upload the original with trim points; the player clamps to them

**Status:** mostly done (Tasks 4 and 5). Trim points are uploaded, stored and enforced by the player. The fallback is chosen automatically whenever conversion isn't possible (no WebCodecs, an undecodable codec, a non-MP4/MOV container, or a mid-way failure). localStorage `pt.forceVideoFallback=1` forces it for tests, and the journey uses that. The server caps originals at 500 MB. Still to do: the cellular-data warning before a large original uploads.

**Description:** When WebCodecs is missing or can't decode the input (per Task 1), upload the original and send `trimStart`/`trimEnd`. The player starts at `trimStart` and stops at `trimEnd`. No poster: show a generic tile.

**Acceptance criteria:**
- [x] Forcing the fallback (a test flag) still yields a clip that plays only the selected range
- [ ] The size cap and the cellular warning apply to originals

**Verification:**
- [x] Journey test with the fallback forced
- [x] e2e: trim points stored and returned

**Dependencies:** Task 5
**Files likely touched:** `web/js/encode.js`, `web/js/videos.js`, `crates/api/src/routes/videos.rs`, `tests/ui/journey.py`
**Estimated scope:** S

## Checkpoint: Trim
- [ ] Trimmed clips from iPhone and Android play for the coach on desktop and phone
- [ ] Review with the human

---

## Phase 3: Resumable and offline (VS-3)

### Task 7: Upload manager: parts and progress in IndexedDB, resume after reload

**Status:** done except the manual throttled-3G run on a phone. `web/js/uploads.js` saves each clip in IndexedDB (`uploads` store, DB version 2) with the multipart upload id and each part's ETag, and sends part by part on `online`, every 30 s and at start-up. Expired part URLs (403) are re-issued and retried once; an upload S3 already discarded (404 on a part, or 400/404 on complete) starts again. After 6 restarts, or on a permanent 4xx, the upload is marked failed with Try again and Remove. The app asks for persistent storage. Pending uploads show as chips ("⏳ Set 2 · 45%", "waiting for connection", "⚠ failed") in the exercise's video bar. The journey cuts a part upload off, reloads the page and sees the upload finish.

**Description:** Add `web/js/uploads.js`. It stores the processed blob and upload state (video id, uploaded parts and their ETags) per user in IndexedDB. It uploads part by part, re-requests expired part URLs, and resumes from the last completed part after a reload, a dropped connection or a browser restart. The upload status shows on the set and in a small upload indicator, the same way the sync indicator works.

**Acceptance criteria:**
- [x] Killing the tab mid-upload and reopening resumes from the next part, not from zero
- [x] Expired part URLs are re-issued automatically
- [x] The clip is removed from IndexedDB once the upload completes

**Verification:**
- [x] Journey test: block the network mid-upload, reload, unblock, and the upload completes
- [ ] Manual: real phone on throttled 3G

**Dependencies:** Task 3 (works with Tasks 5 and 6 output)
**Files likely touched:** `web/js/uploads.js` (new), `web/js/idb.js` (new store, version bump), `web/js/videos.js`, `web/js/app.js`, `web/sw.js`
**Estimated scope:** M

### Task 8: Offline queueing; a client can delete their video

**Status:** done. Converting and queueing work offline; the upload starts by itself on reconnect. `DELETE /clients/{cid}/videos/{vid}` (the owning client) aborts an unfinished multipart upload, deletes the video and poster objects, and removes the coach's queue entry and the record. Clients get Delete video in their player. Sign-out removes that user's queued clips. e2e covers the delete, the 403s, removal from the queue, and the aborted multipart upload; the journey queues a clip offline, sees it upload after reconnecting, and deletes it.

**Description:** Filming while offline queues the processed clip. The upload is created when the connection returns (from the same loop as the outbox flush), and works even if the workout log itself hasn't synced yet. Add `DELETE …/videos/{id}` for the owning client: it removes the object, the poster and any queue item.

**Acceptance criteria:**
- [x] Airplane mode → film and trim → reconnect → the upload completes without user action
- [x] Queued clips are per user, like the outbox
- [x] Deleting removes the video for both client and coach, including from the review queue

**Verification:**
- [x] Journey test: offline queue, then upload after reconnect
- [x] e2e: delete by the owner; 403 for the coach and other users

**Dependencies:** Task 7
**Files likely touched:** `web/js/uploads.js`, `web/js/videos.js`, `crates/api/src/routes/videos.rs`, `crates/api/tests/e2e.rs`, `tests/ui/journey.py`
**Estimated scope:** M

## Checkpoint: Resumable
- [ ] Roadmap exit test part 1: a 60 s clip on throttled 3G resumes after airplane mode (manual, on dev)
- [ ] Review with the human

---

## Phase 4: Coach review queue (VS-4)

### Task 9: Review queue API (oldest first, mark reviewed) and a coach Videos tab

**Status:** done. API: completing an upload writes a `ReviewEntry` (`COACH#` / `REVIEW#{createdAt}#{video}`) before marking the video ready, so a retried complete re-queues it. `GET /videos/queue` (coach) lists entries oldest first with playback and poster URLs and audit-logs `view-video-queue` per client. `POST /clients/{cid}/videos/{vid}/reviewed` (that client's coach) sets `reviewedAt` and removes the entry; repeating it is fine. UI: a coach **Videos** tab with an unreviewed count, and queue cards with a thumbnail, client, exercise · set · length, how long it has waited (older than 2 days highlighted), Watch, Comment, Open workout and ✓ Reviewed. The player also gets ✓ Mark reviewed (in the queue and in Logs), and reviewed videos show ✓ in Logs. e2e covers ordering, permissions, idempotency, not-ready videos and the audit entry; the journey covers the tab, badge, both ways of marking reviewed, and the ✓ in Logs.

**Description:** Completing an upload writes `COACH#{coach}` / `REVIEW#{createdAt}#{videoId}` with the client name, exercise, set and date. `GET /videos/queue` (coach) returns them oldest first with playback URLs. `POST …/videos/{id}/reviewed` sets `reviewedAt` and deletes the queue item. The coach gets a **Videos** tab with the queue count. Each item has a player, the set details, a link to the workout, the existing comment-on-set action, and **Mark reviewed**.

**Acceptance criteria:**
- [x] The queue is ordered oldest first across all of the coach's clients
- [x] Mark reviewed removes the item; the video stays visible in Logs with a "reviewed" mark
- [x] Only the client's coach sees their queue items

**Verification:**
- [x] e2e: three uploads at different times come back oldest first; reviewed removes one; tenant isolation
- [x] Journey test: client upload → coach Videos tab → mark reviewed → queue empty

**Dependencies:** Task 2 (UI also uses Task 3's player)
**Files likely touched:** `crates/api/src/routes/videos.rs`, `crates/api/src/lib.rs`, `crates/api/tests/e2e.rs`, `web/js/views/videos.js` (new), `web/js/app.js`
**Estimated scope:** M

## Checkpoint: Complete
- [ ] Roadmap exit test passes end to end on dev (resume after airplane mode; queue oldest first)
- [ ] All tests and CI green; deployed to dev
- [ ] Review with the human

---

## Phase 5: Hardening

### Task 10: Retention lifecycle, limits, journey test and docs

**Status:** done. Retention is 180 days, the plan's proposal, adopted as template parameter `VideoRetentionDays` since no other period was chosen; change it per deploy. Marking reviewed tags the video and poster `reviewed=true`; lifecycle rule `expire-reviewed-set-videos` deletes them that many days after upload, and the API hides expired records (`videos::expired`, unit-tested). The IAM statement gained `s3:AbortMultipartUpload` for client deletes. The caps are unchanged: 500 MB per upload, 60 s watched, 8 MiB parts. Docs: README, ARCHITECTURE (offline model, API surface, media lifecycle) and ROADMAP (section 1 rewritten as built, status table).

**Description:** Apply the agreed retention policy (open question 1). Proposal: a lifecycle rule that deletes reviewed videos after N days, applied by tagging an object `reviewed=true` when it's marked reviewed. Finalize the size and duration caps. Update `docs/ROADMAP.md` (VS-1..4 status), `docs/ARCHITECTURE.md` (keys and media layout) and README.

**Acceptance criteria:**
- [x] The lifecycle rule exists in `template.yaml`, and objects are tagged on review
- [x] Docs describe the video flow, keys, caps and retention

**Verification:**
- [x] `cfn-lint template.yaml`; e2e asserts the tag is set on review
- [ ] Deploy to dev; the rule is visible on the media bucket

**Dependencies:** Task 9, open question 1 answered
**Files likely touched:** `template.yaml`, `crates/api/src/routes/videos.rs`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`, `README.md`
**Estimated scope:** S

### Task 11 (only if the spike requires it): server transcode fallback

**Description:** Only if Task 1 shows the fallback path is common or its originals don't play for coaches. First, a throwaway stack checks that the org allows ECR, a container-image Lambda and SQS. Then: an S3 event → SQS → an ffmpeg Lambda transcodes fallback originals to H.264 MP4 plus a poster, and status goes processing → ready. Split into smaller tasks before starting.

**Dependencies:** Tasks 1, 6
**Estimated scope:** L (break down before starting)
