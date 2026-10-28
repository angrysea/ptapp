# Implementation Plan: Coach profiles and a public coach directory

## Overview

Coaches fill in a detailed profile when they create their account: private business details for the admin, and a public profile for people looking for a coach. Anyone visiting gymtxcoach.com can browse the list of coaches and open a coach's page, which shows the coach's email to contact them. The coach then invites the person through the existing client invitation, so client sign-up stays invitation-only.

## Decisions from review (2026-10-28)

- **Public directory, no account needed** to browse coaches or see their email.
- **No messaging from the profile page:** it shows the coach's contact email (a mailto link) instead.
- **Tax ID is not stored by the app.** Stripe collects and verifies it for payouts when payments go live (BZ-1). Sign-up skips it.
- **Chat continues** after this work (`tasks/chat/`).

## Architecture Decisions

1. **What the profile collects.** Everything is required unless marked optional.
   - **Private** (the coach and admins only):
     - legal name;
     - phone;
     - mailing address (street, optional second line, city, state, ZIP, country);
     - certification numbers;
     - CPR/AED certification expiry;
     - liability insurance (provider, policy number, expiry) (optional).
   - **Public:**
     - display name;
     - photo (optional, but encouraged);
     - headline;
     - bio (at least 80 characters);
     - specialties (at least one, from a list plus "other");
     - certifications, name and issuer (at least one);
     - years of experience;
     - training formats (in person, online; at least one);
     - city and state, from the address;
     - languages;
     - rates (optional free text, e.g. "From $60/session");
     - contact email (defaults to the sign-in email);
     - website and Instagram (optional);
     - accepting new clients (yes or no).
   - **Listing:** a "Show me in the coach directory" switch, on by default once the profile is complete.

2. **Two items, so private fields can't leak.**
   - **The full profile:** `USER#{coach}` / `COACHPROFILE`, readable only by the coach and admins.
   - **The public entry:** saving a complete, listed profile writes a separate `DIRECTORY` / `COACH#{coach}` item holding only the public fields. Unlisting deletes it.
   - **Public reads only ever touch the `DIRECTORY` partition** (one query for the list, one get for a page), so no private field is reachable from them.

3. **Validation lives in `ptcore`** (`CoachProfile::problems()`), unit-tested, and the API refuses an incomplete save with the list of what's missing. The form checks the same rules before sending.

4. **Public routes** are `GET /public/coaches` and `GET /public/coaches/{id}`, on an unauthenticated API Gateway route (`/public/{proxy+}`, GET only).
   - **Photos:** the media bucket stays private. Responses carry presigned photo URLs (1 hour).
   - **Email scraping:** the email comes in the API response but the page shows it only after a tap ("Show email"), which keeps it out of the initial HTML that simple scrapers read.

5. **Coaches must finish their profile before using the app.** After accepting a coach invitation, and for existing coaches whose profile is incomplete, the app opens the profile form instead of the roster. `GET /me` reports `coachProfileComplete`. The profile also takes over `Profile.name` (the display name), which clients and invitations show.

6. **Pages**
   - **Public coach pages** use the app's own router, so they work signed out too: `#/coaches` lists coaches and `#/coaches/{id}` is one coach's page.
   - **Links:**
     - "Find a coach" on the landing page;
     - "Browse coaches" for signed-in clients (on their Profile tab);
     - "Edit coach profile" and "View my public page" on a coach's Profile tab.

7. **Admin view:** the admin's Coaches page opens each coach's full profile, including private details. The admin can hide a coach from the directory (`hiddenByAdmin`), for example to check certifications first.

## Task List

### Phase 1: Profile and directory
- [ ] Task 1: Coach profile API: model and validation, save and read, photo upload, directory entry, public routes
- [ ] Task 2: Coach profile form: required at setup, editable from Profile
- [ ] Task 3: Public coach list and coach page; links from the landing page and the client's Profile

### Checkpoint: Directory works
- [ ] A coach completes the profile on dev; a signed-out browser finds them in the list and sees their email; private fields never appear in public responses
- [ ] Review with the human

### Phase 2: Admin and finish
- [ ] Task 4: Admin coach details and hide-from-directory; docs and roadmap

### Checkpoint: Complete
- [ ] All tests and CI green; deployed to dev; review with the human

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| A private field leaks into the public API | High | Separate directory item built from an explicit public struct; e2e asserts that public JSON has no phone, address, legal name or certification numbers |
| Public email scraped for spam | Low | Shown on tap only; the coach can use a separate contact email |
| Unverified certifications on public pages | Med | Admin can hide a coach until checked; coaches give certification numbers for checking |
| Public routes hammered | Low | Two cheap DynamoDB reads; API Gateway throttling can be added if needed |
| Existing coaches suddenly blocked by the new form | Low | It's a one-time form, and their current name pre-fills it |

## Open Questions

None blocking.
