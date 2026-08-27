# WO-0005: plate physics diagnosis and model proposal

CONTEXT. Dan's ruling (2026-08-27): sim behavior must come from real physics and geology. No mechanic may exist "for the sake of happening". Observed defects at seed `cyrus`, Draft L6, 2 Gy: plates weld into one or two giant plates by ~1 Gy; plates leave disconnected fragments (exclaves); plates slow within a few steps of any contact; rifting appears without a physical driver. This session diagnoses and proposes. It changes no sim code.

RULES. Single-track. Branch `docs/plate-physics-model` in worktree `../WorldMaker-physics`, created from `main`. May run at the same time as WO-0004. Checkpoint commits every 30–45 minutes. Probes are dev tests or examples; they do not change `crates/worldmaker-sim/src`. No subagents. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

0. On `main`: run `git pull --ff-only origin main`. If `docs/work-orders/WO-0005-plate-physics-diag.md` is not yet committed, wait for WO-0004 step 0 and pull again.
1. Create the worktree: `git worktree add ../WorldMaker-physics -b docs/plate-physics-model main`.
2. Read `crates/worldmaker-sim/src/tectonics/step.rs`, `setup.rs`, `keyframe.rs`, `mod.rs`, and `docs/plan/tectonics-design.md`.
3. Write `docs/plan/plate-physics-audit.md`. It has one table with one row per mechanic in `step.rs`. Columns: mechanic; function or constant names; what it does; physical basis (cite the real process, or write "none"); verdict (keep / rework / remove). Mechanics to cover at minimum: `motion_update()`, `SLAB_PULL_GAIN`, `COLLISION_DAMP`, `COLLISION_SATURATION`, `SPEED_FLOOR_JAMMED`, `SPEED_MAX`, `POLE_WALK_DEG`, `update_pair_timers_and_sutures()`, `SUTURE_SLOW_CMYR`, `SUTURE_AFTER_MY`, `SUTURE_DECAY_MULT`, `BREAKUP_SUTURE_AGE_MY`, the breakup/rifting trigger, and the cell-ownership update that can produce exclaves.
4. Measure. Write a probe that runs `run_history` at seed `cyrus` and seed 42, L6, 2 Gy, defaults, and records per 100 My: alive plate count; number of sutures; number of breakups; number of plates with more than one connected component, and the cell count of each fragment; mean plate speed; number of plates with speed below 0.05 deg/My. Write results to `docs/results/plate-physics-probe-<seed>.json`. Put the headline numbers in the audit.
5. Trace one suture and one breakup event from the probe: which condition fired and why. Record in the audit.
6. Write `docs/plan/plate-physics-model.md`, the proposed model. Required sections:
   6.1 Force balance. Per plate: slab pull proportional to slab area, ridge push proportional to ridge length, basal drag proportional to plate area, boundary resistance at collisions. Speed is the balance, relaxed with a timescale of 20–50 My. No momentum term. A plate with no slab and no ridge slows to a residual drift.
   6.2 Slab ledger. Per plate, track subducted area and its age. Per cell, record which plate's slab lies beneath it and when it went under. This record is the data for a future "Overlay" map layer. Slab detachment: a slab older than a set age stops pulling.
   6.3 Suture. A pair may suture only when all three hold: continent-on-continent contact along ≥ 30% of the smaller plate's perimeter; relative velocity across the contact below a threshold; no oceanic crust remaining on either side of the contact. State each threshold and its source.
   6.4 Lithosphere strength field. Per cell, from existing fields: crust age, thickness, time since last suture. State the function. Old thick cold crust is strong; young, thin, or recently sutured crust is weak.
   6.5 Rifting. Allowed drivers only: a hotspot plume under continent; back-arc extension behind a subduction zone; opposing slab pull on two sides of one plate. A rift starts only where a driver exists and follows the path of least strength. No random breakup.
   6.6 Microplates. Created only by: a plate slice trapped between a trench and a consumed ridge; back-arc basin opening; ridge jump. Orphaned cells from ownership updates are never a microplate; they are reassigned to the surrounding plate.
   6.7 Exclave fix. State the rule that keeps each plate a single connected region and where in the step it runs.
   6.8 Gaps. List every real process the model still lacks (examples: rock type, mantle flow, flat-slab subduction, terrane accretion). For each: one line on what it does on Earth and one line on what it would take. Do not add a mechanic for any of them.
   6.9 Acceptance metrics for the implementation session: plate count drift over 2 Gy, suture frequency, breakup frequency, zero exclaves, fraction of collisions that build relief, all with target ranges and their Earth-based justification.
7. Add an entry to `docs/plan/decision-log.md` dated today: "Plate physics: model proposed, awaiting Dan's ruling. See plate-physics-model.md."
8. Commit, push, open a pull request titled `WO-0005: plate physics audit and model proposal`. Merge when CI is green (docs and dev-test only; sim hashes must be unmoved). Run `git worktree remove ../WorldMaker-physics`.
9. Report to Dan in plain language, under 400 words: how many mechanics had no physical basis, the three worst, the probe headline numbers, and the questions he must answer before implementation.

DONE WHEN. Both documents merged on `main`; probe JSON committed; sim code untouched; report delivered.


FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
