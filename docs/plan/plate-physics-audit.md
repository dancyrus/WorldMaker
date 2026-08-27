# Plate physics audit (WO-0005)

Audit of every mechanic in `crates/worldmaker-sim/src/tectonics/step.rs`
against Dan's ruling (2026-08-27): sim behavior must come from real physics
and geology; no mechanic may exist "for the sake of happening". Companion
proposal: `plate-physics-model.md`. Probe data:
`docs/results/plate-physics-probe-cyrus.json` / `-42.json` (written by the
ignored dev test `plate_physics_probe` in
`crates/worldmaker-sim/tests/plate_physics_probe.rs`; it replays the exact
`run_history` trajectory, including the keyframe quantization round-trip).

Verdicts: **keep** = physically grounded as is (values may still be re-derived);
**rework** = the real process exists but the implementation doesn't model it;
**remove** = no physical basis, exists to force behavior or patch a symptom.

## The mechanic table

| # | Mechanic | Names | What it does | Physical basis | Verdict |
|---|----------|-------|--------------|----------------|---------|
| 1 | Intrinsic plate speed | `base_speed_deg_my` (setup.rs), used in `motion_update()` | Every plate gets a random preferred speed \|N(0.5, 0.15)\|·vigor at t=0 and relaxes toward it (scaled) forever. | **None.** A plate has no intrinsic speed; real speed is the quotient of driving forces (slab pull, ridge push) over resistances (basal drag, collision). A plate that loses its slab slows; here it re-accelerates to its birth speed. | remove |
| 2 | Speed update | `motion_update()` | Sets speed toward `base·(1+SLAB_PULL_GAIN·f_sub)·(1−COLLISION_DAMP·f_coll)`, asymmetric relaxation, clamped to [floor, SPEED_MAX]. | Partial. That subduction drives and collision resists is real (Forsyth & Uyeda 1975: trench-attached plates move 5–10× faster). But multiplying a random base speed by boundary *fractions* is not a force balance: forces don't normalize by boundary length, and drag doesn't scale with plate area anywhere. | rework |
| 3 | Slab pull gain | `SLAB_PULL_GAIN = 1.0` | +100% speed at fully-subducting boundary. | Partial. Slab pull is the dominant plate driver (~70–90% of net driving force; Lithgow-Bertelloni & Richards 1998). But it should scale with slab area and age (negative buoyancy), not the *fraction* of boundary subducting, and it should be a force, not a multiplier on a random base. | rework |
| 4 | Collision damping | `COLLISION_DAMP = 1.0` | Zeroes the target speed at saturated collision fraction. | Partial. Continental collision does brake plates (India slowed ~15 → ~5 cm/yr at the Asia collision, ~50 Ma). But as a multiplier it can only slow, never balance: a huge slab and a small contact still gives speed 0. | rework |
| 5 | Collision saturation | `COLLISION_SATURATION = 0.05`, `COLLISION_SATURATION_MIN_CELLS = 4` | A contact of max(5% of boundary, 4 cells) counts as full jam — 4 cells stall a plate of any size. | **None.** Tuned to stop margin-grinding (its own comment says so). Whether a contact stalls a plate is a force ratio (resistance vs drivers), not a fixed cell count. This is the "plates slow within a few steps of any contact" defect. | remove |
| 6 | Asymmetric relaxation | `SPEED_RELAX_UP = 0.15`, `SPEED_RELAX_DOWN = 0.5` | Braking 0.5/step (τ≈3 My), acceleration 0.15/step (τ≈12 My). | Partial. Relaxation itself is right — plates are inertialess (Stokes regime), speeds re-equilibrate over ~10⁷ yr as forces change. The up/down asymmetry is a tuning device with no cited process. | rework |
| 7 | Speed floor | `SPEED_MIN = 0.1` deg/My (~1.1 cm/yr) | No free plate may move slower than this. | **None.** Real plates can be near-stationary (Antarctica ≲ 1 cm/yr in no-net-rotation frames). The floor papers over the grid-freeze numerics problem that the pending-rotation bank already solves. | remove |
| 8 | Jammed creep floor | `SPEED_FLOOR_JAMMED = 0.05` deg/My | A fully jammed plate keeps creeping at 0.05 deg/My forever, explicitly "so welded pairs still read slow and suture". | **None.** Exists solely so a different mechanic (the suture timer) can detect the jam. A jammed plate creeping by decree is motion for the sake of happening. | remove |
| 9 | Speed cap | `SPEED_MAX = 2.0` deg/My (~22 cm/yr) | Hard clamp on any plate speed. | Plausible as a rail: fastest known sustained plate motion is India in the Late Cretaceous, ~18–20 cm/yr. Fine as a safety clamp; must not be the operating point. | keep |
| 10 | Euler-pole random walk | `POLE_WALK_DEG = 0.6`°/step (1σ) | Every plate's rotation pole wanders randomly every 2 My. | **None.** Real poles shift when the torque balance shifts (new slab, collision, ridge reorganization) — a directed response, not noise. This walk is randomness for its own sake and the only reason plate directions change at all. | remove |
| 11 | Sub-cell motion banking | `pending_rot`, `COMMIT_FRACTION = 0.75` | Slow plates bank rotation and commit at ~3/4 cell so they never freeze to the grid. | Numerics, not physics — and correct numerics. Keeps rigid motion exact at any speed. | keep |
| 12 | Ownership scatter/gather | `advect()` — forward scatter + coverage gather | Cells claim destinations; each cell back-rotates candidates to resolve its owner. | Sound advection numerics for rigid rotation. Defect is in special cases (rows 13–14), not the scheme. | keep |
| 13 | Continent jam freeze | `advect()` overlap branch, `keep_cell(...)` on 2 hard covers | A continent-continent overlap cell "freezes in place": it keeps its old owner while its plate rotates on. | Partial. That continental crust cannot subduct is real (too buoyant; Cloos 1993). But a *rigid* plate cannot have individual cells frozen while the rest moves — the frozen patch detaches from its own plate's rigid motion. This (plus wholesale suture/breakup reassignment, rows 18/21) is the exclave factory. | rework |
| 14 | Ridge-gap fill | `advect()` 0-cover branch | A cell nobody claims becomes fresh ridge crust under the *previous owner's* id. | Real process (seafloor spreading fills divergent gaps), reasonable ownership choice. Transform-jitter gating is a grid artifact patch but harmless. | keep |
| 15 | Boundary classification | `classify_boundaries()`, `CLASSIFY_CMYR = 0.4` | Classifies edges divergent/convergent/transform from relative normal velocity. | Standard kinematic definition (DeMets et al. plate-motion practice). 0.4 cm/yr is a sane dead band. | keep |
| 16 | Suture timers | `update_pair_timers_and_sutures()` | Per pair, accumulate time while mean continent-contact convergence < threshold; at 30 My, the smaller plate is absorbed wholesale. | Partial. Suturing is real (Wilson cycle: ocean closes, continents weld — Iapetus/Caledonides, India–Asia). But: welding on *slowness alone* is backwards (two plates barely touching at one edge weld even if 95% of both are open ocean); absorption is instant and total; and the "slow" signal it detects is manufactured by rows 5+8. This is the plates-weld-into-one defect. | rework |
| 17 | Suture thresholds | `SUTURE_SLOW_CMYR = 1.2`, `SUTURE_AFTER_MY = 30` | Slow means < 1.2 cm/yr mean convergence; weld after 30 accumulated My. | Partial. 30 My matches real collision-to-lock times (India–Asia hard collision ~50→20 Ma). But 1.2 cm/yr was chosen (its own comment) to sit *above the artificial jam-creep ceiling* of row 8 — a threshold derived from an artifact, not from geology. | rework |
| 18 | Suture merge action | second half of `update_pair_timers_and_sutures()` | Reassigns every loser cell to the winner in one step; loser dies. | Partial. Two welded plates moving as one is the right end state, but instant total absorption erases the suture zone and can weld non-contiguous cell sets (exclave source; a real suture is a mapped scar with the weakest lithosphere in the system — Vauchez et al. 1997). | rework |
| 19 | Suture-timer hysteresis | `SUTURE_DECAY_MULT = 2.0` | Fast-convergence steps decay the timer at 2× instead of resetting. | **None.** Patch on the model's own speed oscillation (escape flickers), which itself comes from rows 2–8. With a force balance there is no flicker to patch. | remove |
| 20 | Plate count clamps | `PLATE_FLOOR = 6`, `PLATE_CEIL = 24` | Suturing forbidden at ≤ 6 plates; breakup forbidden at ≥ 24. | **None.** Earth's plate count is an outcome, not an input (7 major + ~50 minor today; counts varied through the Wilson cycle). The floor directly causes the gridlock the next row then has to break. | remove |
| 21 | Breakup trigger + geometry | `maybe_breakup()`, `BREAKUP_AREA_FRACTION = 1/3`, `BREAKUP_RIFT_SPEED = 0.3` | A plate over 1/3 of the sphere (or 1/3 of continental crust) with old sutures splits along a *random* great circle through its continental centroid; halves get ±0.15 deg/My by decree. | Partial. Supercontinent breakup is real (mantle insulation + plume push: Pangea, Rodinia; Gurnis 1988). But the trigger is an area quota, the rift line is RNG (real rifts follow sutures and weak zones — the Atlantic opened along the Caledonian/Variscan sutures), and the divergence speed is imposed, not driven. "Rifting appears without a physical driver" is this row. | rework |
| 22 | Breakup quiescence age | `BREAKUP_SUTURE_AGE_MY = 100` | No breakup within 100 My of the last suture. | Plausible stand-in: supercontinents persist ~100–300 My before dispersal (Pangea: assembled ~320 Ma, rifted ~200–180 Ma) because mantle insulation takes ~100 My to build. As a hard gate it's a proxy for a thermal process the model lacks. | rework |
| 23 | Gridlock breaker | second candidate block in `maybe_breakup()` (WO-0003 Fix 4) | At the plate floor with a matured weld timer, force-break the most-continental plate. | **None.** Exists to keep the Wilson cycle turning after rows 16+20 jam it. Its own comment describes it as a breakup↔suture limit cycle — the definition of a mechanic existing so that something happens. | remove |
| 24 | Rift maturation | `apply_collisions_and_rifts()`, `RIFT_ONSET_MY = 20`, `RIFT_THIN_KM_MY = 0.2`, `RIFT_OCEANIZE_KM = 25`, `RIFT_DECAY_MULT = 2` | Sustained continent-continent divergence accumulates rift age; past 20 My the crust thins 0.2 km/My; below 25 km it becomes ocean. | Partial. The maturation pipeline is decent (McKenzie 1978 stretching; rift-to-drift ~10–30 My; breakup at stretching factor ~3). But the *driver* is "the RNG poles happened to diverge here" — there is no plume, no back-arc, no slab-pull tension selecting the site. Keep the maturation, replace the driver. | rework |
| 25 | Volcanic arcs | `apply_arcs()`, `ARC_MIN_KM = 150`, `ARC_MAX_KM = 250`, growth/convert constants | Arc grows 150–250 km inboard of trenches; oceanic arc converts to continent at 20 km. | Real. Arcs sit where the slab reaches ~100 km depth; global mean trench-arc distance ~166 ± 60 km (Syracuse & Abers 2006). Island-arc accretion is a real continental-growth path. | keep |
| 26 | Collision thickening | `COLLISION_THICKEN = 0.12` km/My per cm/yr, `THICKNESS_CAP_KM = 70` | Convergent continent cells thicken with convergence rate; cap 70 km. | Real. Crustal shortening doubled Tibet's crust to ~70 km; thickening tracking convergence rate is the right first-order law. | keep |
| 27 | Orogenic relaxation | `OROGENY_BASE_KM = 38`, 200 My constant, craton exemption | Inactive orogens relax toward 38 km over ~200 My; cratons exempt. | Real. Post-orogenic gravitational collapse + erosion decay dead ranges (Appalachians/Caledonides); cratons persist for Gy. Timescale is the right order. | keep |
| 28 | Hotspots | `apply_hotspots()`, rates/cap, 200 My subsidence | Fixed mantle points build shields; buildup decays as plates drift. | Real. Plume-fed hotspot tracks with subsiding chains (Hawaii–Emperor). Fixed-mantle-frame points are the standard first-order model. | keep |
| 29 | Crust aging/cooling | `age_and_relax()`, age-depth in elevation.rs | Ages advance; ocean deepens with √age. | Real. Half-space cooling (Parsons & Sclater 1977). | keep |
| 30 | Subductible thin continent | `SUBDUCTIBLE_CONT_KM = 30` | Continent under 30 km can be consumed. | Real-ish. Thin, young arc/rifted slivers do subduct or underplate (Cloos 1993 buoyancy analysis: ~30 km is about where net buoyancy flips with a slab attached). | keep |

