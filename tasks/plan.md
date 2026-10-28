# Implementation Plan: Exercise video submission (VS-1 … VS-4)

## Overview

Clients film a set (or pick a clip from their camera roll) from the workout logger, trim it to at most 60 seconds, and upload it. The upload survives poor gym connectivity and airplane mode. Coaches get a review queue of unreviewed videos, oldest first, and can watch each clip next to the set it belongs to.

This is Phase 1b, section 1 of `docs/ROADMAP.md` (requirements VS-1 to VS-4). It deliberately departs from that section in four places, explained below. VS-5 (filming guidance) and the coach reply editor (VR-*) are out of scope.

**Exit test (from the roadmap):** a 60 s phone clip on throttled 3G resumes after airplane mode, and the coach queue shows it oldest first.

## Architecture Decisions

1. **Settle where trimming and encoding happen with a spike first (Task 1).** The roadmap assumes a server-side ffmpeg worker: a container-image Lambda plus ECR, SQS and S3 event triggers. That is the riskiest infrastructure in the feature. This AWS account sits in an Organization whose service control policies already blocked one resource type, and building container images on Windows adds a Docker step to every deploy.
   - **Preferred:** trim and re-encode **in the browser** with WebCodecs (H.264 MP4, plus a poster frame drawn to a canvas), then upload the final file. The server only stores and serves it, so there is no transcoding infrastructure.
   - **Fallback when a browser can't** (no WebCodecs, or it can't decode the input, such as HEVC on some Android devices): upload the original and store the trim points. The player then clamps playback to them.
   - **Server transcoding** becomes a later task, and only if the spike shows the fallback is common or unplayable for coaches.
2. **Resumable uploads use S3 multipart** with 8 MB parts.
   - Each part URL is presigned for its exact length, so S3 enforces sizes, as with today's `util::presign_put`.
   - The multipart upload is created with the existing `state=pending` tag. The existing lifecycle rules then abort abandoned multipart uploads after 2 days and expire unconfirmed objects.
   - Completion is the existing upload-then-confirm pattern: the API checks the object exists, removes the tag and marks the video ready.
3. **The review queue is a per-coach item, not a new index.** A ready video writes `COACH#{coach}` / `REVIEW#{createdAt}#{videoId}`, which sorts oldest first under the coach's partition. Marking a video reviewed deletes that item. This needs no table change, unlike the roadmap's GSI.
4. **Playback uses presigned S3 GET URLs**, the same as progress photos. They support range requests, so seeking works. CloudFront signed URLs (cheaper egress at scale) are deferred to the coach video-reply work, where heavy playback starts.
5. **A video belongs to a set:** `CLIENT#{cid}` / `VIDEO#{id}` with `logId`, `itemId`, `setIndex` and `exerciseId`.
   - The log id is generated in the browser and already exists in the workout draft. Filming mid-workout, even offline, can therefore reference a log that hasn't synced yet.
   - Media keys: `media/{coach}/{client}/videos/{id}.mp4` and `…/{id}.jpg` (poster).
6. **Offline uploads use a dedicated upload manager** (`web/js/uploads.js`). It keeps the file blob and the completed part list in IndexedDB and runs alongside the outbox flush loop.
   - Uploads are per user, like the outbox.
   - Media are not cached for offline playback; only uploads must work offline.
7. **Libraries ship without a build step.** The demuxer (`mp4box.js`) and muxer (`mp4-muxer`) are vendored as pinned ES modules under `web/vendor/`. They are loaded only when the trim screen opens, and added to the service worker's offline cache.

## Spike Results (Task 1, done 2026-10-27)

Spike page: https://claude.ai/artifact/QuVA7gQ56VFiK7kwARXuwR (results in its `runs` store).

