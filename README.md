# PT Coach: online personal training app (MVP)

A coach-and-client training platform built from the *Online Personal Training App: Preliminary Requirements* BRD (Oct 5, 2026).

* **Frontend:** a mobile-first HTML PWA (vanilla JS ES modules, no build step) that works offline. One app serves both roles: clients get Today / Progress / Coach / Profile, coaches get Clients / Programs / Exercises / Profile.
* **Backend:** one Rust Lambda (axum router on `lambda_http`, arm64) behind an API Gateway HTTP API with a Cognito JWT authorizer.
* **Data:** a single-table DynamoDB design, a private S3 bucket for photos and videos (presigned URLs only), and S3 + CloudFront for the web app.
* **Infra:** AWS SAM (`template.yaml`).

```
crates/core   pure domain logic: models, skinfold/body-fat equations, scheduling, adherence & strength stats (unit tested)
crates/api    Lambda + local dev server; routes, auth/tenancy checks, DynamoDB/S3 access; e2e test
web/          the PWA (index.html, css/, js/, sw.js)
template.yaml SAM stack: DynamoDB, S3, Cognito, HTTP API, Lambda, CloudFront
scripts/      deploy.sh
tests/ui      Playwright browser journey test
docs/         ARCHITECTURE, DEVELOPMENT, DEPLOY, ROADMAP
```

## What the MVP does

| Area | Implemented (BRD ids) |
|---|---|
| Onboarding | Accounts by invitation only: an admin emails coaches an invitation tied to their address (from invites@gymtxcoach.com via SES); the link opens the app's own sign-up page with the email filled in and locked, password and confirmation with show/hide, and stops working once used. Coaches with no link can ask to join: the landing page's form queues an email for the admin (approve emails the same invitation, deny doesn't; an auto-approve switch approves on arrival, routes/requests.rs) (ON-1); intake questionnaire (ON-2); PAR-Q+-style screening and a signed waiver with timestamp, IP and user agent (ON-3) |
| Library & builder | 34 built-in exercises plus coach custom exercises with cues, tags, demo link or uploaded demo video (WB-1); sets/reps/load/tempo/rest/RPE/RIR/notes (WB-2); straight, superset, circuit, AMRAP, EMOM and interval blocks (WB-3); multi-week calendar builder with copy week, move/copy workout, duplicate program, templates (WB-4) |
| Logging | Today view; per-set reps/load/RPE logging pre-filled from the prescription (LG-1); rest timer plus EMOM/AMRAP/interval timers with beeps and wake lock (LG-2); **fully offline** with an outbox and last-write-wins sync (LG-3); complete/modified/skipped plus comment (LG-4) |
| Coach dashboard | Roster with 28-day adherence and flags (pain, needs clearance, no waiver, no program, missed); per-client adherence, missed dates, weekly volume, e1RM trends, check-ins (LG-6) |
| Set videos | Clients record a set in the app (720p, stops at 60 s) or pick a clip (VS-1), trim it to at most 60 s (VS-2) and it's converted to an H.264 MP4 with a thumbnail on the device; uploads are saved on the device first and resume after lost connections, reloads and offline stretches (VS-3); coaches get a review queue, oldest first, with watch, comment on the set and mark reviewed (VS-4). Reviewed videos are deleted after 180 days (configurable) |
| Feedback | Coach comments on a workout or a single set, plus a message thread (FB-1); post-workout RPE, energy, sleep, soreness and pain with location (FB-2) |
| Body metrics | Weight with 7-day trend and weekly averages (BM-1); tape (BM-2); skinfolds JP3/JP7/DW4 with site guide and figure (BM-3); Siri/Brozek body-fat %, lean and fat mass (BM-4); multiple readings averaged, recorded as coach or client (BM-5); private progress photos with date comparison (BM-7); kg/lb and cm/in (BM-8) |
| Security | Cognito (optional TOTP MFA), per-tenant authorization on every route, private encrypted buckets, 15-minute presigned URLs, audit log of coach access to sensitive client data, DynamoDB PITR |

Not in this build, and planned by phase in **[docs/ROADMAP.md](docs/ROADMAP.md)**: the coach video-reply editor, real-time chat and push notifications, Stripe billing, and everything after that.

## Quick start

* Run it locally without AWS: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)
* Deploy to AWS: [docs/DEPLOY.md](docs/DEPLOY.md)
* How it fits together: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

## Tests

```bash
cargo test                                                     # unit tests (core + api)
E2E_ENDPOINT=http://127.0.0.1:5055 cargo test -p api --test e2e  # API journey against moto (about 100 requests)
python tests/ui/journey.py                                     # browser journey: coach + client, offline sync
```
