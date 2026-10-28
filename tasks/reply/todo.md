# Tasks: Coach video reply editor (VR-1 … VR-3)

Plan and decisions: `tasks/reply/plan.md`. Standard verification for every task:

- `cargo clippy --all-targets -- -D warnings` and `cargo test`
- `E2E_ENDPOINT=http://127.0.0.1:5055 cargo test -p api --test e2e` (moto running)
- `python tests/ui/journey.py` (local API and web server running) for UI tasks
- CI green on push

---

## Phase 0: De-risk

### Task 1: Spike: record a canvas-drawn S3 clip plus microphone in the coach's browsers

**Description:** A throwaway page (an Artifact, like the submission spike) that loads a real set video from the dev media bucket through a presigned URL with `crossorigin="anonymous"`, draws it to a canvas every animation frame, and records `canvas.captureStream(30)` plus the microphone with `MediaRecorder`. While recording, the tester plays, pauses, switches between 0.25×, 0.5× and 1×, and steps frames. The page shows the output format, size and duration, and plays it back. Results are saved to the page's log. Then the output is played on an iPhone.

**Acceptance criteria:**
- [ ] Canvas recording works with the S3 clip (no tainted-canvas error) on dev, in Chrome or Edge and in Safari on a Mac if available
- [ ] Recorded format per browser (MP4/H.264+AAC or WebM) noted, and the output plays on iPhone Safari
- [ ] Slow motion, pauses and frame steps look right in the recording (steady frame rate, no black frames)
- [ ] Recommendation recorded: confirm browser recording, or fall back to the roadmap's server render

**Verification:**
- [ ] Manual: the human records on their coach machine and plays the result on their iPhone
- [ ] Results table in `tasks/reply/plan.md`

**Dependencies:** None (needs a deployed set video on dev)
**Files likely touched:** scratch only; `tasks/reply/plan.md`
**Estimated scope:** M

## Checkpoint: Recording decision
- [ ] Decision 1 confirmed or revised with the human

---

## Phase 1: Watch closely and mark

### Task 2: Review screen: slow motion, frame stepping, keyboard shortcuts (VR-1)

**Description:** A full-page review screen (`#/review/{clientId}/{videoId}`), opened from **Reply** on a queue card, in the coach's player, and from Logs. The clip draws on a canvas, the same one Task 4 records. Controls: play/pause, 0.25× / 0.5× / 1×, step back and forward one frame (seek ± 1/fps, frame rate from the video or `requestVideoFrameCallback`), a scrubber, and the time. Keyboard: space play/pause, `,` and `.` step, `1` `2` `3` speeds, arrow keys ± 1 s. Trimmed originals keep to their trim points. Context is shown: client, exercise, set, and the logged reps/load/RPE for that set.

**Acceptance criteria:**
- [ ] All controls and shortcuts work; each frame step moves exactly one frame
- [ ] Works with converted clips and with trimmed originals
- [ ] Usable by touch at tablet width, and on a phone at least for watching (open question 5)

**Verification:**
- [ ] Journey test: open the review screen from the queue, switch speeds, step frames (check `currentTime` moves by about 1/fps)
- [ ] Manual: coach laptop

**Dependencies:** Task 1
**Files likely touched:** `web/js/views/review.js` (new), `web/js/views/videos.js`, `web/js/views/coach.js`, `web/js/app.js`, `web/css/app.css`, `web/sw.js`
**Estimated scope:** M

### Task 3: Segments: mark in/out, reorder, play through them (VR-3)

**Description:** While watching, the coach marks **In** and **Out** (buttons, and keys `i` and `o`) to add a segment. The segment list shows each range with Play, move up, move down and remove; drag to reorder where pointer events allow. **Play segments** runs them in order, jumping between them. Segments are kept per video in local storage until the reply is sent, so a reload doesn't lose them.

**Acceptance criteria:**
- [ ] Mark, reorder, remove and play-through work; In must come before Out; segments stay inside the clip (and its trim)
- [ ] With no segments, the whole clip is used
- [ ] Segments survive a reload until sent

**Verification:**
- [ ] Journey test: mark two segments, swap them, play through (check the order of times reached)

**Dependencies:** Task 2
**Files likely touched:** `web/js/views/review.js`, `web/css/app.css`
**Estimated scope:** S

## Checkpoint: Review screen
- [ ] A coach studies a clip at 0.25× and frame by frame, and plays marked segments in order
- [ ] Review with the human

---

## Phase 2: Record and send

### Task 4: Record a reply: canvas plus voice, preview, re-record, 5-minute cap (VR-2)

**Description:** **● Record reply** asks for the microphone, then records the canvas (`captureStream(30)`) plus the mic while the coach uses the player and segments as normal; the clip's sound is off (decision 2). A timer shows the length and recording stops at 5 minutes. **Stop** shows a preview of the reply with **Send**, **Re-record** and an optional written note. The format is picked as Task 1 decides (MP4 preferred).

**Acceptance criteria:**
- [ ] The reply shows exactly the coach's playback (slow motion, pauses, steps, segment jumps) with their voice
- [ ] A denied microphone gives a clear message (and text-only if open question 4 says so)
- [ ] Recording stops at the cap; Re-record discards the previous take

