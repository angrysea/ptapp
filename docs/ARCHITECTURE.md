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
| `COACH#{coach}` | `INVITE#{code}` | pending invite (deleted on accept) |
| `INVITE#{code}` | `INVITE` | invite lookup, `ttl` 14 days |
| `COACH#{coach}` | `EXERCISE#{id}` | custom exercise (built-ins are compiled into the binary) |
| `COACH#{coach}` | `PROGRAM#{id}` | whole program document |
| `CLIENT#{client}` | `INTAKE` | intake answers |
| `CLIENT#{client}` | `WAIVER#{rfc3339}` | immutable signature record |
| `CLIENT#{client}` | `ASSIGN#{id}` | program assignment |
| `CLIENT#{client}` | `SCHED#{date}#{id}` | scheduled workout with a workout snapshot |
| `CLIENT#{client}` | `LOG#{date}#{id}` | workout log; numeric `ver` for last-write-wins |
| `CLIENT#{client}` | `COMMENT#{rfc3339}#{id}` | feedback / message |
| `CLIENT#{client}` | `METRIC#{date}-{rand}` | weight / tape / skinfold (with computed result) |
| `CLIENT#{client}` | `PHOTO#{date}-{rand}` | photo record; S3 key `media/{coach}/{client}/photos/…` |
| `CLIENT#{client}` | `AUDIT#{rfc3339}#{rand}` | coach access log, `ttl` 1 year |

"Today's workout" is one `Query` on `CLIENT#id` between `SCHED#{from}` and `SCHED#{to}#~`. Phases 1b and 2 add a GSI (`gsi1pk`/`gsi1sk`) for cross-client queues, such as unreviewed videos sorted oldest first (see ROADMAP).

## Offline model (LG-3)

1. The **service worker** caches the app shell (network first, cache fallback).
2. **Reads**: `api.get(path, {cache:true})` stores the last good response in IndexedDB and serves it when the network fails.
3. **Workout in progress**: every set tick is saved to an IndexedDB draft, so closing the app loses nothing.
4. **Finishing**: the log (with a client-generated id) goes into the **outbox**, keyed `log:{id}` so newer edits replace older queued ones. The outbox flushes on `online`, every 30 s and at start-up.
5. **Server**: `PUT /clients/{id}/logs/{logId}` is an idempotent upsert with condition `attribute_not_exists(pk) OR ver <= :incomingMillis`. An older replay gets `409` plus the current copy and is dropped; the newest edit wins. Phone clocks more than 5 minutes ahead are clamped. A replay that edits the log's date replaces the old-dated item too, so one log id never has two live copies.

## API surface

All routes need `Authorization: Bearer <Cognito ID token>` except `GET /health`.

```
GET  /me                         PUT /me                     POST /coach/register
GET  /invites  POST /invites     GET|DELETE /invites/{code}  POST /invites/{code}/accept
GET  /exercises POST /exercises  PUT|DELETE /exercises/{id}  POST /exercises/{id}/demo-upload
GET  /programs POST /programs    GET|PUT|DELETE /programs/{id}  POST /programs/{id}/copy
GET  /clients                    GET /clients/{cid}
GET|PUT  /clients/{cid}/intake   GET|POST /clients/{cid}/waiver   GET /clients/{cid}/dashboard?weeks=
GET|POST /clients/{cid}/assignments   DELETE /clients/{cid}/assignments/{aid}
GET  /clients/{cid}/schedule?from&to  GET /clients/{cid}/logs?from&to  PUT /clients/{cid}/logs/{lid}
GET|POST /clients/{cid}/comments
GET|POST /clients/{cid}/metrics?kind=  DELETE /clients/{cid}/metrics/{mid}
GET|POST /clients/{cid}/photos         DELETE /clients/{cid}/photos/{pid}
```

## Cost at MVP scale (1 coach, 10 to 50 clients)

Request-priced services stay in or near the free tier: Lambda, the HTTP API (about $1 per million requests), DynamoDB on-demand, and Cognito (free under 10k MAU on the Lite tier). CloudFront serves the static app for cents. Photos are shrunk to 1600 px JPEG in the browser (about 250 KB). **Expect under $5/month before video.** Video storage and egress are the main cost driver (BRD); the Phase 1b plan includes compression, a 90-day archive lifecycle and CloudFront signed URLs to keep the total under the $50/month target.
