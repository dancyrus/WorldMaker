# WO-0006-S2: strength field, suture, rifting, microplates

CONTEXT. Session 2 of 3 for `docs/plan/plate-physics-model.md`, with Dan's amendments A–C (decision-log entry from S1). S1 is merged. This session lands model §3–§6 and deletes the remaining non-physical mechanics. Goldens stay ignored until S3.

RULES. Same as S1. Branch `feat/plate-physics-s2` from `main`.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch.
2. Suture scar. Add per-cell `suture_at_my: f32` (NEVER_SUTURED default) to the keyframe encoding and round-trip test.
3. Strength field. Add `fn strength(&self, cell) -> f32` per model §4 with the ordering craton > old ocean > young continent > fresh suture or rift. Add amendment B: `g_insulation`, a factor that falls from 1.0 toward 0.5 for continental cells inside a plate whose continental area exceeds 1/3 of the world's continental area, ramping in over 100–300 My since that plate last sutured. Delete `BREAKUP_SUTURE_AGE_MY`. Wire `strength()` into `R_bnd` in `motion_update()`.
4. Suture. Rewrite `update_pair_timers_and_sutures()` per model §3. A pair timer accumulates only while all three hold: continent-on-continent contact along ≥ 30% of the smaller plate's perimeter; mean relative velocity across the contact below `SUTURE_LOCK_CMYR = 0.4`; every cell within 2 rings of the contact on both sides is continental. At `SUTURE_AFTER_MY = 30`, merge smaller into larger and write `suture_at_my = t` on every contact cell. Delete `SUTURE_SLOW_CMYR`, `SUTURE_DECAY_MULT`, `PLATE_FLOOR`, `PLATE_CEIL`.
5. Rift drivers. Replace `maybe_breakup()` with `fn rift_drivers(&self) -> Vec<RiftDriver>` per model §5, evaluated in fixed order: plume under continent ≥ 20 My; back-arc band 200–600 km inboard of a trench whose newest segment has `age_at_subduction_my > 60`; opposing slab pull (two attached segment groups with pull directions ≥ 120° apart). Each driver carries a `stress: f32`. Delete `BREAKUP_AREA_FRACTION`, `BREAKUP_RIFT_SPEED`, `random_tangent`, and the gridlock-breaker block. `maybe_breakup()` no longer takes `master_seed` or `step_idx`.
6. Rift growth with amendment A. Add `fn grow_rifts(&mut self)`: a rift nucleates at the driver cell only if `stress > strength(cell)`; each step it advances up to `RIFT_PROPAGATION_CELLS` cells (set from 50–100 km/My at the active level) along the neighbor of least strength, only while `stress > strength(next)`; otherwise it stalls and is recorded as a failed rift with `rift_age` still accumulating for `apply_collisions_and_rifts()` maturation. Keep the existing maturation constants. When a rift path oceanizes from boundary to boundary, split the plate along it; the halves' speed and pole come from the force balance only.
7. Microplates. Implement the three §6 origins: trench-trapped slice, back-arc basin detaching the arc sliver, ridge jump. Each new plate starts with an empty slab ledger and the force balance. Extend `PLATE_COLORS` in `layers.rs` from 24 to 48 entries, and index plate colors by `id % 48`.
8. Event log. Add `SimState::events: Vec<TectonicEvent>` with variants `Suture { a, b, t, contact_fraction }`, `RiftStart { plate, driver, t }`, `RiftFailed { plate, t }`, `Split { parent, child, driver, t }`, `Microplate { id, origin, t }`. The probe writes the event list to its JSON.
9. Tests. Unit tests: a pinprick contact never sutures; a 30% locked continental contact with ocean two rings away does not suture; the same with no ocean sutures at 30 My; a plume under a craton (`strength` ≥ 1.5) does not nucleate; the same under a 100 My suture does; a split plate has two connected components each with a new ridge.
10. Run `cargo test --workspace`; green apart from the ignored goldens. Run the probe at seeds `cyrus` and 42; commit `docs/results/plate-physics-probe-s2-<seed>.json`.
11. Commit, push, PR titled `WO-0006-S2: strength, suture, rifting, microplates`. Merge when green. Delete the branch.
12. Report to Dan in plain language, under 300 words: counts of sutures, rift starts, failed rifts, splits, and microplates per Gy at each seed, next to the WO-0005 baseline, and the one-line paste for S3.

DONE WHEN. PR merged; the eight constants named in steps 3–5 no longer exist; every suture and split in the probe event log carries its condition or driver; `cargo test --workspace` green apart from the ignored goldens.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

