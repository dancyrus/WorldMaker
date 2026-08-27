# WO-0006-S3: calibrate, gates, goldens, overlay layer

CONTEXT. Session 3 of 3. S1 and S2 are merged. This session calibrates the force-balance coefficients to the nine acceptance metrics in `docs/plan/plate-physics-model.md` §9, arms them as CI gates, regenerates the goldens (third sanctioned move of the project), adds the Overlay map layer, and reports.

RULES. Same as S1. Branch `feat/plate-physics-s3` from `main`. Calibration runs are L6 only. Verification pass at the end: at most eight short subagents, one per metric group.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch.
2. Calibrate. Vary only `K_SLAB`, `K_RIDGE`, `K_MANTLE`, `C_DRAG`, `C_CONTACT`, `C_TRANSFORM`, `TAU_MY`, `SLAB_DETACH_MY`, and the strength coefficients in §4. Do not add constants. Do not add clamps. Target: all nine §9 metrics inside their ranges at seeds `cyrus` and 42, L6, 2 Gy. Record every trial's coefficients and metric values in `docs/results/plate-physics-calibration.json`. If a metric cannot be reached without a new mechanic, stop calibrating that metric, leave it failing, and report why.
3. Gates. Turn the nine metrics into `crates/worldmaker-sim/tests/plate_physics_gates.rs`, fast enough for CI at L6 (cap the run at 1 Gy if 2 Gy exceeds 60 s on the Air; record which). Replace `liveliness_tests.rs` gate 7.2 with §9 metric 8. Wire into CI.
4. Goldens. Regenerate `GOLDEN_TECTONIC_ELEVATION_L6_SEED42` and `GOLDEN_TECTONIC_PLATES_L6_SEED42` once, remove the `#[ignore]`, and write the decision-log entry in the style of the M3 entry: "third sanctioned golden move, WO-0006". Verify the Phase 0 noise golden is unmoved.
5. Overlay layer. Add `Layer::Overlay`, name "Overlay", in `layers.rs` and `Layer::ALL`. Base: the Plates layer at 40% brightness. On top: each cell with `slab_plate != NONE` is drawn in the slab plate's color, with brightness falling with `t - slab_since_my` over `SLAB_DETACH_MY` so fresh slabs are bright and detached slabs fade out. Respect `viewing_kf`.
6. Screenshots to `docs/media/plate-physics/`: at seed `cyrus`, Draft L6, 2 Gy, Split, Eckert IV: `plates-0500.png`, `plates-1000.png`, `plates-2000.png`, `elevation-2000.png`, `overlay-1000.png`, `velocity-1000.png`.
7. Update `docs/plan/tectonics-design.md` to describe the new model, replacing the old motion, suture, and breakup sections; link the audit and model documents. Update `docs/plan/roadmap.md` status.
8. Run the full workspace suite, the Phase 1 harness, and the plategen gates. Verification pass per RULES.
9. Commit, push, PR titled `WO-0006-S3: calibration, gates, goldens, overlay`. Merge when green. Delete the branch. Tag `v0.2.2`.
10. Report to Dan in plain language, under 400 words: each of the nine metrics with its value at each seed and pass or fail, any metric left failing and the missing physics behind it, the six screenshots, and open questions for Phase 2.

DONE WHEN. Tag `v0.2.2` exists; the gates run in CI; goldens regenerated once; the six screenshots exist; report delivered.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

