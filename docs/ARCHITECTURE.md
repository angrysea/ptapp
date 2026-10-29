# Architecture

```
 Phone / desktop browser (PWA)                       AWS
 ┌─────────────────────────────┐   HTTPS   ┌────────────────────┐
 │ index.html + ES modules     │──────────▶│ CloudFront (OAC)   │──▶ S3 web bucket
 │ service worker (app shell)  │           └────────────────────┘
 │ IndexedDB: read cache,      │   OAuth2  ┌────────────────────┐
 │   drafts, write outbox      │◀────────▶ │ Cognito hosted UI  │  (code + PKCE)
 └──────────────┬──────────────┘           └────────────────────┘
                │ Bearer ID token
                ▼
        ┌──────────────────┐ JWT authorizer ┌──────────────────────────┐
        │ API Gateway HTTP │───────────────▶│ Lambda `api` (Rust/axum) │
        └──────────────────┘                └─────┬──────────────┬─────┘
                                                  │              │ presigned PUT/GET (15 min)
                                                  ▼              ▼
                                        DynamoDB (1 table)   S3 media bucket (private)
                                                                 ▲
                                    browser uploads/downloads ───┘ directly
```

## Why these choices

* **One Lambda, one router.** At MVP scale a "lambdalith" has the fewest cold starts and the simplest deploy. Rust on arm64 cold-starts in tens of milliseconds and runs at 256 MB. Routes live in `crates/api/src/routes/*` and can be split into separate functions later without code changes (each is a plain axum handler).
* **DynamoDB, not Postgres.** It needs no VPC, connection pool or idle cost, and every screen is served by one or two key lookups or queries. The relational-looking parts of the BRD (programs, sets) are stored as documents: a program is one item, and a scheduled workout carries a snapshot of its workout.
* **Roles in our own table, not Cognito groups.** `USER#{sub}/PROFILE` holds `role` and `coachId`, so joining a coach or changing roles never needs token reissue.
* **Pure domain crate.** `ptcore` has no I/O. Body-composition maths, schedule materialization and dashboard statistics are unit tested in isolation, and the future AI progression rules (AI-3) belong here too: deterministic numbers, as the BRD requires.

## Multi-tenancy and authorization

Each coach is a tenant (`coachId = coach's Cognito sub`). Every client-scoped route goes through `auth::client_scope`:

* a **client** may access only `/clients/{their own id}/…`;
* a **coach** may access `/clients/{id}/…` only if that client's profile has `coachId == coach`;
* coach-only routes (`/programs`, `/invites`, custom exercises, assignments) call `ctx.require_coach()`.

When a coach reads intake, waiver, metrics or photos, an `AUDIT#` item is written to the client's partition and kept for one year (TTL), covering the BRD security NFR "audit log of coach access". The e2e test asserts that a second coach gets 403 on another coach's client.

## DynamoDB single-table layout

