# Implementation Plan: Phase 2, coach productivity

## Overview

Phase 2 of `docs/ROADMAP.md`: make a coach fast enough to handle 30+ clients. It covers 17 requirements, so the work is split into slices that each ship on their own (API + UI + tests + deploy), ordered by value to a strength coach and by dependency.

**Exit gate (roadmap):** a coach handles 30+ clients without the app being the bottleneck.

## Architecture decisions

1. **Loads by %1RM resolve at assign time** (WB-5). `Item` gains `loadPct`; a client's maxes live at `CLIENT#{id}/MAX#{exerciseId}`. Assigning a program fills `loadKg` from the max, rounded to the client's plates (5 lb imperial, 2.5 kg metric). Changing a max re-resolves upcoming, unlogged workouts. A workout without a max shows "75% 1RM" and the client picks the weight. The arithmetic is in `crates/core` and unit-tested.
2. **Personal records are computed when a log is saved** (LG-5), compared against `CLIENT#{id}/PR#{exerciseId}#{reps}` items. A rep-max at 1-10 reps, plus best e1RM. New PRs come back in the save response so the client gets a celebration, and are listed for the coach.
3. **Progression suggestions and alerts never change anything on their own** (WB-6, LG-7). They are written as pending items in a coach inbox; the coach accepts or dismisses them. Pain flags block load-increase suggestions (BRD risk).
4. **Scheduled work runs on EventBridge Scheduler → the existing Lambda** (FB-3 reminders, MS-4 automated messages, LG-7 missed-workout checks), not a separate worker. One daily sweep per stage, not one schedule per client, keeps the infrastructure to a single schedule.
5. **No new tables or indexes.** Everything fits the single-table `pk/sk` design, as Phase 1 did.

## Task list (slices, in order)

### Slice A: strength programming (the 12-week block is the reference program)
- [x] A1 WB-5 %1RM loads + client maxes (ON-4's test-lift part)
- [ ] A2 LG-5 PR detection and celebration
- [ ] A3 WB-6 progression suggestions (coach-approved)

### Checkpoint A
- [ ] e2e + journey green; deployed; the 12-week block re-entered with %1RM loads

### Slice B: coach speed
- [ ] B1 FB-4 saved reply snippets (comment box, video reply, `/` shortcut)
- [ ] B2 MS-3 deep links: messages that reference a workout, set or video
- [ ] B3 VS-5 filming guidance per exercise, shown before the camera opens
- [ ] B4 LG-7 alert inbox: pain flags and missed workouts

### Slice C: check-ins and automation
- [ ] C1 FB-3 weekly check-in templates, answers, 4-week comparison
- [ ] C2 MS-4 automated messages + check-in reminders (EventBridge Scheduler daily sweep)
- [ ] C3 BZ-3 habits with streaks on Today

### Slice D: video review
- [ ] D1 VR-4 remaining drawing tools (angle with live degrees, circle, arrow, timer)
- [ ] D2 VR-5 compare with an earlier clip or the exercise demo (side by side, synced)

### Slice E: business and records
- [ ] E1 BZ-2 scheduling: coach availability, sessions, per-user `.ics` feed
- [ ] E2 BZ-6 PDF progress report
- [ ] E3 ON-5 coach intake templates; BM-6 Navy body-fat and external readings

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| %1RM rounding in lb vs kg drifts (stored in kg) | Med | Round in the client's unit, then convert; unit tests for 5 lb and 2.5 kg steps |
| Re-resolving upcoming workouts overwrites a coach's manual edit | Med | Only items that still have `loadPct` and haven't been logged are touched |
| PR noise from typos (e.g. 1000 lb) | Low | Coach can delete a PR; PRs are only from sets marked done |
| Scheduler sweep grows with users | Low | One query per coach partition; fine well past 30 clients/coach |

## Open questions
- Slice E's scheduling: is a calendar feed enough, or is two-way Google Calendar sync needed?
