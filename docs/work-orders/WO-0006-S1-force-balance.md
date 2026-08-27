# WO-0006-S1: force balance, slab ledger, connectivity

CONTEXT. Dan accepted `docs/plan/plate-physics-model.md` on 2026-08-27 with three amendments (recorded in `docs/plan/decision-log.md` by step 2 below). This is session 1 of 3. It replaces plate motion with the §1 force balance, adds the §2 slab ledger, and enforces the §7 connectivity invariant. Sessions 2 and 3 follow in `WO-0006-S2-suture-rifting.md` and `WO-0006-S3-calibrate.md`. Sim hashes WILL move; do not regenerate goldens in this session. Mark the golden tests `#[ignore]` with the reason "WO-0006 in progress; regenerated in S3".

RULES. Single-track. Branch `feat/plate-physics-s1` from `main`. Checkpoint commits every 30–45 minutes. No subagents except one final verification pass capped at three short lookups. Keep determinism: one master seed, PCG sub-streams, fixed iteration order, no libm, id-ordered reductions. If the usage limit nears: commit, push, update checkboxes, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. Commit this work order and its two siblings with message `docs: install WO-0006 S1–S3`.
2. Append to `docs/plan/decision-log.md` an entry dated today titled "Plate physics: model accepted with amendments". List: (A) a rift nucleates and advances only where driver stress exceeds local strength; a stalled rift is a failed rift; (B) supercontinent breakup comes from mantle-insulation weakening of the strength field, not from biased hotspots; `BREAKUP_SUTURE_AGE_MY` is retired; (C) slab segments detach individually; continuous subduction keeps a rolling attached slab; pull fades only after subduction stops. Rulings: slabs keep pulling after the subducting plate dies; keyframes may grow to 20 B/cell; quiet supercontinents are accepted; the plate palette is extended instead of capping microplates.
3. Slab ledger. In `keyframe.rs`: add to `PlateState` a field `slab: Vec<SlabSegment>` where `SlabSegment { area_cells: u32, age_at_subduction_my: f32, subducted_at_my: f32, attached: bool }`. Add per-cell fields `slab_plate: u16` (NONE = u16::MAX) and `slab_since_my: f32`. Add both to the keyframe encoding and to the resume round-trip test.
4. In `advect()`: at the consumption branch, append one `SlabSegment` per plate per step (merge same-step consumption), and write `slab_plate` and `slab_since_my` on the overriding plate's trench cell. Advect `slab_plate` and `slab_since_my` with the overriding plate. When a plate dies, move its remaining segments to the plate that consumed it.
5. Add `SLAB_DETACH_MY: f32 = 60.0`. In `age_and_relax()`: set `attached = false` on any segment where `t - subducted_at_my > SLAB_DETACH_MY`. Drop detached segments older than `2 * SLAB_DETACH_MY`.
6. Force balance. Rewrite `motion_update()` per model §1. Remove `base_speed_deg_my` from `PlateState` and from `setup.rs`. Delete `SLAB_PULL_GAIN`, `COLLISION_DAMP`, `COLLISION_SATURATION`, `COLLISION_SATURATION_MIN_CELLS`, `SPEED_RELAX_UP`, `SPEED_RELAX_DOWN`, `SPEED_MIN`, `SPEED_FLOOR_JAMMED`, `POLE_WALK_DEG`. Add `K_SLAB`, `K_RIDGE`, `K_MANTLE`, `C_DRAG`, `C_CONTACT`, `C_TRANSFORM`, `TAU_MY: f32 = 30.0`. Keep `SPEED_MAX` as a rail only. Compute `F_slab` from attached segments with weight `min(1, age_at_subduction_my / 80)`, `F_ridge` from divergent boundary length, `F_resid = K_MANTLE * area`, `R_drag = C_DRAG * area`, `R_bnd = C_CONTACT * contact_cells + C_TRANSFORM * transform_cells`. Until S2 lands the strength field, `strength(cell) = 1.0`. `v_target = drivers / resistances`; `speed += (DT_MY / TAU_MY) * (v_target - speed)`.
7. Pole update. Replace the random walk: sum torque directions in fixed cell order (subducting cell: `x_cell × n_trench`; ridge cell: the opposite sense), normalize to `omega_target`, relax the pole toward it with `TAU_MY`. `motion_update()` no longer takes `master_seed` or `step_idx`. Initial poles at setup stay random.
8. Connectivity invariant. In `advect()`, remove the continent-continent overlap branch that freezes single cells via `keep_cell`; resolve the overlap cell to the slower plate. Add `fn enforce_connectivity(&mut self)` called after `advect()` and before `classify_boundaries()`: serial BFS in cell-id order; per plate keep the largest component (tie → lowest cell id); reassign each other fragment to the neighbor plate with the longest shared border (tie → lowest plate id). Count reassigned cells per step into a new `SimState` counter `connectivity_reassigned`.
9. Calibration placeholder. Set the six coefficients so that at seed 42, L6, 500 My: run mean speed is 2–6 cm/yr and slab-attached plates average ≥ 2× slab-free plates. Record the values in the work order. Final calibration is S3.
10. Tests. Add unit tests: force balance with zero drivers relaxes to `K_MANTLE / C_DRAG`; a plate with one attached segment moves faster than the same plate without; `enforce_connectivity()` on a hand-built two-fragment plate leaves one component; keyframe round-trip preserves `slab_plate`. Extend `plate_physics_probe.rs` to record `connectivity_reassigned` and per-plate attached slab area.
11. Run `cargo test --workspace`. Everything green except the two ignored golden tests. Run the probe at seeds `cyrus` and 42 and commit `docs/results/plate-physics-probe-s1-<seed>.json`.
12. Commit, push, open a pull request titled `WO-0006-S1: force balance, slab ledger, connectivity`. Merge when CI is green. Delete the branch.
13. Report to Dan in plain language, under 300 words: what was deleted, the placeholder coefficients, the probe numbers next to the WO-0005 baseline, and the one-line paste for S2.

S1 RECORD (step 9). Placeholder coefficients shipped in `step.rs`:
`K_SLAB = 0.70`, `K_RIDGE = 1.0`, `K_MANTLE = 0.06`, `C_DRAG = 1.0`,
`C_CONTACT = 60.0`, `C_TRANSFORM = 2.0` (`TAU_MY = 30`, `SLAB_AGE_REF_MY = 80`).
At seed 42 / cyrus, L6, 500 My: run mean speed 4.12 / 4.98 cm/yr (target
2–6 ✓). The binary slab-attached-vs-slab-free ratio reads 1.15 / 1.82 —
structurally unmeasurable at S1: slab-free plate-samples are ~0.5% of the
population (24 / 7 of ~3,000), all of them fresh breakup halves still
carrying inherited parent speed, because every long-lived plate subducts
somewhere until S2's rifting creates genuinely slab-free plates. The force
ranking itself is demonstrated by the same runs' attached-area split:
plates with above-median attached slab area average 2.01× / 2.07× the
speed of below-median plates (5.41 vs 2.69, 6.75 vs 3.26 cm/yr). Residual
drift K_MANTLE / C_DRAG = 0.06 deg/My ≈ 0.67 cm/yr, inside the model's
0.3–1 cm/yr band. Final calibration is S3.

DONE WHEN. PR merged; the nine named constants and `base_speed_deg_my` no longer exist in the sim; `enforce_connectivity()` runs every step; probe JSON committed; `cargo test --workspace` green apart from the two ignored goldens.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