| pk | sk | item |
|---|---|---|
| `USER#{sub}` | `PROFILE` | Profile: role, coachId, name, units, sex, birthDate |
| `COACH#{coach}` | `CLIENT#{client}` | roster entry |
| `COACH#{coach}` | `INVITE#{code}` | pending client invite (deleted on accept) |
| `INVITE#{code}` | `INVITE` | invite lookup (client or coach invite), `ttl` 14 days |
| `ADMIN` | `INVITE#{code}` | pending coach invite from an admin (deleted on accept) |
| `ADMIN` | `COACH#{coach}` | coach list entry, written when a coach joins |
| `ADMIN` | `REQUEST#{email}` | public "request an invitation" queue entry (the landing form's anonymous POST); one per email, `ttl` 30 days so a spammed queue ages out |
| `ADMIN` | `SETTINGS#AUTO` | admin auto-approve switch: when on, a landing request is approved (invited+emailed) on arrival |
| `USER#{coach}` | `COACHPROFILE` | coach profile: private business details (legal name, phone, address, certification numbers, CPR, insurance) and the public profile; read only by the coach and admins |
| `DIRECTORY` | `COACH#{coach}` | public copy of a complete, listed profile (no private fields); the only item the unauthenticated `/public/coaches` routes read |
| `COACH#{coach}` | `EXERCISE#{id}` | custom exercise (built-ins are compiled into the binary) |
| `COACH#{coach}` | `PROGRAM#{id}` | whole program document |
| `COACH#{coach}` | `THREAD#{client}` | message thread summary: last message and each side's unread count; the coach's inbox (`GET /messages`) and both sides' badges read it; a new message also goes to the recipient's AppSync Events inbox channel so open apps refresh |
| `COACH#{coach}` | `REVIEW#{createdAt}#{video}` | unreviewed set video in the coach's review queue (oldest first); deleted when marked reviewed |
| `CLIENT#{client}` | `INTAKE` | intake answers |
| `CLIENT#{client}` | `WAIVER#{rfc3339}` | immutable signature record |
| `CLIENT#{client}` | `ASSIGN#{id}` | program assignment |
| `CLIENT#{client}` | `SCHED#{date}#{id}` | scheduled workout with a workout snapshot |
| `CLIENT#{client}` | `LOG#{date}#{id}` | workout log; numeric `ver` for last-write-wins |
| `CLIENT#{client}` | `COMMENT#{rfc3339}#{id}` | feedback / message |
| `CLIENT#{client}` | `METRIC#{date}-{rand}` | weight / tape / skinfold (with computed result) |
| `CLIENT#{client}` | `PHOTO#{date}-{rand}` | photo record; S3 key `media/{coach}/{client}/photos/…` |
| `CLIENT#{client}` | `VIDEO#{date}-{rand}` | set video (log, item, set index), status uploading/ready; S3 keys `media/{coach}/{client}/videos/{id}.{mp4,mov,webm}` and `.jpg` poster, uploaded as a multipart upload in 8 MiB parts |
| `CLIENT#{client}` | `AUDIT#{rfc3339}#{rand}` | coach access log, `ttl` 1 year |
| `CLIENT#{client}` | `MEAL#{date}#{id}` | food journal: whole meal (item lines with parsed grams and food snapshots); per-day kcal computed on read, `updatedAt` server-stamped |
| `COACH#{coach}` | `FOOD#{id}` | coach's saved staple food, usually with a snapshot ("their usual lunch") |
| `FOODCACHE#{query}` | `CACHE` | food-search results from the USDA proxy, `ttl` 30 days |

"Today's workout" is one `Query` on `CLIENT#id` between `SCHED#{from}` and `SCHED#{to}#~`. Phases 1b and 2 add a GSI (`gsi1pk`/`gsi1sk`) for cross-client queues, such as unreviewed videos sorted oldest first (see ROADMAP).

## Offline model (LG-3)

1. The **service worker** caches the app shell (network first, cache fallback).
2. **Reads**: `api.get(path, {cache:true})` stores the last good response in IndexedDB and serves it when the network fails.
3. **Workout in progress**: every set tick is saved to an IndexedDB draft, so closing the app loses nothing.
4. **Finishing**: the log (with a client-generated id) goes into the **outbox**, keyed `log:{id}` so newer edits replace older queued ones. The outbox flushes on `online`, every 30 s and at start-up.
4b. **Set videos** (`web/js/uploads.js`): a clip is converted on the device (`encode.js`), then saved in IndexedDB's `uploads` store with the multipart upload's id and each part's ETag as it lands. Sending runs on `online`, every 30 s and at start-up, and resumes at the next part. Expired part URLs are re-issued; an upload S3 already discarded starts again. Queued clips are per user and removed at sign-out.
5. **Server**: `PUT /clients/{id}/logs/{logId}` is an idempotent upsert with condition `attribute_not_exists(pk) OR ver <= :incomingMillis`. An older replay gets `409` plus the current copy and is dropped; the newest edit wins. Phone clocks more than 5 minutes ahead are clamped. A replay that edits the log's date replaces the old-dated item too, so one log id never has two live copies.

## API surface

All routes need `Authorization: Bearer <Cognito ID token>` except `GET /health`, `GET /invites/{code}/public` and CORS preflights (`OPTIONS`, which API Gateway would otherwise send through the JWT authorizer and fail; `deploy.sh` smoke-tests this).

**Sign-up is by invitation only.** An invitation stores its email; the emailed link (`#/join/CODE`) opens the app's own sign-up page (`web/js/views/signup.js`), which reads the invitation through the public route, fills in and locks the email, and creates the account with the Cognito user pool API (`SignUp`, `ConfirmSignUp`). Sign-in stays on Cognito's managed login page, opened with `login_hint` so the email is filled in. Accepting the invitation marks it used, after which the link and its lookup return 404.

```
GET  /me                         PUT /me
POST /admin/register             GET /admin/coaches          GET|POST /admin/invites   DELETE /admin/invites/{code}
POST /admin/invites/{code}/resend
GET  /admin/coach-requests  GET|PUT /admin/coach-requests/settings  POST /admin/coach-requests/{email}/invite|deny
POST /public/coach-requests  # the landing page's anonymous ask-to-join form (queue only; signup stays invite-only)
GET  /invites  POST /invites     GET|DELETE /invites/{code}  POST /invites/{code}/accept
GET  /invites/{code}/public      (no sign-in: the sign-up page reads the invited email)
GET  /exercises POST /exercises  PUT|DELETE /exercises/{id}  POST /exercises/{id}/demo-upload  POST /exercises/{id}/demo-confirm
GET  /programs POST /programs    GET|PUT|DELETE /programs/{id}  POST /programs/{id}/copy
GET  /clients                    GET /clients/{cid}
GET|PUT  /clients/{cid}/intake   GET|POST /clients/{cid}/waiver   GET /clients/{cid}/dashboard?weeks=
GET|POST /clients/{cid}/assignments   DELETE /clients/{cid}/assignments/{aid}
GET  /clients/{cid}/schedule?from&to  GET /clients/{cid}/logs?from&to  PUT /clients/{cid}/logs/{lid}
GET|POST /clients/{cid}/comments
GET|POST /clients/{cid}/metrics?kind=  DELETE /clients/{cid}/metrics/{mid}
GET|POST /clients/{cid}/meals          PUT|DELETE /clients/{cid}/meals/{mid}   # food journal
GET  /foods/search?q=                  GET|POST /foods  DELETE /foods/{fid}    # USDA proxy + coach staples
GET|POST /clients/{cid}/photos         DELETE /clients/{cid}/photos/{pid}
GET|POST /clients/{cid}/videos?logId=  DELETE /clients/{cid}/videos/{vid}
POST /clients/{cid}/videos/{vid}/parts   POST /clients/{cid}/videos/{vid}/complete   POST /clients/{cid}/videos/{vid}/reviewed
GET  /videos/queue
```

Media in the private bucket is uploaded with presigned PUTs signed for an exact size (set videos as a multipart upload in 8 MiB parts) and tagged `state=pending` until the API confirms it; a lifecycle rule deletes anything still pending after 2 days. Marking a set video reviewed tags it `reviewed=true`, and another rule deletes those objects `VideoRetentionDays` (default 180) after upload; the API hides such records. Unreviewed videos are never deleted automatically.

## Cost at MVP scale (1 coach, 10 to 50 clients)

Request-priced services stay in or near the free tier: Lambda, the HTTP API (about $1 per million requests), DynamoDB on-demand, and Cognito (free under 10k MAU on the Lite tier). CloudFront serves the static app for cents. Photos are shrunk to 1600 px JPEG in the browser (about 250 KB). **Expect under $5/month before video.** Video storage and egress are the main cost driver (BRD); the Phase 1b plan includes compression, a 90-day archive lifecycle and CloudFront signed URLs to keep the total under the $50/month target.
