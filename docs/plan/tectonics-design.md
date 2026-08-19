# Phase 1 design: tectonics stage and era picker

Design of record for WO-0002. Constants marked (log) get decision-log entries
when finalized. Reference: Cortial et al. 2019, Procedural Tectonic Planets.

## 1. Data model

### FieldStore gains integer fields
`FieldStore` adds a parallel `Vec<(String, Vec<u32>)>` store with
`set_u32 / get_u32 / get_u32_mut / get_or_insert_mut_u32`. Rationale: plate ids
and the feature bitmask need exact equality and bit ops; f32-encoded ids invite
silent corruption. Continuous quantities stay f32. (log)

### Per-cell fields written by the tectonics stage ("present" state)
- f32 `elevation_m` — derived, relative to solved sea level (sea level = 0)
- f32 `crust_thickness_km`
- f32 `crust_age_my`
- f32 `orogeny_age_my`
- f32 `rift_age_my` — internal rift timer (needed for full-state resume)
- f32 `hotspot_buildup_km` — advected shield-volcano construction
- u32 `plate_id`
- u32 `crust_type` — 0 ocean, 1 continent
- u32 `features` — bit 0 RIDGE, 1 TRENCH, 2 ARC, 3 HOTSPOT, 4 RIFT,
  5 BOUNDARY_DIVERGENT, 6 BOUNDARY_CONVERGENT, 7 BOUNDARY_TRANSFORM
  (boundary bits are display/classification results, recomputed each step)

### Params (UI-visible, all hashed into params_hash)
| param | default | range |
|---|---|---|
| plate_count | 12 | 8–24 |
| land_fraction | 0.29 | 0.05–0.7 |
| tectonic_vigor | 1.0 | 0.25–2.0 |
| span_my | 500 | 200–2,000 |
| hotspot_count | 6 | 0–12 |

Edit overlays (also hashed): craton overlay = sorted `Vec<(cell: u32, i8)>`
(+1 paint continent, −1 force ocean); hotspot overlay = `Option<Vec<[f32;3]>>`
(when Some, replaces the generated hotspot set entirely).

### Stage plumbing
- `StageContext` gains `progress: Option<Arc<Progress>>` where
  `Progress { fraction: AtomicU32 /* f32 bits */, cancel: AtomicBool }`.
  Cancel makes the stage return a `Cancelled` error; the pipeline already
  leaves `last_key` unset on error, so a cancelled run never poisons the cache.
- `WorldState` gains `history: Option<TectonicsHistory>`; the stage fills it.
- The app pins "present": `TectonicsStage` gets `present_my: Option<f32>`
  (param-hashed); when set, the stage decodes that keyframe into the world
  fields instead of the final one. Downstream stages and exports read fields
  as usual and never know about time.

## 2. Determinism rules (additions for Phase 1)

- **No platform trig in the sim path.** Rodrigues rotation needs sin/cos of the
  per-step rotation angle only (|θ| ≤ 1.2 deg/My × 2 My = 2.4° = 0.042 rad).
  Implement `det_sin_cos(x)` in worldmaker-core as fixed-order Taylor
  polynomials (x⁷ / x⁶ terms; error « f32 ulp on this range). +, −, ×, /, sqrt
  are IEEE-exact; libm sin/cos/exp/atan2 are not and are banned from the sim.
  Orogeny relaxation uses the precomputed literal `exp(−dt/200)` = f32 constant.
- **Reductions are integer counts or serial loops.** rayon float reductions
  have nondeterministic order; every cross-cell aggregate in the sim (plate
  areas, boundary-type fractions, hypsometric bisection) counts integers or
  runs serially in cell-id order.
- **RNG keyed by absolute step.** Per-step randomness uses
  `sub_rng(seed, "phase1-tectonics", &format!("{purpose}-step{n}"))` and
  per-plate purposes include the plate id. A re-run from keyframe K replays
  steps K+1.. with identical randomness — resumability without storing RNG
  state.
- Advection writes are per-cell pure functions into a double buffer — parallel
  safe and order-independent.

## 3. Setup (t = 0)

1. **Plate seeds**: farthest-point sampling — first seed uniform-random cell,
   each next seed maximizes min angular distance (dot products only, serial,
   ties → lower cell id).
2. **Ownership**: great-circle Voronoi — every cell assigned to the seed with
   max dot product (ties → lower plate id). O(N·P) direct loop.
3. **Cratons**: continental crust target fraction = land_fraction × 1.35
   (shelf allowance, log). ~20% of plates are drawn oceanic-only (no craton);
   remaining target cells distributed ∝ plate area × U(0.5, 1.5). Per plate,
   nucleus center = interior cell farthest from plate boundary; grow by BFS to
   size. Thickness 35–45 km (peak at nucleus, tapering to 35 at edge), age
   U(1,500–3,500) My. Craton overlay applied last: painted cells become
   continent (40 km, 2,000 My), erased cells forced ocean.