| Device / browser | Input | Decodes | Encode 720p | Speed | Output plays | Notes |
|---|---|---|---|---|---|---|
| Windows Edge (headless, synthetic clips) | 1920×1080 H.264 + AAC, MP4 and iPhone-style MOV, rotated | yes | ok, 720×1280, audio kept | about 19–20× real time | yes | orientation verified frame-by-frame |
| Windows Edge and Chrome (headless), **real iPhone clip** `IMG_1685.MOV` | 1920×1080 H.264 High ("Most Compatible"), 30 fps, rotation −90°, AAC-LC 48 kHz stereo, 11 Mbps, 10.7 s, plus 5 Apple `mebx` metadata tracks | yes | ok, 720×1280 (3.98 MB) and 1080×1920 (6.91 MB), stereo audio kept | 720p about 19×, 1080p about 13.5× real time | yes; the original also plays as-is | orientation verified frame-by-frame; metadata tracks ignored correctly |
| **iPhone, iOS 18.7, real 1080p clip picked from the library** (`IMG_1685.mov`, Claude app WebKit) | 1920×1080 H.264 High, 30 fps, 10.7 s, 14.7 MB | yes | **ok, 1080×1920, 4.37 MB, stereo audio kept** | **2.3× real time (4.6 s)** | yes; original plays as-is | the deciding run: a phone re-encodes a full-quality clip quickly |
| iPhone Safari, clips **recorded through the page's file input** (4 runs) | 480×360 H.264 Baseline, 24–26 fps, about 0.9 Mbps | yes | ok (720p and 1080p targets give 360×480), audio kept | 11.5–13.4× real time | yes | iOS records at low quality when a page uses `<input capture>`, in Safari too |
| iPhone, iOS 18.7, **inside the Claude app** | 480×360 H.264 Baseline MOV, 24 fps, about 0.9 Mbps | yes | ok, 360×480 | about 14× real time (tiny input) | yes | the page had dropped the audio (fixed in v2: MOV audio config); the input was a compressed copy |

**Decision (proposed): confirm browser encoding.** Trim and re-encode on the device, default to **720p** (smaller and faster; enough to judge form), and keep the upload-the-original fallback for browsers that can't decode a clip. Server transcoding (Task 11) stays out unless HEVC testing shows otherwise.

**Change to the plan: in-app recording.** On iOS, `<input type="file" capture>` hands the page a 480×360 recording, even in Safari, while picking from the library gives the full-quality original. "Record" therefore uses an in-page recorder (`getUserMedia` + `MediaRecorder`, 720p, with the 60 s cap enforced while recording); "Choose from library" stays a file input. New Task 4b.

**Not yet tested:** HEVC ("High Efficiency", the iPhone default). Whether the phone's WebCodecs decodes HEVC, and whether a Windows coach's Chrome/Edge plays an HEVC original on the fallback path. Tested on a real device as part of Task 5.

