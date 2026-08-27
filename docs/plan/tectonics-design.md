# Phase 1 design: tectonics stage and era picker (as built)

Design of record for WO-0002, updated to the implementation that shipped
and to the WO-0006 plate-physics replacement (force balance, slab ledger,
strength-field suture and rifting). The WO-0002 draft was reviewed by a
five-lens adversarial agent panel; the WO-0005 audit
(`docs/plan/plate-physics-audit.md`) then catalogued every ad-hoc motion
mechanic, and `docs/plan/plate-physics-model.md` (accepted by Dan with
amendments A-C, decision log 2026-08-27) is the design of record for
everything in section 4 below. References: Cortial et al. 2019 for the
grid/advection skeleton; the model document carries the geodynamics
citations (Forsyth & Uyeda 1975, Turcotte & Schubert, Wilson 1966, Gurnis
1988, and others) per mechanism. Constants live in
`crates/worldmaker-sim/src/tectonics/step.rs`; the calibrated values and
every calibration trial are in
`docs/results/plate-physics-calibration.json`.

## 1. Data model

- `FieldStore` carries u32 integer fields (plate_id, crust_type, features
  bitmask) beside the f32 fields — exact bit ops, no float-encoded ids.
- Per-cell state: `elevation_m` (relative to solved sea level),
  `crust_thickness_km`, `crust_age_my`, `orogeny_age_my`, `rift_age_my`,
  `hotspot_buildup_km` (f32); `plate_id`, `crust_type`, `features` (u32);
  since WO-0006: `slab_plate` (u16, whose slab lies beneath this cell),
  `slab_since_my` (when it went under), `suture_at_my` (the suture scar —
  data, not just an event count).
  Feature bits 0–4: RIDGE, TRENCH, ARC, HOTSPOT, RIFT — all *current status*,
  rebuilt every step (no fossil flags); bits 5–7: boundary class
  (divergent/convergent/transform) for display and event gating.
- Params (all hashed): plate_count 8–24 (12), land_fraction 0.05–0.7 (0.29),
  tectonic_vigor 0.25–2 (1), span_my 200–2,000 (500), hotspot_count 0–12 (6),
  craton overlay (sorted `(cell, ±1)`), hotspot overlay (replaces the set).
- `StageContext.progress`: shared fraction + cancel atomics. A cancelled run
  returns `Cancelled`; `Pipeline::run` marks a stage dirty *before* running
  it, so a failed run can never serve a stale cache entry (regression test).
- `WorldState.history: Option<TectonicsHistory>` — keyframes, hotspots,
  run diagnostics (continental-inventory flows, suture/breakup counts).

## 2. Determinism (the golden-hash contract)

- No libm in the sim path. `worldmaker_core::dmath` provides fixed-order
  Taylor `det_sin_cos` (|x| ≤ 0.75), Irwin–Hall gaussians, cube-rejection
  unit vectors, raw-bit uniforms; `exp(−dt/200)` is a literal constant.
  +, −, ×, /, sqrt, round, floor, clamp are IEEE-exact and allowed.
- All cross-cell reductions are integer counts or serial id-ordered loops;
  the only atomics are commutative ORs (candidate masks) and integer adds.
- RNG purposes embed the absolute step index (and plate id), so a resumed
  run replays identical randomness.
- **Keyframes are exact state**: at every keyframe the sim round-trips its
  own f32 arrays through the u16 quantization (round-then-clamp on both
  sides — rounding makes the encode idempotent), and `PlateState` carries
  the pending sub-cell rotation plus the previous step's boundary stats.
  `resume_from_keyframe_is_bit_exact` proves it.

## 3. Setup (t = 0)

Farthest-point plate seeds → great-circle Voronoi ownership → per-plate
Euler pole (uniform) and speed |N(0.5, 0.15)|·vigor → cratons: continental
target = land_fraction × 1.35 of the sphere, ~20% of plates drawn oceanic,
per-plate BFS growth from the most interior cell, thickness 35–45 km
tapering outward, age U(1,500–3,500) My, orogeny_age = age (primordial,
exempt from relaxation) → ocean: 7 km, age = 30 + 50·ramp along a random
axis → craton overlay applied last → hotspots ≥ 15° apart.