4. **Ocean init**: thickness 7 km, age = 30 + 50·(0.5 + 0.5·dot(x, u)) My for
   one random unit vector u (smooth ramp; spreading self-organizes it away
   within the first steps).
5. **Plate motion**: Euler pole = uniform random unit vector; speed =
   |N(0.5, 0.15)| × vigor, clamped 0.1–1.2 deg/My.
6. **Hotspots**: hotspot_count fixed mantle points, uniform random unit
   vectors (min 15° apart, retry draw), unless the overlay replaces them.

## 4. Time step (dt = 2 My)

Per step, double-buffered prev → next:

**F. Motion update** (serial over plates): pole random-walk — rotate pole by
N(0, 0.6°) about a random tangent axis per step (log). Slab pull:
`target = base_speed × (1 + 1.0·f_sub) × (1 − 0.7·f_coll)` where f_sub /
f_coll are the plate's subducting / colliding boundary-cell fractions from the
previous step (integer counts); `speed += 0.15·(target − speed)`; clamp
0.1–1.2 deg/My (all constants log).

**A. Ownership & advection** (parallel over cells): for cell c at x, candidate
plates = {owner(c)} ∪ {owner(n): n ∈ neighbors(c)}, deduped, sorted by id.
For each candidate p: `src = R_p⁻¹·x`, `src_cell = nearest_cell(src, hint=c)`,
p covers c iff `prev.plate_id[src_cell] == p`.
- exactly one cover → copy prev fields from src_cell, owner = p
- none (gap, divergent) → new ocean crust: age 0, 7 km, RIDGE flag,
  owner = prev owner of c (plates grow at trailing edges)