## Tallies

30 mechanics audited: **13 keep, 9 rework, 8 remove.**
The 8 removals (rows 1, 5, 7, 8, 10, 19, 20, 23) have **no physical
basis** — each exists to force behavior or to patch a defect another
non-physical mechanic created. Three of the reworks (16, 17, 21) are the
direct causes of the observed defects. The dependency chain of patches is
itself diagnostic: 8 (creep decree) exists so 16 (suture) can fire; 17's
threshold exists because of 8; 19 exists because 2–8 oscillate; 23 exists
because 16+20 gridlock; 21's imposed speed exists because nothing else
would open the rift that 23 forced.

## Probe headline numbers (step 4)

Config: L6, 2 Gy, default params (12 plates, land 0.29, vigor 1.0),
seeds `cyrus` (`seed_from_text`) and 42. Full series in
`docs/results/plate-physics-probe-*.json`.

| Metric (2 Gy, per seed) | `cyrus` | `42` |
|---|---|---|
| Alive plates: start → pinned at | 12 → 6–7 by 200 My | 12 → 6–7 by 300 My |
| Largest plate, share of sphere | 87% at 200 My; peak **97.3%** | peak **93.6%** |
| Sutures (total / per 100 My) | 34 / 1.7 | 35 / 1.75 |
| Breakups total | 31 | 30 |
| — by the gridlock breaker | **20** | **21** |
| — by the 1/3-sphere area quota | 11 | 7 |
| — by continental share | 0 | 0 |
| — unattributed (timer matured mid-step) | 0 | 2 |
| Samples (of 21) with ≥ 1 fragmented plate | **20** | **20** |
| Max simultaneously fragmented plates | 7 | 7 |
| Mean plate speed, t=0 → late-run band | 0.54 → 0.11–0.22 deg/My | 0.43 → 0.19–0.34 deg/My |
| Plates below 0.05 deg/My | always 0 — **the clamp forbids it** | always 0 |
| Plates parked exactly at the 0.05 floor | up to 3 at once | up to 1 |

