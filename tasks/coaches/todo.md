# Tasks: Coach profiles and a public coach directory

Plan and decisions: `tasks/coaches/plan.md`.

## Task 1: Coach profile API
- [x] `CoachProfile` model with `problems()` validation and `public()` view in `ptcore` (unit tests)
- [x] `GET/PUT /me/coach-profile` (coach only); saving updates `Profile.name`; writes or deletes the `DIRECTORY` entry
- [x] `POST /me/coach-profile/photo` presigned upload; a saved `photoKey` must be the coach's own
- [x] `GET /public/coaches`, `GET /public/coaches/{id}` (no auth; template route `/public/{proxy+}`)
- [x] `GET /me` reports `coachProfileComplete`
- [x] e2e: validation errors, public JSON has no private fields, unlisted and hidden coaches are absent, clients get 403 on the coach routes

## Task 2: Coach profile form
- [x] Form sections: business details (private), public profile, certifications (add and remove rows), insurance, listing
- [x] Shown after accepting a coach invitation and for coaches with an incomplete profile; editable from the Profile tab
- [x] Photo picked, shrunk and uploaded
- [x] Journey: a new coach must complete the form before the roster appears

## Task 3: Public coach list and page
- [x] `#/coaches` and `#/coaches/{id}` work signed out and signed in
- [x] "Find a coach" on the landing page; "Browse coaches" on the client's Profile; "View my public page" for coaches
- [x] Email shown on tap, with a mailto link
- [x] Journey: a signed-out browser lists the coach, opens the page and reveals the email

## Checkpoint
- [ ] Deployed to dev; a real coach profile shows publicly; review with the human

## Task 4: Admin and finish
- [x] Admin opens a coach's full profile; hide or show in the directory
- [x] Docs (ARCHITECTURE data model, ROADMAP), CI green, deployed
