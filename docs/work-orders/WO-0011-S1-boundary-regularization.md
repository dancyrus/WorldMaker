# WO-0011-S1: strength-gated boundary regularization (anti-fray)

CONTEXT. Plate-shape fix, session 1 of 3. Runs BEFORE WO-0009-S2 (Dan's sequencing ruling, 2026-08-28). Diagnosis and strategy are in the Cowork docs `plate-shape-diagnosis-v1` and `plate-shape-fix-strategy-v1` (Project); the short version: `advect()` re-samples the plate-id field across each inter-plate velocity discontinuity, shear stretches the interleaved teeth into strips, and nothing resists it — the pipeline enforces connectedness, never compactness. The diagnostic is committed at `crates/worldmaker-sim/tests/plate_shape_probe.rs`. This WO changes plate_id trajectories, so the tectonic goldens go `#[ignore]` here and regenerate ONCE at the end of S2 — the sixth sanctioned move, announced now (WO-0008 S0 precedent). WO-0009-S2's move is renumbered to seventh in the same install commit.

RULES. Single-track. Branch `feat/plate-shape` from `main`. S1 and S2 run on this one branch; the PR opens at the end of S2. Checkpoint commits every 30–45 minutes. Determinism rules apply: serial cell-id order, f64 accumulation, no wall-clock RNG. No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. (This WO, its siblings, and the probe are already committed.)
2. New pass `regularize_boundaries()` in `step.rs`, called after `enforce_connectivity()` and before `classify_boundaries()`. The physics: a plate boundary localizes on weak lithosphere; propagating one through strong interior requires stress the driving forces do not supply (Vauchez et al. 1997). So a re-sampling flip stands only where the lithosphere can actually fail:
   2.1 Candidates: cells whose owner changed this step (advect holds the previous field). EXEMPT any cell whose previous-step `features` carry a process-edge bit (`F_BND_DIVERGENT`, `F_BND_CONVERGENT`, `F_BND_TRANSFORM`) — real ridges, trenches and transforms are never straightened. Previous-step classes are the correct input by construction: `classify_boundaries()` runs after this pass, the same one-step-old convention `advect()`'s `was_transform_only` already uses.
   2.2 Failure test (Dan's ruling: normal distribution). A candidate keeps its new owner only when `strength(c) < STRENGTH_FAIL_MEAN + eps`, where `eps` is a per-(cell, step) draw from `N(0, STRENGTH_FAIL_SIGMA^2)` computed by deterministic seeded hash (`worldmaker_core::hash` over master seed, cell id, step index). Same seed, same world: the motion path stays RNG-free in the reproducibility sense.
   2.3 Geometry test. A candidate that passes 2.2 still reverts when keeping it increases the local strength-weighted boundary energy: `E = sum over inter-plate edges of the cell of (strength(c) + strength(nb)) / 2`, evaluated for the new owner vs the previous owner. Lower E wins; a tie keeps the previous owner. Strong-interior teeth lose to the straight interface; weak young margins keep their irregularity.
   2.4 Craton floor. A cell in the craton strength regime (`cont && age_ref >= OROGENY_RELAX_MAX_AGE_MY`, exactly the `strength()` branch) never transfers, EXCEPT at an active trench consuming its plate under the per-pair subduction polarity rule already in `advect()`. Count violations in a `craton_transfer_violations` counter (instrumentation; S3 arms it at zero).
   2.5 Serial, cell-id order, iterate to a fixed point with a pass cap of 8 (seam-repair convention). Re-run `enforce_connectivity()` after the pass — reverts can re-fragment a plate.
3. Constants with doc comments, in `strength()` units: `STRENGTH_FAIL_MEAN` (the strength at which lithosphere can fail and change plates; anchor between young-ocean and continental-platform strength), `STRENGTH_FAIL_SIGMA` (sub-grid strength heterogeneity; start near 15% of MEAN). Calibrate both at L6 seed cyrus until step 6 passes without erasing young weak margins. The craton floor reuses the existing regime; no new constant.
4. Lift the probe's series into `tectonics/metrics.rs` as reusable functions: mean boundary/area (compact), finger fraction (<=2 of 6 same-plate neighbours), largest-plate share. The probe calls them; behavior unchanged; S3 gates them.
5. `#[ignore]` the tectonic goldens with a note naming the sixth sanctioned move. The Phase 0 noise golden must stay green and unmoved.
6. Probe run, L6, 24 plates, land 0.40, vigor 1.0, 2 Gy, seed cyrus. Required: compact at 2 Gy <= 1.15 x its 100 My value (no monotone fray), finger fraction <= 0.5% at every sample after 100 My. Largest share and alive count are NOT required here — welding is S2's job. Record the full table for the report.
7. Run `cargo test --workspace`: green except the announced `#[ignore]` goldens. Commit, push. No PR yet.
8. Report to Dan, under 300 words: before/after probe table, the calibrated MEAN and SIGMA, one sentence on what the margins look like, and the S2 paste line.

DONE WHEN. Branch pushed; regularization pass in with all four sub-rules; probe table shows compact flat and fingers <= 0.5%; workspace green except the announced ignores; report delivered.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