Findings:
- **The iPhone encoder path works:** WebCodecs is present, and H.264 720p and 1080p encoders are available (iOS 18.7).
- **Watch input quality on iOS:** the Claude app's in-app browser handed the page a 480×360 compressed copy of the video. PT Coach runs in Safari or as a home-screen app, which may pass the full-quality file. **Needs a run in Safari** with a real camera clip, including one recorded in HEVC ("High Efficiency").
- **iPhone `.MOV` audio** stores its AAC config as `mp4a › wave › esds`, which mp4box doesn't parse. Deriving the AAC-LC config from the sample rate and channel count works (verified locally).
- **"Plays as-is" was unreliable on iOS** (media doesn't load until play is requested). Fixed in v2 by attempting muted playback.

## Dependency Graph

```
Task 1: spike: browser trim/encode on real devices ──► decides Tasks 5, 6 (and optional 11)

Video model (core) ─► create / parts / complete API ─► list + presigned playback ─► review-queue items
        │                       │                              │                          │
        │                       ▼                              ▼                          ▼
        │              upload from set row (online) ──► coach watches in Logs ──► coach Videos tab
        │                       │
        │                       ├──► trim + 60 s cap (WebCodecs) ─► fallback (trim points)
        │                       │
        │                       └──► upload manager (IndexedDB, resume, offline)
        ▼
 lifecycle / retention, journey test, docs
```

## Task List

Tasks with acceptance criteria are in `tasks/todo.md`.

### Phase 0: De-risk
- [ ] Task 1: Spike: trim and re-encode a phone clip in the browser on real devices

### Checkpoint: Processing decision
- [ ] Spike results recorded in this file; decision 1 confirmed or revised with the human

### Phase 1: Thin slice (upload one clip, coach watches it)
- [ ] Task 2: Video API: create, presigned part URLs, complete, list (with e2e)
- [ ] Task 3: Client uploads a clip from the set row; coach plays it in the Logs tab

### Checkpoint: Thin slice
- [ ] A clip uploaded online on a phone plays for the coach; all tests green; deployed to dev

### Phase 2: Trim and cap (VS-2)
- [ ] Task 4: Trim screen with in/out handles and a 60 s cap
- [ ] Task 5: Browser re-encode of the trimmed range to H.264 MP4 plus a poster
- [ ] Task 6: Fallback: upload the original with trim points; the player clamps to them

### Checkpoint: Trim
- [ ] Trimmed clips from iPhone and Android play on coach desktop Chrome/Edge and on phones

### Phase 3: Resumable and offline (VS-3)
- [ ] Task 7: Upload manager: parts and progress in IndexedDB, resume after reload
- [ ] Task 8: Offline queueing: film offline, upload when back online; a client can delete their video

### Checkpoint: Resumable
- [ ] Roadmap exit test part 1: a 60 s clip on throttled 3G resumes after airplane mode

### Phase 4: Coach review queue (VS-4)
- [ ] Task 9: Review queue API (oldest first, mark reviewed) and a coach Videos tab

### Checkpoint: Complete
- [ ] Roadmap exit test passes end to end on dev; journey test covers upload → queue → reviewed

### Phase 5: Hardening
- [ ] Task 10: Retention lifecycle, limits, journey test and docs
- [ ] Task 11 (only if the spike requires it): server transcode fallback

## Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Browser can't decode the camera's format (iPhone HEVC on Android/Windows Chrome) or lacks WebCodecs | High | Task 1 measures this on real devices before anything else is built; fallback path (Task 6); server transcode only if needed (Task 11) |
| Re-encoding a 60 s 1080p clip on a phone is too slow or drains battery | Med | Spike measures it; encode at 720p if 1080p is slow; show progress and allow cancel |
| Org SCPs block infrastructure we add | Med | The preferred design adds no new resource types (only S3, DynamoDB and Lambda, which deployed fine); Task 11 would check ECR/SQS in a throwaway stack first |
| IndexedDB quota or eviction on iOS loses a queued clip | Med | Request persistent storage; keep a clip only until its upload completes; tell the user if storage is refused |
| Large originals on the fallback path (4K, about 400 MB/min) | Med | Size cap per upload (proposed 500 MB); warn before uploading over cellular |
| Egress cost as coaches rewatch clips | Low now | Presigned S3 for now; CloudFront signed URLs with the reply editor |

## Open Questions

1. **Retention (default adopted: 180 days after upload, once reviewed; template parameter `VideoRetentionDays`):** how long should videos be kept? The BRD calls for a video-retention policy. Proposal: keep reviewed videos 180 days, then delete them; never delete unreviewed ones automatically.
2. **Sources:** should clients both record in the app and pick from their camera roll? Proposal: both. Camera roll matters when a training partner films.
3. **Cap:** the BRD says 60 s. Is that a hard limit, or should coaches be able to raise it per exercise?
4. **Who can delete:** proposal is that the client can delete their own videos, and the coach can mark them reviewed but not delete them.
5. **Notifications:** coaches get no alert for new videos until push notifications (MS-2). Is a queue count on the Videos tab enough for now?
