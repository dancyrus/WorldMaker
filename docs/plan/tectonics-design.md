# Phase 1 design: tectonics stage and era picker (as built)

Design of record for WO-0002, updated to the implementation that shipped.
The pre-implementation draft was reviewed by a five-lens adversarial agent
panel; every accepted finding is reflected here and in the decision log.
Reference: Cortial et al. 2019, Procedural Tectonic Planets. Constants live
in `crates/worldmaker-sim/src/tectonics/step.rs` and the decision log.

## 1. Data model

- `FieldStore` carries u32 integer fields (plate_id, crust_type, features
  bitmask) beside the f32 fields — exact bit ops, no float-encoded ids.
- Per-cell state: `elevation_m` (relative to solved sea level),
  `crust_thickness_km`, `crust_age_my`, `orogeny_age_my`, `rift_age_my`,
  `hotspot_buildup_km` (f32); `plate_id`, `crust_type`, `features` (u32).
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

1. **Motion**: pole random-walk N(0, 0.6°)/step. Slab pull:
   `target = base·(1 + f_sub)·(1 − f_coll_sat)` where f_coll saturates at
   max(5% of boundary, 4 cells) — a continental collision along even a small
   arc stalls the whole plate (India–Asia; conservation demands it). Braking
   relax 0.5/step, acceleration 0.15/step; clamp [0.1·(1−f_coll), 1.2] deg/My.
   Each step's rotation banks into a per-plate pending matrix; advection
   commits it once the banked angle reaches 0.75 cell — slow plates never
   freeze to the grid.
2. **Advection** (forward-scatter + gather): committing plates scatter
   claims (dst cell + ring) into an atomic per-cell bitmask over ≤ 32 alive
   plates; each cell then coverage-tests its candidates by back-rotation and
   `nearest_cell`. 0 covers → ridge crust (unless the cell was
   transform-classified last step — hex-zigzag gating); 1 cover → copy;
   overlap → polarity duel: "hard" crust (continent ≥ 30 km) cannot be
   consumed, two hard plates jam in place (cell frozen, collision recorded);
   otherwise the hard plate — or the youngest soft crust — overrides and the
   loser is consumed (TRENCH + slab-pull stats, suppressed at transform
   cells). Thin continent (< 30 km: island arcs, rifted slivers) is
   subductible — the continental inventory closes. Fully consumed plates die.
3. **Classification**: per foreign-neighbor edge, separation =
   dot(v_n − v_c, ê_c→n) at the edge midpoint in cm/yr; divergent > +0.4,
   convergent < −0.4, else transform. Feeds display bits, rift driver,
   collision stats (continent-continent contact, not overlap events — slow
   contacts must keep reading as collisions or nothing ever sutures).
4. **Events**: arcs — BFS ring ceil(150/spacing)..floor(250/spacing) from
   this step's trenches on the overriding plate; growth 0.6 (ocean) / 0.15
   (continent) km/My, cap 70; oceanic cells convert to continent at 20 km.
   Collision thickening 0.12 km/My per cm/yr, cap 70, orogeny_age = 0.
   Rifts: +dt on continent-continent divergence, −2·dt otherwise
   (hysteresis); thin 0.2 km/My past 20 My; oceanize below 25 km. Sutures:
   per-pair slow-contact timers (< 0.5 cm/yr accrues, fast resets); at 30 My
   the smaller plate merges (floor 6). Breakup: a plate over 1/3 of the
   sphere *or* 1/3 of the world's continental crust (logged deviation —
   breaks floor-6 gridlock) with suture age > 100 My splits along a random
   great circle through its continental centroid; the halves get ±0.15
   deg/My across the plane and the rift line starts mature. Hotspots: 0.8 /
   0.4 km/My (center/ring), cap 8 km, decaying on the 200 My constant —
   chains subside as they drift.
5. **Aging**: ages += dt; inactive non-primordial orogens relax toward
   38 km (×0.990049834 per step).
6. **Elevation** (keyframe steps only; never integrated): continent
   150 m/km above 35 km; ocean −(2600 + 365·√age) floored at −5,600;
   trench blend 75% toward −8,500; arc +2,000 m; buildup ×1,000 (ocean) /
   ×400 (continent) m/km; ±300 m fBm detail. Sea level bisected (40
   iterations, integer counts) to the ocean-fraction target; elevations
   stored relative to it; the offset is recorded per keyframe.

## 5. Keyframes

Every 10 My (20 My at L8 — 1 GB budget), 16 B/cell: elev i16, plate u16,
age u16, thickness u16 (×100), orogeny u16, rift u16, buildup u16 (×100),
flags u16 (features + boundary class + crust_type in bit 15); plus exact
per-plate state, pair timers, sea offset. Measured: 527 MB for 2 Gy at L7.
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
  Layers: Elevation (hypsometric), Plates (24 categorical + boundary bands
  by type), Crust age (viridis on ocean age, gray continents), Thickness
  (batlow); Climate greyed out.
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
