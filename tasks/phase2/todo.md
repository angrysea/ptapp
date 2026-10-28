# Phase 2 tasks

## A1: %1RM loads + client maxes (WB-5, ON-4 test lifts)

**Acceptance criteria:**
- [x] A coach can set a program item's load as a % of 1RM instead of a weight
- [x] A coach or client can record a max per exercise (tested, or taken from the best e1RM)
- [x] Assigning resolves % loads from the client's maxes, rounded to 5 lb / 2.5 kg; no max shows "75% 1RM"
- [x] Changing a max updates upcoming, unlogged workouts that use a % load

**Verification:** core unit tests (rounding, resolve); e2e (maxes CRUD, assign resolves, max change re-resolves); journey (set a max, see the resolved load)

## A2: PR detection (LG-5)

**Acceptance criteria:**
- [ ] Saving a log with a done set that beats a rep-max (1-10 reps) or best e1RM records a PR
- [ ] The client sees a celebration on save; client and coach see a PR list
- [ ] The coach can delete a bogus PR

## A3: progression suggestions (WB-6)

**Acceptance criteria:**
- [ ] A coach sets a rule per program item (all reps at or below RPE x → add load or reps, with a cap)
- [ ] After a log, matching rules propose next-week changes as pending suggestions
- [ ] The coach accepts (updates the upcoming workout) or dismisses; pain flags block increases
