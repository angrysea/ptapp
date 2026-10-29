# Tasks: Warm-ups and post-workout cardio

Plan and decisions: `tasks/cardio/plan.md`.

## Done (2026-10-29)

- [x] Core: `cardio.rs` (phases, cardio targets, zones, program defaults), new fields on `Block`, `Item`, `Workout`, `Program`, `SetLog` and `Exercise`, validation, cardio and mobility seeds
- [x] Stats and records: warm-up and cardio sets left out of strength numbers; weekly cardio minutes and distance
- [x] API: assignment adds the program's defaults; ids for default blocks; custom exercises carry a type
- [x] Builder: phase per block, add warm-up or post-workout blocks, time/distance items with zone and RPE, the program's "every workout" warm-up and cardio, per-workout opt-out
- [x] Logger: warm-up card with one tick, cardio rows (minutes, distance, average HR, RPE) prefilled from the prescription, countdown timer for timed cardio, zones in bpm; status ignores warm-ups and cardio
- [x] Coach logs, Overview and progress report show cardio
- [x] Unit tests: defaults placement, target bounds, max HR, strength vs cardio stats, modality of seeds

## Open

- [ ] Journey test: program with a default warm-up and post-workout cardio → assign → client ticks the warm-up, logs cardio → coach sees weekly cardio minutes
- [ ] e2e: assigning a program with `defaultBlocks` puts them into each scheduled workout; a log with out-of-range cardio numbers is refused
- [ ] Manual: builder and logger at phone width