Readings:

- **Welding.** The "one or two giant plates by ~1 Gy" defect is worse than
  reported: at seed `cyrus` one plate already holds 87% of the sphere at
  200 My. The suture rate (one per ~60 My) plus wholesale absorption
  outruns breakup for the entire run.
- **The plate count is pinned by clamps, not dynamics.** From ~300 My on,
  the count sits at the floor of 6, and the Wilson "cycle" is the
  gridlock-breaker limit cycle: **two-thirds of all breakups** (41 of 61
  across both seeds) are the bookkeeping breaker, not the supercontinent
  trigger. Zero breakups fired on continental share.
- **Exclaves are the steady state, not a glitch.** 20 of 21 samples in both
  seeds contain fragmented plates; fragments range from single cells to
  ~500 cells. At `cyrus` t=800 My, plate 4 consists *only* of fragments
  (88 + 84 + 22 cells, no main body).
- **Speeds live on the artificial floors.** Mean speed collapses from
  0.54 to ~0.15 deg/My (≈1.7 cm/yr) as every plate acquires a contact and
  gets damped to a clamp; the WO's "plates with speed below 0.05 deg/My"
  metric reads zero forever because `SPEED_FLOOR_JAMMED` makes speeds
  below 0.05 structurally impossible — itself an audit finding (row 8).