## 4. Time step (dt = 2 My)

Replaced wholesale by WO-0006 (S1-S3); the model document is normative.
Per step: motion -> advection -> connectivity backstop -> classification ->
stats -> arcs -> collisions/rift maturation -> sutures -> rift splits ->
rift growth/nucleation -> hotspots -> aging. The step is RNG-free — poles
wander exactly when a plate's boundary makeup changes.

1. **Force balance** (model §1): plates are inertialess Stokes flow.
   `v_target = (F_slab + F_ridge + F_resid) / (R_drag + R_bnd)`, relaxed
   over `TAU_MY`; the pole relaxes toward the summed boundary torque
   direction on the same timescale. Slab pull comes from the plate's
   attached slab-ledger segments weighted by thermal age at subduction;
   ridge push from divergent boundary cells (young "nascent ridge" ocean
   counts, so a fresh split corridor pushes its halves apart); residual
   mantle traction `K_MANTLE·A` stands in for unsolved mantle flow — a
   slab-free plate coasts to `K_MANTLE / C_DRAG` (~0.8 cm/yr) and that is
   correct behavior; there is no floor. Resistance: basal drag `C_DRAG·A`,
   strength-weighted continent-continent contact `C_CONTACT·Σ strength`,
   and transform friction. `SPEED_MAX` survives only as a safety rail.
