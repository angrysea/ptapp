# Implementation Plan: Coach video reply editor (VR-1 … VR-3)

## Overview

A coach opens a client's set video from the review queue (or Logs), studies it in slow motion and frame by frame, marks the parts worth talking about, and records a **video reply**: the clip as the coach plays, pauses, slows and steps through it, with the coach's voice over it. The client gets the reply on the set and in their message thread. Sending a reply marks the video reviewed.

This is Phase 1b, section 2 of `docs/ROADMAP.md` (requirements VR-1 to VR-3). Drawing tools (VR-4), compare (VR-5), the coach's camera picture-in-picture (VR-6) and the skeleton overlay (VR-7) are Phase 2 and later, but the design below is chosen so they slot in.

**BRD exit gate:** a coach reviews a video in under 5 minutes end to end, measured from opening the queue to the reply being sent.

The video submission plan (`tasks/plan.md`, `tasks/todo.md`) still has real-device checks open, so this plan lives in `tasks/reply/`.

## Decisions from review (2026-10-28)

- **Approach approved:** record what the coach sees in the browser (decision 1), no server rendering.
- **Clip sound:** kept, **lowered under the coach's voice** (mixed with Web Audio, about 25% volume). The coach hears it lowered too. This replaces decision 2.
- **Reply cap:** 5 minutes.
- **Text-only replies:** allowed (a written reply without recording).
- **Coach devices:** computers, iPads **and phones**; the review screen must work by touch at phone width.
- **Browsers:** some can be required or blocked. Firefox (WebM-only recording) is blocked for recording replies, with a message naming Chrome, Edge or Safari.
- **Captions:** no answer; deferred as proposed (decision 7).

## Architecture Decisions

1. **Record what the coach sees, in the browser, instead of rendering an edit list on the server.** The roadmap plans an edit list (play, pause, seek and rate events against the voice clock) rendered by an ffmpeg Lambda. That needs the container-image Lambda, ECR, SQS and Docker build that the submission work avoided, the infrastructure most likely to hit this AWS organization's policies.
   - **Proposed:** the review screen draws the clip onto a `<canvas>` every frame. Recording captures that canvas (`canvas.captureStream()`) plus the microphone with `MediaRecorder`, so the reply is exactly what the coach saw and heard, as they did it.
   - **Pros:** no server rendering, no queue, no wait for a render. The reply is ready the moment the coach stops, which helps the 5-minute gate. Drawings (VR-4), a side-by-side compare (VR-5) and the coach's webcam in a corner (VR-6) are just more things drawn on the same canvas.
   - **Cons:** a reply can't be re-edited after recording; the coach re-records instead. The reply is as long as the coach talks, so it's capped (proposed: 5 minutes). Output format depends on the coach's browser. Chrome/Edge (126+) and Safari record H.264 MP4; Firefox records WebM, which iOS may not play, so Firefox is either unsupported or its replies get converted (Task 1 decides).
   - **To verify first (Task 1):** the clip comes from S3 on another origin. Drawing it on a canvas *taints* the canvas unless the `<video>` uses `crossorigin="anonymous"` and S3 answers with CORS headers for GET (the media bucket's CORS already allows the app's origin). A tainted canvas can't be recorded at all.
2. ~~The clip's own sound is off in replies.~~ Superseded: the clip's sound is mixed in lowered under the coach's voice (see Decisions from review). This needs `createMediaElementSource` on the clip, which also requires the S3 CORS response, like the canvas.
3. **Segments (VR-3) are a playlist for the recording.** The coach marks in/out ranges and orders them. While recording, Play runs through the segments in order, jumping between them. Without segments, the whole clip is the one segment.
4. **Replies reuse the set-video upload path.** A reply is a video file uploaded the same way: a multipart upload tagged pending, then complete, which removes the tag. The server's multipart helpers in `routes/videos.rs` move to a shared module. `web/js/uploads.js` learns a second kind of upload, so replies also resume after a lost connection.
   - **Storage:** `CLIENT#{cid}` / `REPLY#{id}` (`videoId`, coach, key, poster, duration, `createdAt`, `seenAt`).
   - **Media keys:** `media/{coach}/{client}/replies/{id}.mp4`.