**Verification:**
- [ ] Journey test with a fake microphone: record about 3 s while switching speeds; the preview plays and is about 3 s long
- [ ] Manual: coach laptop, played back on iPhone

**Dependencies:** Tasks 2 and 3
**Files likely touched:** `web/js/views/review.js`, `web/js/reply-recorder.js` (new), `web/sw.js`
**Estimated scope:** M

### Task 5: Reply API: shared multipart helpers; sending marks the video reviewed

**Description:**
- **Shared helpers:** move the multipart create, re-issue and complete logic from `routes/videos.rs` into a shared module (`routes/media.rs`) used by both set videos and replies.
- **Routes:** `POST /clients/{cid}/videos/{vid}/replies` (the client's coach) creates `REPLY#{id}` and the multipart upload. `…/replies/{rid}/parts` and `…/complete` are as for videos.
- **Complete** also marks the video reviewed (same as `reviewed`), tags the reply and the video `reviewed=true`, and writes a thread comment with `replyId` and the coach's note.
- **Listing:** `GET /clients/{cid}/replies?videoId=` (client or coach) returns playback and poster URLs; the client's `POST …/replies/{rid}/seen` sets `seenAt`.

**Acceptance criteria:**
- [ ] Only the client's coach creates and completes replies; the client and their coach can list them
- [ ] Completing marks the video reviewed, removes it from the queue, tags both objects and adds the thread comment
- [ ] The set-video endpoints behave exactly as before (e2e unchanged and green)

**Verification:**
- [ ] e2e: the whole reply flow, permissions, the side effects on the video and queue, idempotent complete, `seenAt`

**Dependencies:** None (can run alongside Tasks 2–4)
**Files likely touched:** `crates/core/src/model.rs`, `crates/api/src/routes/media.rs` (new), `crates/api/src/routes/videos.rs`, `crates/api/src/routes/replies.rs` (new), `crates/api/src/lib.rs`, `crates/api/tests/e2e.rs`
**Estimated scope:** M

### Task 6: Send from the review screen through the upload manager (resumable)

**Description:** **Send** queues the reply in `uploads.js`, which gains a kind field: `video` (today) and `reply` (new endpoints, coach as owner). The review screen shows progress and returns to the queue, and a chip shows while the reply uploads. A failed or interrupted send resumes the same way set videos do.

**Acceptance criteria:**
- [ ] A reply survives a reload or a lost connection mid-upload
- [ ] After sending, the video leaves the queue and Logs shows "replied"

**Verification:**
- [ ] Journey test: send a reply; the queue count drops; cut off a part upload once and see it resume

**Dependencies:** Tasks 4 and 5
**Files likely touched:** `web/js/uploads.js`, `web/js/views/review.js`, `web/js/views/videos.js`, `web/js/views/coach.js`
**Estimated scope:** M

## Checkpoint: Coach side complete
- [ ] A coach records and sends a reply on dev; it uploads and the video leaves the queue
- [ ] Review with the human

---

## Phase 3: Client side and finish

### Task 7: Client sees the reply: on the set, in Messages, with a new-reply marker

**Description:** In the workout's video bar, a set with a reply shows **▶ Coach reply** (marked new until seen). The Messages thread shows the reply comment with a play button and the coach's note. The client's Coach tab shows a dot while a reply is unseen. Playing a reply marks it seen.

**Acceptance criteria:**
- [ ] The client finds the reply from the set and from Messages; it plays on iPhone Safari
- [ ] The new marker clears once played

**Verification:**
- [ ] Journey test: the client sees the reply on the set and in Messages, plays it, and the marker clears

**Dependencies:** Tasks 5 and 6
**Files likely touched:** `web/js/views/workout.js`, `web/js/views/comments.js`, `web/js/app.js`, `web/js/videos.js`
**Estimated scope:** M

### Task 8: Retention, delete rules, journey test, docs, and timing the 5-minute gate

**Description:**
- **Retention:** replies are tagged for retention like their video.
- **Deleting:** when the client deletes their video, its replies go too. A coach can delete their own reply.
- **Tests:** the journey covers the whole loop.
- **Timing:** the review screen logs queue-open-to-sent time (console, plus a field on the reply) so the BRD exit gate can be measured on dev.
- **Docs:** ROADMAP (VR-1..3, section 2 rewritten as built), ARCHITECTURE (keys, API) and README.

**Acceptance criteria:**
- [ ] The lifecycle rule covers replies; deleting a video removes its replies
- [ ] The exit-gate time is recorded for each reply
- [ ] Docs updated

**Verification:**
- [ ] e2e: the delete cascade and tags; the journey runs the full loop; `cfn-lint`
- [ ] Manual on dev: time a real review, start to send, under 5 minutes

**Dependencies:** Task 7
**Files likely touched:** `crates/api/src/routes/replies.rs`, `crates/api/src/routes/videos.rs`, `template.yaml` (if a rule changes), `tests/ui/journey.py`, `docs/*.md`, `README.md`
**Estimated scope:** M