- ≥2 (overlap, convergent) → resolve polarity: continent overrides ocean;
  ocean vs ocean → younger (less dense) overrides; continent vs continent →
  thicker crust wins the cell (tie → lower plate id), no consumption, both
  flagged colliding. Owner = overrider, fields from its src_cell. If a loser
  was oceanic: subduction event (TRENCH flag here; loser's plate logs
  subducting boundary; consumed crust simply isn't copied anywhere).

**B. Boundary classification** (parallel over cells): for each neighbor pair
(c, n) with different owners, relative surface velocity
`v_rel = (ω_a − ω_b) × x` (deg/My converted to cm/yr; 1 deg/My = 11.12 cm/yr
on Earth radius); normal component along the tangent-projected direction
c→n. Divergent > +0.4 cm/yr, convergent < −0.4, transform between. Sets the
three boundary display bits; feeds rift thinning and collision rates.

**C. Events** (mixed, deterministic order):
- Arc placement: BFS rings from this step's trench cells into the overriding
  plate; cells at graph distance chosen to land 150–250 km inboard (ring 3 at
  L7 ≈ 165 km, scaled per level: ring = round(200 km / mean cell spacing)).
  Arc cells: ARC flag; thickness += 0.15 km/My × dt while active (log);
  oceanic overrider cells with thickness ≥ 20 km convert to continent
  (island arc).
- Continental collision: colliding continental cells (and their 1-ring)
  thicken at 0.12 km/My per cm/yr of convergence (log), cap 70 km,
  orogeny_age = 0.
- Rifting: continental cells on a divergent boundary accumulate rift_age
  (RIFT flag); after 20 My, thin at 0.2 km/My; below 25 km → convert to
  ocean crust (age 0, 7 km, ridge).
- Suturing: per unordered plate pair, track continuous slow-collision time
  (mean convergence < 0.5 cm/yr while colliding); after 30 My the smaller
  plate merges into the larger (cells reassigned, poles/speed of survivor
  kept, suture time recorded). Skipped if plate count is at the floor of 6.
- Supercontinent breakup: a plate holding > 1/3 of all cells whose youngest
  suture is > 100 My old (never-sutured counts as ancient) splits: plane
  through its continental-interior centroid with sub_rng orientation; cells
  on the far side → new plate id with a diverged Euler pole; continental
  cells within 1 ring of the plane get RIFT flags and rift_age starts.
- Hotspots: the cell containing each hotspot point (nearest_cell) and its
  neighbors gain hotspot_buildup at 0.5 / 0.25 km/My (center/ring, log),
  cap 4 km; HOTSPOT flag while buildup > 0.5 km. Buildup advects with the
  crust — drifting plates leave age-progressive chains.

**D. Aging** (parallel per cell): crust_age += dt; orogeny_age += dt;
inactive orogens (continent, thickness > 38, not thickened this step) relax:
`thickness = 38 + (thickness − 38) × 0.990049834` (= exp(−2/200), literal).

**E. Elevation derive** (parallel per cell; keyframe steps only — elevation
never feeds back into dynamics):
- continent: `elev = 150 m/km × (thickness − 35 km)`
- ocean: `elev = max(−5600, −(2600 + 365·sqrt(age)))`
- trench: `elev = 0.75·(−8500) + 0.25·elev` (log)
- arc relief bonus: +1,500 m (log) — island arcs (20–25 km crust) come up
  near sea level, some peaks emerge
- hotspot: `elev += buildup_km × 1000` (ocean) / `× 400` (continent, log)
- detail noise: `elev += 300 m × fbm(x)` (reuses Phase 0 value-noise fBm,
  seeded via this stage's sub_rng; low amplitude so coastlines aren't blobby)
- sea level: bisect offset s (40 fixed iterations, integer counts) so
  fraction(elev < s) = 1 − land_fraction; store `elev − s`. Solved per
  keyframe; the UI sea-level slider is an offset around 0.

## 5. Keyframes and history

Every 10 My (every 5th step) plus t=0. Per cell, packed:
elev i16 (m), plate u16, crust_age u16 (My, saturating), thickness u16
(km × 100), orogeny_age u16, rift_age u16, buildup u16 (km × 100), flags u16
(includes crust_type bit 15) = **16 B/cell**. L7 × 2 Gy: 163,842 × 16 × 201
≈ 527 MB ≤ 1 GB budget. Per keyframe also: per-plate states (pole, speed,
base_speed, alive, youngest_suture), pair-collision timers, hotspot points,
solved sea offset, t_my — full state, so a run can restart from any keyframe
(plate drag, Phase 2 branching).

`TectonicsHistory { dt_my, keyframes: Vec<Keyframe>, hotspots, approx_bytes }`
lives in `WorldState.history`; the app moves it out after a run.

## 6. App

- **Async sim**: Generate spawns a worker thread (WorldState + Pipeline built
  there); `Arc<Progress>` shared with UI (progress bar + Cancel button);
  result posted over mpsc, polled nonblocking each frame. Window stays live.
- **Timeline**: slider snapped to keyframes, epoch label "t = N My",
  play/pause (~100 My/s), "Set as present" pins the viewed keyframe (default:
  final). Present marker drawn on the strip.
- **Layer rendering — CPU color bake** (log): the renderer's per-cell buffer
  becomes packed RGBA u32 colors baked on CPU (rayon) from the decoded
  keyframe: Elevation (hypsometric), Plates (24-color categorical palette;
  boundary cells overridden by boundary-type color: ridge/trench/transform),
  Crust age (perceptually uniform sequential, no rainbow), Thickness (debug
  ramp). Scrub = decode + bake + one buffer upload (~1 ms at L7) — instant.
  Sea-level slider re-bakes (still live). WGSL palette code is removed; globe
  keeps its normal-based shading; palettes become testable Rust.
- **Craton brush**: paint mode with radius slider; hit position via the
  existing per-canvas inverse mappings → `nearest_cell` → cells within radius
  (neighbor BFS while dot > cos r). Editing jumps the view to t=0 to show
  nuclei; on stroke end, re-run from t=0 (same seed — layout and motions
  repeat, new continents ride along).
- **Hotspot placement**: click adds a hotspot (or removes the nearest within
  300 km); overlay replaces the generated set; re-run.
- **Plate drag**: attempted only if brush + hotspots are green; drag vector at
  hit point → recompute that plate's Euler pole to satisfy the surface
  velocity; re-run **from the currently viewed keyframe** (full-state
  restart). Otherwise queued to Phase 2.

## 7. Goldens & tests

- NoiseElevationStage leaves the app pipeline but its unit golden stays
  (unchanged code, still passes). New committed goldens: tectonics final
  `elevation_m` hash and `plate_id` hash, L6 seed 42 defaults; regenerating
  them is a deliberate, logged act.
- Unit tests: det_sin_cos accuracy; keyframe encode/decode roundtrip;
  boundary classification on hand-built two-plate cases; sea-level bisection;
  rigid advection of a single-plate sphere shifts fields coherently; suture
  floor of 6; craton overlay determinism.
- Acceptance harness: `--tectonics-results <file>` headless runs: default
  500 My L7 (timed), 1 Gy L7 (timed, ≤ 60 s), 2 Gy L6 (stability: plate count
  6–24, land fraction ±5%), age-depth bins vs cooling curve (±10%),
  hypsometry bimodality (2-means split + Ashman's D > 2, log), arc-trench
  adjacency (≥95% within 400 km on the overriding side), determinism double
  run, keyframe memory bytes.