5. **Sending a reply marks the client's video reviewed** (`reviewedAt`, queue item removed, retention tag). It also adds a comment to the client's thread that links the reply, so it shows in Messages and on the set. Comments gain an optional `replyId`.
6. **Retention:** replies follow their video. They're tagged `reviewed=true` too, so the same lifecycle rule deletes them `VideoRetentionDays` after upload.
7. **Captions (accessibility NFR) are deferred.** Transcription needs Amazon Transcribe or the Phase 3 AI box. The reply still has a text comment field the coach can fill in. (Open question 3.)

## Dependency Graph

```
Task 1: spike: canvas capture + mic in coach browsers ──► confirms decision 1 (or falls back to server render)

Review screen (player, VR-1) ─► segments (VR-3) ─► record reply (VR-2) ─► send reply ─► client sees reply
                                                        │                    │
                                       reply API + shared multipart ─────────┘
```

## Task List

Tasks with acceptance criteria are in `tasks/reply/todo.md`.

### Phase 0: De-risk
- [ ] Task 1: Spike: record a canvas-drawn S3 clip plus microphone in the coach's browsers

### Checkpoint: Recording decision
- [ ] Spike results recorded here; decision 1 confirmed or revised with the human

### Phase 1: Watch closely and mark (VR-1, VR-3)
- [ ] Task 2: Review screen: slow motion, frame stepping, keyboard shortcuts
- [ ] Task 3: Segments: mark in/out, reorder, play through them

### Checkpoint: Review screen
- [ ] A coach can study a clip at 0.25× and frame by frame, and play marked segments in order

### Phase 2: Record and send (VR-2)
- [ ] Task 4: Record a reply: canvas plus voice, with preview, re-record and the 5-minute cap
- [ ] Task 5: Reply API: create/parts/complete via shared multipart helpers; sending marks the video reviewed
- [ ] Task 6: Send from the review screen through the upload manager (resumable)

### Checkpoint: Coach side complete
- [ ] A coach records and sends a reply; it uploads, and the video leaves the queue

### Phase 3: Client side and finish
- [ ] Task 7: Client sees the reply: on the set, in Messages, with a new-reply marker
- [ ] Task 8: Retention, delete rules, journey test, docs, and timing the 5-minute gate

### Checkpoint: Complete
- [ ] BRD exit gate measured on dev: queue open to reply sent in under 5 minutes
- [ ] All tests and CI green; deployed; review with the human

## Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| The canvas is tainted by the cross-origin S3 video, so it can't be recorded | High | `crossorigin="anonymous"` plus the bucket's GET CORS; Task 1 proves it on dev, not just locally |
| A coach's browser records only WebM (Firefox), which iPhones may not play | Med | Task 1 checks; then either block Firefox with a clear message or convert the reply with WebCodecs before upload |
| Dropped or uneven frames in the recording at slow speeds or while stepping | Med | Draw every animation frame (`requestAnimationFrame`) regardless of playback state, so the stream runs at a steady rate |
| No microphone, or permission denied | Med | Clear message; allow a reply with a text comment only (open question 4) |
| A reply can't be edited after recording | Low | Preview before sending, with Re-record; replies are short |
| Coaches on iPad Safari: canvas capture + MediaRecorder support | Med | Included in Task 1's device list |

## Open Questions

1. **Clip sound in replies:** off (proposed), or kept quietly under the coach's voice?
2. **Reply length cap:** 5 minutes proposed. Enough?
3. **Captions:** OK to defer until transcription exists (Amazon Transcribe now, or the Phase 3 AI box)?
4. **Text-only replies:** allow sending just a written comment without recording, for when the coach has no microphone handy? (They can already comment on a set.)
5. **Coach devices:** mainly laptops/desktops, or also iPad and phone? This decides how much the review screen must work by touch.