## Event traces (step 5)

**Suture trace — seed `cyrus`, t = 176 My, plates 0 + 1.** Plate 0
(13,889 cells) welded into plate 1 (21,726 cells) after pair timer (0,1)
reached exactly 30 My. The condition that fired is
`update_pair_timers_and_sutures()`: mean continent–continent contact
convergence < `SUTURE_SLOW_CMYR` (1.2 cm/yr) accumulated for
`SUTURE_AFTER_MY` (30 My). Why it was slow: the causal chain is rows
5→4→8→16 of the table. The two plates made continental contact; a contact
of only max(5% of boundary, 4 cells) saturates `f_coll` to 1
(`COLLISION_SATURATION`), which zeroes the target speed
(`COLLISION_DAMP = 1.0`); both plates dropped to the jammed clamp floor
of 0.05 deg/My (`SPEED_FLOOR_JAMMED`) within a few steps
(`SPEED_RELAX_DOWN = 0.5`, τ ≈ 3 My); closure at two floor speeds is at
most ~1.1 cm/yr at the rotation equator — *by construction* below the
1.2 cm/yr threshold, whose comment says it was set above the jam-creep
ceiling for exactly this purpose. So every continental contact, however
small, is a guaranteed weld 30 My later, and the merge takes the loser's
entire cell set (oceans included). Ten steps earlier the same pair-1
machinery had already absorbed plate 9 (986 cells, timer 30.0 at
t = 164 My); by the 200 My sample plate 1 held 87% of the sphere. No
force balance, contact extent, or ocean-closure condition was consulted.

**Breakup trace — seed `cyrus`, t = 264 My, plate 12 → 12 + 14.** The
condition that fired is the second candidate block of `maybe_breakup()`,
the WO-0003 Fix 4 gridlock breaker: alive count 6 ≤ `PLATE_FLOOR` and a
pair timer ≥ 30 My whose suture is *blocked by that same floor*. Plate 12
was chosen as the most-continental eligible plate (413 continental cells
≥ 32; last suture 102 My ago, just past `BREAKUP_SUTURE_AGE_MY` = 100) —
not because anything was pulling it apart: it held 3,241 cells, nowhere
near the 13,654-cell area quota, and its continental cells were 8.5% of
the world's total, nowhere near the 1/3 share trigger. The rift line is a
random great circle through its continental centroid (`random_tangent`)
and the halves were *given* ±0.15 deg/My (`BREAKUP_RIFT_SPEED`) by
decree. This is the "rifting appears without a physical driver" defect in
one event: the trigger is plate-count bookkeeping, the geometry is RNG,
and the kinematics are imposed. 20 of the run's 31 breakups fired this
way.