2. **Slab ledger** (model §2, amendment C): every consumption step appends
   one merged segment `{area, age_at_subduction, when, attached}` to the
   subducting plate; segments detach individually after `SLAB_DETACH_MY`
   (pull fades only after subduction stops), and a dead plate's ledger
   transfers to its consumer (slabs keep sinking — Dan's ruling). The
   per-cell `slab_plate`/`slab_since_my` fields ride with the overriding
   plate and feed the app's Overlay layer.
3. **Advection** (forward-scatter + gather, unchanged skeleton): overlap
   duels as before, except the continent-continent overlap resolves to the
   SLOWER plate — the §7 cause-removal: rigid plates cannot shed frozen
   cells, so the plates stop instead. A serial connected-components
   backstop runs after every advection (to a fixpoint): each plate keeps
   its largest component and fragments reassign to the longest-border
   neighbor; a sizable young-oceanic fragment against an active trench
   becomes a trench-trapped microplate instead (model §6, Juan de Fuca).
4. **Lithosphere strength** (model §4 + amendment B):
   `S = S_type · g_age · g_suture · thickness penalties · g_insulation`,
   ordering craton > old ocean > young continent > fresh suture or rift.
   Primordial continent keeps ocean-grade S_type so cratons anchor ~2.0.
   Mantle insulation under a plate holding > 1/3 of continental crust
   weakens its continental cells toward `INSULATION_FLOOR` — amendment B's
   supercontinent-breakup driver.
5. **Suture** (model §3): a pair welds only when all three hold for 30 My
   sustained — contact ≥ 30% of the smaller plate's perimeter, mean
   relative speed across the contact < 0.4 cm/yr, and no ocean within two
   rings of the contact on either side (the terminal act of the Wilson
   cycle). The weld writes `suture_at_my` on every contact cell: the scar
   weakens the strength field for `SUTURE_HEAL_MY` and is the preferred
   path for later rifting.
6. **Rifting** (model §5 + amendment A): a rift nucleates only at a real
   driver — plume under continent ≥ 20 My, back-arc extension behind an
   old-slab trench, or opposing slab pull ≥ 120° apart — and only where
   driver stress exceeds local strength; it advances a finite
   ~75 km/My along the weakest neighbor and stalls (= fails) the moment
   stress no longer beats the strength ahead. A corridor that oceanizes
   across the plate splits it; the halves' motion comes from the force
   balance alone. No random breakup, no plate-count trigger, no area
   quota; a 200 My per-plate refractory models stress re-accumulation.
7. **Arcs, collisions, hotspots, aging**: unchanged from WO-0002 (arc BFS
   bands, collision thickening 0.12 km/My per cm/yr, hotspot shields,
   orogenic relaxation exempting primordial crust).

Acceptance is the model's §9: nine gates measured by
`tectonics::metrics::PhysicsTracker` at seeds cyrus and 42 (2 Gy, L6) and
armed in `tests/plate_physics_gates.rs` — see that file's header for which
clauses are armed and which are recorded-with-reasons (three §9 targets
need mechanics the model deliberately lacks: enclosed-basin closure at
terminal collisions, stress-directed rift pathing, connectivity-aware
advection).

## 5. Keyframes

Every 10 My (20 My at L8, 100 My at L9 — 1 GB budget), 22 B/cell since
WO-0006: elev i16, plate u16, age u16, thickness u16 (×100), orogeny u16,
rift u16, buildup u16 (×100), flags u16 (features + boundary class +
crust_type in bit 15), slab_plate u16, slab_since u16, suture_at u16;
plus exact per-plate state (including the slab ledger), pair timers, live
rifts, sea offset. Measured (S3): 22.01 B/cell at L7, 0.725 GB for a 2 Gy
history — inside the 1 GB budget.
`run_history(resume: Option<ResumeFrom>)` restarts bit-exactly from any
keyframe — the foundation for plate drag (Phase 2) and branching (Phase 6).

## 6. App

- Sim on a worker thread (`SimJob`: Progress + mpsc); progress bar and a
  working Cancel; starting a run drops the old history first (never two in
  memory). The stage publishes the final keyframe; the app pins any other
  "present" by decoding that keyframe into the world fields — never a re-sim.
- Timeline: integer keyframe slider (inherent snap), epoch readout,
  play/pause at 100 My/s, Set-as-present + present marker.
- Rendering: per-cell RGBA8 colors baked on the CPU per (layer, keyframe,
  sea offset) into one storage buffer; globe interpolates vertex colors with
  its Lambert shade, flat looks colors up through the cell-id raster.
  Layers: Elevation (hypsometric), Plates (48 categorical + smoothed
  boundary polylines by type), Crust age (viridis on ocean age, gray
  continents), Thickness (batlow), the two velocity layers (WO-0004), and
  Overlay (WO-0006: Plates at 40% brightness with each cell above a
  subducted slab drawn in the slab plate's color, fading toward
  detachment); Climate greyed out.
- Craton brush/eraser (radius 150–2,000 km, neighbor flood on the grid,
  both canvases through `nearest_cell`): paints a per-level overlay shown at
  t = 0; stroke end re-runs from t = 0 with the same seed. Hotspot tool:
  click adds, click-near removes (300 km); the overlay replaces the
  generated set. Plate drag: queued to Phase 2 (hashed drag-overlay design).

## 7. Verification

- Unit/integration: dmath accuracy + reproducibility; keyframe roundtrip +
  saturation; pipeline cache (including the cancel-poisoning regression);
  sanity of a short run (land fraction, plate band, ridges/trenches, ocean
  age self-organization); same-seed hash equality; bit-exact resume; craton
  overlay determinism + plate-layout invariance; cache reaction to overlays;
  golden hashes for the default L6 world (elevation + plate ids) that Linux
  CI must reproduce bit-for-bit.
- Acceptance harness (`--tectonics-results`, results JSON committed):
  age-depth bins vs cooling curve, 2-means + Ashman's D bimodality,
  arc-trench adjacency (same-plate trench within 400 km), determinism
  double-run, 2 Gy L6 stability (plate band, land fraction, continental
  inventory, sutures/breakups), timings (500 My / 1 Gy / 2 Gy) and keyframe
  memory vs budgets.
- Plate-physics gates (WO-0006 S3): the model's §9 metrics as CI gates in
  `tests/plate_physics_gates.rs`, canonical implementation in
  `tectonics::metrics::PhysicsTracker`; calibration record with all
  trials in `docs/results/plate-physics-calibration.json`.
